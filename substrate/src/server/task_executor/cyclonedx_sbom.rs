use std::path::PathBuf;

use base64::Engine as _;

use crate::server::cyclonedx_sbom::{
    aggregate_contracts, render_json, render_xml, CycloneDxAggregateOptions, CycloneDxSbomContract,
};
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

        let contract = match contract_from_input(input) {
            Ok(contract) => contract,
            Err(error) => {
                result.success = false;
                result.error_message = error;
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

fn contract_from_input(input: &TaskInput) -> Result<CycloneDxSbomContract, String> {
    if let Some(encoded_contract) = input.options.get("sbom_contract_json_b64") {
        let contract_bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded_contract)
            .map_err(|error| format!("Invalid sbom_contract_json_b64: {}", error))?;
        return serde_json::from_slice::<CycloneDxSbomContract>(&contract_bytes)
            .map_err(|error| format!("Invalid CycloneDX SBOM contract JSON: {}", error));
    }
    if let Some(encoded_contracts) = input.options.get("aggregate_input_contracts_json_b64") {
        let contracts_bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded_contracts)
            .map_err(|error| format!("Invalid aggregate_input_contracts_json_b64: {}", error))?;
        let contracts = serde_json::from_slice::<Vec<CycloneDxSbomContract>>(&contracts_bytes)
            .map_err(|error| {
                format!(
                    "Invalid CycloneDX aggregate input contract JSON array: {}",
                    error
                )
            })?;
        return aggregate_contracts(
            &contracts,
            CycloneDxAggregateOptions {
                spec_version: required_option(input, "aggregate_spec_version")?,
                serial_number: required_option(input, "aggregate_serial_number")?,
                timestamp: required_option(input, "aggregate_timestamp")?,
                root_group: input
                    .options
                    .get("aggregate_root_group")
                    .cloned()
                    .unwrap_or_default(),
                root_name: required_option(input, "aggregate_root_name")?,
                root_version: required_option(input, "aggregate_root_version")?,
                root_component_type: required_option(input, "aggregate_root_component_type")?,
                root_project_path: input
                    .options
                    .get("aggregate_root_project_path")
                    .cloned()
                    .unwrap_or_default(),
                root_vcs_url: String::new(),
                external_references: whitespace_values(
                    input
                        .options
                        .get("aggregate_external_references")
                        .map(String::as_str)
                        .unwrap_or_default(),
                ),
            },
        );
    }
    Err(
        "CycloneDxSbom task is missing sbom_contract_json_b64 or aggregate_input_contracts_json_b64"
            .to_string(),
    )
}

fn required_option(input: &TaskInput, key: &str) -> Result<String, String> {
    input
        .options
        .get(key)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("CycloneDxSbom aggregate task is missing {key}"))
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

fn whitespace_values(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .filter(|entry| !entry.trim().is_empty())
        .map(|entry| entry.to_string())
        .collect()
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

    #[tokio::test]
    async fn writes_aggregate_outputs_from_input_contracts() {
        let dir = tempfile::tempdir().unwrap();
        let json_output = dir.path().join("aggregate.json");
        let contract = serde_json::json!({
            "schema": CONTRACT_SCHEMA,
            "spec_version": "1.6",
            "serial_number": "urn:uuid:00000000-0000-0000-0000-000000000001",
            "timestamp": "2026-05-12T10:00:00Z",
            "root_component": {
                "type": "library",
                "bom-ref": "pkg:maven/org.example/lib@1.0",
                "group": "org.example",
                "name": "lib",
                "version": "1.0",
                "purl": "pkg:maven/org.example/lib@1.0"
            },
            "components": [],
            "dependencies": []
        });

        let mut input = TaskInput::new("CycloneDxSbom");
        input.options.insert(
            "aggregate_input_contracts_json_b64".to_string(),
            base64::engine::general_purpose::STANDARD
                .encode(serde_json::to_string(&vec![contract]).unwrap()),
        );
        input
            .options
            .insert("aggregate_spec_version".to_string(), "1.6".to_string());
        input.options.insert(
            "aggregate_serial_number".to_string(),
            "urn:uuid:00000000-0000-0000-0000-000000000007".to_string(),
        );
        input.options.insert(
            "aggregate_timestamp".to_string(),
            "2026-05-12T14:00:00Z".to_string(),
        );
        input.options.insert(
            "aggregate_root_group".to_string(),
            "org.example".to_string(),
        );
        input
            .options
            .insert("aggregate_root_name".to_string(), "aggregate".to_string());
        input
            .options
            .insert("aggregate_root_version".to_string(), "1.0".to_string());
        input.options.insert(
            "aggregate_root_component_type".to_string(),
            "application".to_string(),
        );
        input
            .options
            .insert("aggregate_root_project_path".to_string(), ":".to_string());
        input.options.insert(
            "aggregate_external_references".to_string(),
            "https://example.invalid/aggregate".to_string(),
        );
        input.options.insert(
            "output_files_json".to_string(),
            serde_json::to_string(&vec![json_output.to_string_lossy().into_owned()]).unwrap(),
        );

        let result = CycloneDxSbomTaskExecutor::new().execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        let json = std::fs::read_to_string(json_output).unwrap();
        assert!(json.contains("pkg:maven/org.example/aggregate@1.0"));
        assert!(json.contains("pkg:maven/org.example/lib@1.0"));
        assert!(json.contains("\"externalReferences\""));
        assert!(json.contains("https://example.invalid/aggregate"));
    }
}
