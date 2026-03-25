use crate::server::task_executor::{TaskExecutor, TaskInput, TaskResult};

/// Copies files from source paths to a target directory.
pub struct CopyTaskExecutor;

impl Default for CopyTaskExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl CopyTaskExecutor {
    pub fn new() -> Self {
        Self
    }
}

#[tonic::async_trait]
impl TaskExecutor for CopyTaskExecutor {
    fn task_type(&self) -> &str {
        "Copy"
    }

    async fn execute(&self, input: &TaskInput) -> TaskResult {
        let start = std::time::Instant::now();
        let mut result = TaskResult::default();

        if !input.target_dir.exists() {
            if let Err(e) = tokio::fs::create_dir_all(&input.target_dir).await {
                result.success = false;
                result.error_message = format!("Failed to create target directory: {}", e);
                return result;
            }
        }

        for source in &input.source_files {
            if !source.exists() {
                result.success = false;
                result.error_message = format!("Source file not found: {}", source.display());
                return result;
            }

            let dest = input
                .target_dir
                .join(source.file_name().unwrap_or_default());

            match tokio::fs::copy(source, &dest).await {
                Ok(bytes) => {
                    result.files_processed += 1;
                    result.bytes_processed += bytes;
                    result.output_files.push(dest);
                }
                Err(e) => {
                    result.success = false;
                    result.error_message = format!("Failed to copy {}: {}", source.display(), e);
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
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_copy_single_file() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        let dest_dir = tmp.path().join("dest");
        tokio::fs::create_dir_all(&src_dir).await.unwrap();

        let src_file = src_dir.join("test.txt");
        tokio::fs::write(&src_file, b"hello world").await.unwrap();

        let executor = CopyTaskExecutor::new();
        let mut input = TaskInput::new("Copy");
        input.source_files.push(src_file.clone());
        input.target_dir = dest_dir.clone();

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert_eq!(result.files_processed, 1);
        assert_eq!(result.output_files.len(), 1);

        let content = tokio::fs::read(&dest_dir.join("test.txt")).await.unwrap();
        assert_eq!(content, b"hello world");
    }

    #[tokio::test]
    async fn test_copy_multiple_files() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        let dest_dir = tmp.path().join("dest");
        tokio::fs::create_dir_all(&src_dir).await.unwrap();

        for name in &["a.txt", "b.txt", "c.txt"] {
            let path = src_dir.join(name);
            tokio::fs::write(&path, name.as_bytes()).await.unwrap();
        }

        let executor = CopyTaskExecutor::new();
        let mut input = TaskInput::new("Copy");
        input.source_files.push(src_dir.join("a.txt"));
        input.source_files.push(src_dir.join("b.txt"));
        input.source_files.push(src_dir.join("c.txt"));
        input.target_dir = dest_dir;

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert_eq!(result.files_processed, 3);
        assert_eq!(result.output_files.len(), 3);
    }

    #[tokio::test]
    async fn test_copy_missing_source() {
        let tmp = tempfile::tempdir().unwrap();
        let dest_dir = tmp.path().join("dest");

        let executor = CopyTaskExecutor::new();
        let mut input = TaskInput::new("Copy");
        input
            .source_files
            .push(PathBuf::from("/nonexistent/file.txt"));
        input.target_dir = dest_dir;

        let result = executor.execute(&input).await;
        assert!(!result.success);
        assert!(result.error_message.contains("not found"));
    }

    #[tokio::test]
    async fn test_copy_creates_target_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let src_file = tmp.path().join("test.txt");
        tokio::fs::write(&src_file, b"data").await.unwrap();

        let dest_dir = tmp.path().join("nested/deep/dest");

        let executor = CopyTaskExecutor::new();
        let mut input = TaskInput::new("Copy");
        input.source_files.push(src_file);
        input.target_dir = dest_dir.clone();

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert!(dest_dir.exists());
    }
}
