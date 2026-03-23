use crate::server::task_executor::{TaskExecutor, TaskInput, TaskResult};

/// Creates directories.
pub struct MkdirTaskExecutor;

impl Default for MkdirTaskExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl MkdirTaskExecutor {
    pub fn new() -> Self {
        Self
    }
}

#[tonic::async_trait]
impl TaskExecutor for MkdirTaskExecutor {
    fn task_type(&self) -> &str {
        "Mkdir"
    }

    async fn execute(&self, input: &TaskInput) -> TaskResult {
        let start = std::time::Instant::now();
        let mut result = TaskResult::default();

        // Option: "parents" (default: true, create parent dirs)
        let create_parents = input
            .options
            .get("parents")
            .map(|v| v != "false")
            .unwrap_or(true);

        for dir in &input.source_files {
            if dir.exists() && dir.is_dir() {
                // Already exists — OK
                continue;
            }

            let res = if create_parents {
                tokio::fs::create_dir_all(dir).await
            } else {
                tokio::fs::create_dir(dir).await
            };

            match res {
                Ok(()) => {
                    result.files_processed += 1;
                    result.output_files.push(dir.clone());
                }
                Err(e) => {
                    result.success = false;
                    result.error_message =
                        format!("Failed to create directory {}: {}", dir.display(), e);
                    return result;
                }
            }
        }

        result.duration_ms = start.elapsed().as_millis() as u64;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mkdir_single() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("new_dir");

        let executor = MkdirTaskExecutor::new();
        let mut input = TaskInput::new("Mkdir");
        input.source_files.push(dir.clone());

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert!(dir.is_dir());
    }

    #[tokio::test]
    async fn test_mkdir_nested() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("a/b/c/d");

        let executor = MkdirTaskExecutor::new();
        let mut input = TaskInput::new("Mkdir");
        input.source_files.push(dir.clone());

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert!(dir.is_dir());
    }

    #[tokio::test]
    async fn test_mkdir_existing_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("existing");
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let executor = MkdirTaskExecutor::new();
        let mut input = TaskInput::new("Mkdir");
        input.source_files.push(dir);

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert_eq!(result.files_processed, 0); // Already existed
    }

    #[tokio::test]
    async fn test_mkdir_no_parents() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("a/b/c");

        let executor = MkdirTaskExecutor::new();
        let mut input = TaskInput::new("Mkdir");
        input.source_files.push(dir);
        input
            .options
            .insert("parents".to_string(), "false".to_string());

        let result = executor.execute(&input).await;
        assert!(!result.success);
        assert!(result.error_message.contains("Failed to create directory"));
    }

    #[tokio::test]
    async fn test_mkdir_multiple() {
        let tmp = tempfile::tempdir().unwrap();
        let dirs: Vec<std::path::PathBuf> = (0..3)
            .map(|i| tmp.path().join(format!("dir_{}", i)))
            .collect();

        let executor = MkdirTaskExecutor::new();
        let mut input = TaskInput::new("Mkdir");
        input.source_files = dirs.clone();

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert_eq!(result.files_processed, 3);
        for d in &dirs {
            assert!(d.is_dir());
        }
    }
}
