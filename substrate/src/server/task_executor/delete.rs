use crate::server::task_executor::{TaskExecutor, TaskInput, TaskResult};

/// Deletes files and directories.
pub struct DeleteTaskExecutor;

impl Default for DeleteTaskExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl DeleteTaskExecutor {
    pub fn new() -> Self {
        Self
    }
}

#[tonic::async_trait]
impl TaskExecutor for DeleteTaskExecutor {
    fn task_type(&self) -> &str {
        "Delete"
    }

    async fn execute(&self, input: &TaskInput) -> TaskResult {
        let start = std::time::Instant::now();
        let mut result = TaskResult::default();

        // Option: "follow_symlinks" (default: false for safety)
        let follow_symlinks = input
            .options
            .get("follow_symlinks")
            .map(|v| v == "true")
            .unwrap_or(false);

        for target in &input.source_files {
            if !target.exists() {
                // Non-existent target is OK (idempotent delete)
                continue;
            }

            let is_symlink = target.is_symlink();
            if target.is_dir() && !is_symlink {
                // Delete directory recursively
                match tokio::fs::remove_dir_all(target).await {
                    Ok(()) => {
                        result.files_processed += 1;
                        result.removed_files.push(target.clone());
                    }
                    Err(e) => {
                        result.success = false;
                        result.error_message =
                            format!("Failed to delete directory {}: {}", target.display(), e);
                        return result;
                    }
                }
            } else {
                // Delete file or symlink
                match tokio::fs::remove_file(target).await {
                    Ok(()) => {
                        result.files_processed += 1;
                        result.removed_files.push(target.clone());
                    }
                    Err(e) => {
                        result.success = false;
                        result.error_message =
                            format!("Failed to delete {}: {}", target.display(), e);
                        return result;
                    }
                }
            }

            let _ = follow_symlinks; // Available for future use
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
    async fn test_delete_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("to_delete.txt");
        tokio::fs::write(&file, b"data").await.unwrap();
        assert!(file.exists());

        let executor = DeleteTaskExecutor::new();
        let mut input = TaskInput::new("Delete");
        input.source_files.push(file.clone());

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert_eq!(result.files_processed, 1);
        assert!(!file.exists());
    }

    #[tokio::test]
    async fn test_delete_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("to_delete");
        tokio::fs::create_dir_all(dir.join("nested")).await.unwrap();
        tokio::fs::write(dir.join("file.txt"), b"data")
            .await
            .unwrap();
        assert!(dir.exists());

        let executor = DeleteTaskExecutor::new();
        let mut input = TaskInput::new("Delete");
        input.source_files.push(dir.clone());

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert!(!dir.exists());
    }

    #[tokio::test]
    async fn test_delete_nonexistent_is_ok() {
        let executor = DeleteTaskExecutor::new();
        let mut input = TaskInput::new("Delete");
        input
            .source_files
            .push(PathBuf::from("/nonexistent/file.txt"));

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert_eq!(result.files_processed, 0);
    }

    #[tokio::test]
    async fn test_delete_multiple() {
        let tmp = tempfile::tempdir().unwrap();
        let mut files: Vec<std::path::PathBuf> = Vec::new();
        for i in 0..5 {
            let p = tmp.path().join(format!("file_{}.txt", i));
            tokio::fs::write(&p, b"data").await.unwrap();
            files.push(p);
        }

        let executor = DeleteTaskExecutor::new();
        let mut input = TaskInput::new("Delete");
        input.source_files = files.clone();

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert_eq!(result.files_processed, 5);
        for f in &files {
            assert!(!f.exists());
        }
    }

    #[tokio::test]
    async fn test_delete_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target.txt");
        tokio::fs::write(&target, b"data").await.unwrap();

        let link = tmp.path().join("link");
        tokio::fs::symlink(&target, &link).await.unwrap();
        assert!(link.is_symlink());

        let executor = DeleteTaskExecutor::new();
        let mut input = TaskInput::new("Delete");
        input.source_files.push(link.clone());

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert!(!link.exists());
        // Target should still exist
        assert!(target.exists());
    }
}
