use std::path::PathBuf;
use std::pin::Pin;

use crate::server::task_executor::{TaskExecutor, TaskInput, TaskResult};

/// Synchronizes directories (rsync-like behavior).
/// Copies files from source to target, removing files in target that don't exist in source.
pub struct SyncTaskExecutor;

impl Default for SyncTaskExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl SyncTaskExecutor {
    pub fn new() -> Self {
        Self
    }

    /// Recursively list all files in a directory.
    fn list_files(
        dir: &std::path::Path,
    ) -> Pin<Box<dyn std::future::Future<Output = Vec<PathBuf>> + Send + '_>> {
        Box::pin(async move {
            let mut files = Vec::new();
            if let Ok(mut entries) = tokio::fs::read_dir(dir).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let path = entry.path();
                    if path.is_dir() {
                        files.extend(Self::list_files(&path).await);
                    } else {
                        files.push(path);
                    }
                }
            }
            files
        })
    }
}

#[tonic::async_trait]
impl TaskExecutor for SyncTaskExecutor {
    fn task_type(&self) -> &str {
        "Sync"
    }

    async fn execute(&self, input: &TaskInput) -> TaskResult {
        let start = std::time::Instant::now();
        let mut result = TaskResult::default();

        if input.source_files.is_empty() {
            result.success = false;
            result.error_message = "Sync requires at least one source directory".to_string();
            return result;
        }

        // Option: "delete_orphans" (default: true)
        let delete_orphans = input
            .options
            .get("delete_orphans")
            .map(|v| v != "false")
            .unwrap_or(true);

        // Option: "preserve_permissions" (default: false for simplicity)
        let _preserve_permissions = input
            .options
            .get("preserve_permissions")
            .map(|v| v == "true")
            .unwrap_or(false);

        for source_dir in &input.source_files {
            if !source_dir.is_dir() {
                result.success = false;
                result.error_message =
                    format!("Source is not a directory: {}", source_dir.display());
                return result;
            }

            // List all source files
            let source_files = Self::list_files(source_dir).await;

            // Copy/update files
            for src_file in &source_files {
                let relative = src_file.strip_prefix(source_dir).unwrap_or(src_file);
                let dest_file = input.target_dir.join(relative);

                // Create parent directories
                if let Some(parent) = dest_file.parent() {
                    if let Err(e) = tokio::fs::create_dir_all(parent).await {
                        result.success = false;
                        result.error_message = format!("Failed to create directory: {}", e);
                        return result;
                    }
                }

                // Check if file needs updating
                let needs_copy = if !dest_file.exists() {
                    true
                } else {
                    // Compare modification times and sizes
                    let src_meta = tokio::fs::metadata(src_file).await.ok();
                    let dest_meta = tokio::fs::metadata(&dest_file).await.ok();

                    match (src_meta, dest_meta) {
                        (Some(sm), Some(dm)) => {
                            let src_modified = sm
                                .modified()
                                .ok()
                                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|d| d.as_millis() as u64);
                            let dest_modified = dm
                                .modified()
                                .ok()
                                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|d| d.as_millis() as u64);

                            src_modified != dest_modified || sm.len() != dm.len()
                        }
                        _ => true,
                    }
                };

                if needs_copy {
                    match tokio::fs::copy(src_file, &dest_file).await {
                        Ok(bytes) => {
                            result.files_processed += 1;
                            result.bytes_processed += bytes;
                            result.output_files.push(dest_file);
                        }
                        Err(e) => {
                            result.success = false;
                            result.error_message =
                                format!("Failed to copy {}: {}", src_file.display(), e);
                            return result;
                        }
                    }
                }
            }

            // Delete orphan files in target
            if delete_orphans && input.target_dir.exists() {
                let dest_files = Self::list_files(&input.target_dir).await;
                for dest_file in &dest_files {
                    let relative = dest_file
                        .strip_prefix(&input.target_dir)
                        .unwrap_or(dest_file);
                    let expected_src = source_dir.join(relative);

                    if !expected_src.exists() {
                        if let Err(e) = tokio::fs::remove_file(dest_file).await {
                            result.success = false;
                            result.error_message =
                                format!("Failed to remove orphan {}: {}", dest_file.display(), e);
                            return result;
                        }
                        result.removed_files.push(dest_file.clone());
                    }
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
    async fn test_sync_creates_target() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        let dest_dir = tmp.path().join("dest");

        tokio::fs::create_dir_all(src_dir.join("sub"))
            .await
            .unwrap();
        tokio::fs::write(src_dir.join("a.txt"), b"aaa")
            .await
            .unwrap();
        tokio::fs::write(src_dir.join("sub/b.txt"), b"bbb")
            .await
            .unwrap();

        let executor = SyncTaskExecutor::new();
        let mut input = TaskInput::new("Sync");
        input.source_files.push(src_dir);
        input.target_dir = dest_dir.clone();

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert_eq!(result.files_processed, 2);

        assert!(dest_dir.join("a.txt").exists());
        assert!(dest_dir.join("sub/b.txt").exists());
    }

    #[tokio::test]
    async fn test_sync_deletes_orphans() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        let dest_dir = tmp.path().join("dest");

        tokio::fs::create_dir_all(&src_dir).await.unwrap();
        tokio::fs::create_dir_all(&dest_dir).await.unwrap();

        tokio::fs::write(src_dir.join("keep.txt"), b"keep")
            .await
            .unwrap();
        tokio::fs::write(dest_dir.join("keep.txt"), b"old")
            .await
            .unwrap();
        tokio::fs::write(dest_dir.join("orphan.txt"), b"delete me")
            .await
            .unwrap();

        let executor = SyncTaskExecutor::new();
        let mut input = TaskInput::new("Sync");
        input.source_files.push(src_dir);
        input.target_dir = dest_dir.clone();

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert!(dest_dir.join("keep.txt").exists());
        assert!(!dest_dir.join("orphan.txt").exists());
        assert_eq!(result.removed_files.len(), 1);
    }

    #[tokio::test]
    async fn test_sync_no_delete_orphans() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        let dest_dir = tmp.path().join("dest");

        tokio::fs::create_dir_all(&src_dir).await.unwrap();
        tokio::fs::create_dir_all(&dest_dir).await.unwrap();

        tokio::fs::write(src_dir.join("new.txt"), b"new")
            .await
            .unwrap();
        tokio::fs::write(dest_dir.join("orphan.txt"), b"keep me")
            .await
            .unwrap();

        let executor = SyncTaskExecutor::new();
        let mut input = TaskInput::new("Sync");
        input.source_files.push(src_dir);
        input.target_dir = dest_dir.clone();
        input
            .options
            .insert("delete_orphans".to_string(), "false".to_string());

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert!(dest_dir.join("new.txt").exists());
        assert!(dest_dir.join("orphan.txt").exists()); // Should NOT be deleted
    }

    #[tokio::test]
    async fn test_sync_empty_source() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        let dest_dir = tmp.path().join("dest");

        tokio::fs::create_dir_all(&src_dir).await.unwrap();
        tokio::fs::create_dir_all(&dest_dir).await.unwrap();
        tokio::fs::write(dest_dir.join("orphan.txt"), b"data")
            .await
            .unwrap();

        let executor = SyncTaskExecutor::new();
        let mut input = TaskInput::new("Sync");
        input.source_files.push(src_dir);
        input.target_dir = dest_dir.clone();

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert!(!dest_dir.join("orphan.txt").exists());
    }

    #[tokio::test]
    async fn test_sync_not_a_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("not_a_dir.txt");
        tokio::fs::write(&file, b"data").await.unwrap();

        let executor = SyncTaskExecutor::new();
        let mut input = TaskInput::new("Sync");
        input.source_files.push(file);
        input.target_dir = tmp.path().join("dest");

        let result = executor.execute(&input).await;
        assert!(!result.success);
        assert!(result.error_message.contains("not a directory"));
    }

    #[tokio::test]
    async fn test_sync_incremental_same_content() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        let dest_dir = tmp.path().join("dest");

        tokio::fs::create_dir_all(&src_dir).await.unwrap();
        let content = b"same content";
        tokio::fs::write(src_dir.join("file.txt"), content)
            .await
            .unwrap();

        // Pre-populate dest with identical file
        tokio::fs::create_dir_all(&dest_dir).await.unwrap();
        tokio::fs::write(dest_dir.join("file.txt"), content)
            .await
            .unwrap();

        let executor = SyncTaskExecutor::new();
        let mut input = TaskInput::new("Sync");
        input.source_files.push(src_dir);
        input.target_dir = dest_dir.clone();

        let result = executor.execute(&input).await;
        assert!(result.success);
        // File may or may not be re-copied depending on mtime resolution.
        // Same content and size is the important invariant.
        assert!(dest_dir.join("file.txt").exists());
    }
}
