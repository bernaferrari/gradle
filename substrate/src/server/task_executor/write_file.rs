use crate::server::task_executor::{TaskExecutor, TaskInput, TaskResult};

pub struct WriteFileTaskExecutor;

impl WriteFileTaskExecutor {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WriteFileTaskExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[tonic::async_trait]
impl TaskExecutor for WriteFileTaskExecutor {
    fn task_type(&self) -> &str {
        "WriteFile"
    }

    async fn execute(&self, input: &TaskInput) -> TaskResult {
        let start = std::time::Instant::now();
        let mut result = TaskResult::default();

        let Some(encoded) = input.options.get("static_output_text_b64") else {
            result.success = false;
            result.error_message = "Missing static_output_text_b64".to_string();
            return result;
        };
        if input.target_dir.as_os_str().is_empty() {
            result.success = false;
            result.error_message = "Missing output file".to_string();
            return result;
        }

        let bytes =
            match base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded) {
                Ok(bytes) => bytes,
                Err(error) => {
                    result.success = false;
                    result.error_message = format!("Invalid static_output_text_b64: {}", error);
                    return result;
                }
            };

        if let Some(parent) = input.target_dir.parent() {
            if let Err(error) = std::fs::create_dir_all(parent) {
                result.success = false;
                result.error_message = format!("Failed to create output directory: {}", error);
                return result;
            }
        }
        if let Err(error) = std::fs::write(&input.target_dir, &bytes) {
            result.success = false;
            result.error_message = format!("Failed to write output file: {}", error);
            return result;
        }

        result.output_files.push(input.target_dir.clone());
        result.files_processed = 1;
        result.bytes_processed = bytes.len() as u64;
        result.duration_ms = start.elapsed().as_millis() as u64;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[tokio::test]
    async fn writes_static_text_to_declared_output_file() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("reports/api-contract.txt");
        let mut input = TaskInput::new("WriteFile");
        input.target_dir = output.clone();
        input.options.insert(
            "static_output_text_b64".to_string(),
            base64::engine::general_purpose::STANDARD.encode("oss-style api contract\n"),
        );

        let result = WriteFileTaskExecutor::new().execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        assert_eq!(
            std::fs::read_to_string(output).unwrap(),
            "oss-style api contract\n"
        );
        assert_eq!(result.files_processed, 1);
    }
}
