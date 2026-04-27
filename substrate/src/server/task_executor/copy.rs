use std::path::{Path, PathBuf};
use std::pin::Pin;

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

    fn list_files(
        dir: &Path,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<PathBuf>, String>> + Send + '_>> {
        Box::pin(async move {
            let mut files = Vec::new();
            let mut entries = tokio::fs::read_dir(dir)
                .await
                .map_err(|e| format!("Failed to read directory {}: {}", dir.display(), e))?;
            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|e| format!("Failed to read directory {}: {}", dir.display(), e))?
            {
                let path = entry.path();
                if path.is_dir() {
                    files.extend(Self::list_files(&path).await?);
                } else if path.is_file() {
                    files.push(path);
                }
            }
            files.sort_unstable();
            Ok(files)
        })
    }

    async fn copy_file(src: &Path, dest: &Path, result: &mut TaskResult) -> Result<(), String> {
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create directory {}: {}", parent.display(), e))?;
        }
        let bytes = tokio::fs::copy(src, dest)
            .await
            .map_err(|e| format!("Failed to copy {}: {}", src.display(), e))?;
        result.files_processed += 1;
        result.bytes_processed += bytes;
        result.output_files.push(dest.to_path_buf());
        Ok(())
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

            if source.is_dir() {
                let files = match Self::list_files(source).await {
                    Ok(files) => files,
                    Err(e) => {
                        result.success = false;
                        result.error_message = e;
                        return result;
                    }
                };
                for file in files {
                    let relative = file.strip_prefix(source).unwrap_or(&file);
                    let dest = input.target_dir.join(relative);
                    if let Err(e) = Self::copy_file(&file, &dest, &mut result).await {
                        result.success = false;
                        result.error_message = e;
                        return result;
                    }
                }
            } else {
                let dest = input
                    .target_dir
                    .join(source.file_name().unwrap_or_default());
                if let Err(e) = Self::copy_file(source, &dest, &mut result).await {
                    result.success = false;
                    result.error_message = e;
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

    #[tokio::test]
    async fn test_copy_directory_recursively() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        let dest_dir = tmp.path().join("dest");
        tokio::fs::create_dir_all(src_dir.join("nested"))
            .await
            .unwrap();
        tokio::fs::write(src_dir.join("root.txt"), b"root")
            .await
            .unwrap();
        tokio::fs::write(src_dir.join("nested/child.txt"), b"child")
            .await
            .unwrap();

        let executor = CopyTaskExecutor::new();
        let mut input = TaskInput::new("Copy");
        input.source_files.push(src_dir);
        input.target_dir = dest_dir.clone();

        let result = executor.execute(&input).await;
        assert!(result.success, "{}", result.error_message);
        assert_eq!(result.files_processed, 2);
        assert_eq!(
            tokio::fs::read(dest_dir.join("root.txt")).await.unwrap(),
            b"root"
        );
        assert_eq!(
            tokio::fs::read(dest_dir.join("nested/child.txt"))
                .await
                .unwrap(),
            b"child"
        );
    }
}
