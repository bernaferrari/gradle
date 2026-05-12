use std::path::PathBuf;

use base64::Engine as _;

use crate::server::cyclonedx_sbom::{render_json, render_xml, CycloneDxSbomContract};
use crate::server::task_executor::{TaskExecutor, TaskInput, TaskResult};

pub struct CycloneDxSbomTaskExecutor;

impl CycloneDxSbomTaskExecutor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CycloneDxSbomTaskExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[tonic::async_trait]
impl TaskExecutor for CycloneDxSbomTaskExecutor {
    fn task_type(&self) -> &str {
        "CycloneDxSbom"
    }

    async fn execute(&self, input: &TaskInput) -> TaskResult {
        let start = std::time::Instant::now();
        let mut result = TaskResult::default();

        let Some(encoded_contract) = input.options.get("sbom_contract_json_b64") else {
            result.success = false;
            result.error_message =
                "CycloneDxSbom task is missing sbom_contract_json_b64".to_string();
            return result;
        };
        let contract_bytes =
            match base64::engine::general_purpose::STANDARD.decode(encoded_contract) {
                Ok(bytes) => bytes,
                Err(error) => {
                    result.success = false;
                    result.error_message = format!("Invalid sbom_contract_json_b64: {}", error);
                    return result;
                }
            };
        let contract = match serde_json::from_slice::<CycloneDxSbomContract>(&contract_bytes) {
            Ok(contract) => contract,
            Err(error) => {
                result.success = false;
                result.error_message = format!("Invalid CycloneDX SBOM contract JSON: {}", error);
                return result;
            }
        };
        let output_files = output_files(input);
        if output_files.is_empty() {
            result.success = false;
            result.error_message = "CycloneDxSbom task is missing output files".to_string();
            return result;
        }

        let json = match render_json(&contract) {
            Ok(json) => json,
            Err(error) => {
                result.success = false;
                result.error_message = error;
                return result;
            }
        };
        let xml = match render_xml(&contract) {
            Ok(xml) => xml,
            Err(error) => {
                result.success = false;
                result.error_message = error;
                return result;
            }
        };

        let mut bytes_processed = 0_u64;
        for output in output_files {
            if let Some(parent) = output.parent() {
                if let Err(error) = std::fs::create_dir_all(parent) {
                    result.success = false;
                    result.error_message = format!(
                        "Failed to create CycloneDX output directory '{}': {}",
                        parent.display(),
                        error
                    );
                    return result;
                }
            }
            let content = if output.extension().and_then(|ext| ext.to_str()) == Some("xml") {
                xml.as_bytes()
            } else {
                json.as_bytes()
            };
            if let Err(error) = std::fs::write(&output, content) {
                result.success = false;
                result.error_message = format!(
                    "Failed to write CycloneDX output '{}': {}",
                    output.display(),
                    error
                );
                return result;
            }
            bytes_processed += content.len() as u64;
            result.output_files.push(output);
        }

        result.files_processed = result.output_files.len() as u64;
        result.bytes_processed = bytes_processed;
        result.duration_ms = start.elapsed().as_millis() as u64;
        result
    }
}

fn output_files(input: &TaskInput) -> Vec<PathBuf> {
    if let Some(value) = input.options.get("output_files_json") {
        if let Ok(paths) = serde_json::from_str::<Vec<String>>(value) {
            let paths = paths
                .into_iter()
                .filter(|path| !path.trim().is_empty())
                .map(PathBuf::from)
                .collect::<Vec<_>>();
            if !paths.is_empty() {
                return paths;
            }
        }
    }
    if input.target_dir.as_os_str().is_empty() {
        Vec::new()
    } else {
        vec![input.target_dir.clone()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::cyclonedx_sbom::CONTRACT_SCHEMA;

    #[tokio::test]
    async fn writes_json_and_xml_outputs_from_contract() {
        let dir = tempfile::tempdir().unwrap();
        let json_output = dir.path().join("bom.json");
        let xml_output = dir.path().join("bom.xml");
        let contract = serde_json::json!({
            "schema": CONTRACT_SCHEMA,
            "spec_version": "1.6",
            "serial_number": "urn:uuid:00000000-0000-0000-0000-000000000001",
            "timestamp": "2026-05-12T10:00:00Z",
            "root_component": {
                "type": "application",
                "bom-ref": "pkg:maven/org.example/app@1.0.0?project_path=%3A",
                "group": "org.example",
                "name": "app",
                "version": "1.0.0",
                "purl": "pkg:maven/org.example/app@1.0.0"
            },
            "components": [],
            "dependencies": []
        })
        .to_string();

        let mut input = TaskInput::new("CycloneDxSbom");
        input.options.insert(
            "sbom_contract_json_b64".to_string(),
            base64::engine::general_purpose::STANDARD.encode(contract),
        );
        input.options.insert(
            "output_files_json".to_string(),
            serde_json::to_string(&vec![
                json_output.to_string_lossy().into_owned(),
                xml_output.to_string_lossy().into_owned(),
            ])
            .unwrap(),
        );

        let result = CycloneDxSbomTaskExecutor::new().execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        assert!(std::fs::read_to_string(json_output)
            .unwrap()
            .contains("\"bomFormat\": \"CycloneDX\""));
        assert!(std::fs::read_to_string(xml_output)
            .unwrap()
            .contains("http://cyclonedx.org/schema/bom/1.6"));
        assert_eq!(result.files_processed, 2);
    }
}
