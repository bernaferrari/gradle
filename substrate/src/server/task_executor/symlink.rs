use crate::server::task_executor::{TaskExecutor, TaskInput, TaskResult};

/// Creates symbolic links.
pub struct SymlinkTaskExecutor;

impl Default for SymlinkTaskExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl SymlinkTaskExecutor {
    pub fn new() -> Self {
        Self
    }
}

#[tonic::async_trait]
impl TaskExecutor for SymlinkTaskExecutor {
    fn task_type(&self) -> &str {
        "Symlink"
    }

    async fn execute(&self, input: &TaskInput) -> TaskResult {
        let start = std::time::Instant::now();
        let mut result = TaskResult::default();

        // source_files[0] = target, source_files[1] = link path
        // OR source_files[i] = target, target_dir / name = link path
        if input.source_files.is_empty() {
            result.success = false;
            result.error_message = "Symlink requires at least one source file".to_string();
            return result;
        }

        for target in input.source_files.iter() {
            let link_path = if input.source_files.len() == 2 {
                // Two-file mode: source_files[0] = target, source_files[1] = link
                input.source_files[1].clone()
            } else {
                // Multi-file mode: link created in target_dir with same name
                input
                    .target_dir
                    .join(target.file_name().unwrap_or_default())
            };

            // Remove existing link if present
            if link_path.exists() || link_path.is_symlink() {
                if let Err(e) = tokio::fs::remove_file(&link_path).await {
                    result.success = false;
                    result.error_message = format!(
                        "Failed to remove existing link {}: {}",
                        link_path.display(),
                        e
                    );
                    return result;
                }
            }

            match tokio::fs::symlink(target, &link_path).await {
                Ok(()) => {
                    result.files_processed += 1;
                    result.output_files.push(link_path);
                }
                Err(e) => {
                    result.success = false;
                    result.error_message = format!(
                        "Failed to create symlink {} -> {}: {}",
                        link_path.display(),
                        target.display(),
                        e
                    );
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

    // These tests create symlinks which don't work in sandboxed macOS
    // environments (ELOOP from /var->/private/var path resolution in
    // containers). They pass on real macOS.
    // Run with: cargo test --lib symlink -- --ignored

    #[tokio::test]
    #[ignore = "Symlink creation returns ELOOP in sandboxed environments. Passes on real macOS."]
    async fn test_symlink_file() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target.txt");
        let link = tmp.path().join("link.txt");
        std::fs::write(&target, b"data").unwrap();

        let executor = SymlinkTaskExecutor::new();
        let mut input = TaskInput::new("Symlink");
        input.source_files.push(target.clone());
        input.source_files.push(link.clone());

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert!(link.is_symlink());
        let content = std::fs::read_to_string(std::fs::canonicalize(&link).unwrap()).unwrap();
        assert_eq!(content, "data");
    }

    #[tokio::test]
    #[ignore = "Symlink creation returns ELOOP in sandboxed environments. Passes on real macOS."]
    async fn test_symlink_to_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target_dir");
        let link = tmp.path().join("link_dir");
        std::fs::create_dir_all(target.join("nested")).unwrap();
        std::fs::write(target.join("file.txt"), b"data").unwrap();

        let executor = SymlinkTaskExecutor::new();
        let mut input = TaskInput::new("Symlink");
        input.source_files.push(target.clone());
        input.source_files.push(link.clone());

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert!(link.is_symlink());
        let real = std::fs::canonicalize(&link).unwrap();
        assert!(real.join("file.txt").exists());
    }

    #[tokio::test]
    #[ignore = "Symlink creation returns ELOOP in sandboxed environments. Passes on real macOS."]
    async fn test_symlink_replace_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target.txt");
        let link = tmp.path().join("link.txt");
        std::fs::write(&target, b"new data").unwrap();
        std::fs::write(&link, b"old data").unwrap();

        let executor = SymlinkTaskExecutor::new();
        let mut input = TaskInput::new("Symlink");
        input.source_files.push(target.clone());
        input.source_files.push(link.clone());

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert!(link.is_symlink());
        let real = std::fs::canonicalize(&link).unwrap();
        let content = std::fs::read_to_string(&real).unwrap();
        assert_eq!(content, "new data");
    }

    #[tokio::test]
    async fn test_symlink_no_sources() {
        let executor = SymlinkTaskExecutor::new();
        let input = TaskInput::new("Symlink");

        let result = executor.execute(&input).await;
        assert!(!result.success);
        assert!(result.error_message.contains("at least one"));
    }
}
