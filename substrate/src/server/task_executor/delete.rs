use crate::server::task_executor::{TaskExecutor, TaskInput, TaskResult};

use std::collections::BTreeMap;
use std::path::Path;
use std::pin::Pin;
use std::time::Duration;

const DELETE_ATTEMPTS: usize = 10;
const DELETE_RETRY_SLEEP_MS: u64 = 10;

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

    fn delete_recursively<'a>(
        path: &'a Path,
        follow_symlinks: bool,
        result: &'a mut TaskResult,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<bool, String>> + Send + 'a>> {
        Box::pin(async move {
            let metadata = match tokio::fs::symlink_metadata(path).await {
                Ok(metadata) => metadata,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(e) => {
                    return Err(format!("Failed to inspect {}: {}", path.display(), e));
                }
            };

            if Self::should_descend(path, &metadata, follow_symlinks).await? {
                let mut children = Self::list_children(path).await?;
                children.sort_unstable();
                for child in children {
                    Self::delete_recursively(&child, follow_symlinks, result).await?;
                }
            }

            Self::delete_entry(path, &metadata, result).await?;
            Ok(true)
        })
    }

    async fn should_descend(
        path: &Path,
        metadata: &std::fs::Metadata,
        follow_symlinks: bool,
    ) -> Result<bool, String> {
        if metadata.file_type().is_symlink() {
            return if follow_symlinks {
                tokio::fs::metadata(path)
                    .await
                    .map(|target| target.is_dir())
                    .map_err(|e| {
                        format!("Failed to inspect symlink target {}: {}", path.display(), e)
                    })
            } else {
                Ok(false)
            };
        }
        Ok(metadata.is_dir())
    }

    async fn list_children(path: &Path) -> Result<Vec<std::path::PathBuf>, String> {
        let mut children = Vec::new();
        let mut entries = tokio::fs::read_dir(path)
            .await
            .map_err(|e| format!("Failed to read directory {}: {}", path.display(), e))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| format!("Failed to read directory {}: {}", path.display(), e))?
        {
            children.push(entry.path());
        }
        Ok(children)
    }

    async fn delete_entry(
        path: &Path,
        metadata: &std::fs::Metadata,
        result: &mut TaskResult,
    ) -> Result<(), String> {
        let is_directory = metadata.is_dir() && !metadata.file_type().is_symlink();
        match Self::try_hard_to_delete(path, is_directory).await {
            Ok(true) => {
                result.files_processed += 1;
                result.removed_files.push(path.to_path_buf());
                Ok(())
            }
            Ok(false) => Ok(()),
            Err(e) => Err(format!("Failed to delete {}: {}", path.display(), e)),
        }
    }

    async fn try_hard_to_delete(path: &Path, is_directory: bool) -> Result<bool, std::io::Error> {
        let mut last_error = None;
        for attempt in 0..DELETE_ATTEMPTS {
            match Self::delete_once(path, is_directory).await {
                Ok(deleted) => return Ok(deleted),
                Err(error) => {
                    last_error = Some(error);
                    let _ = Self::make_writable(path).await;
                    if attempt + 1 < DELETE_ATTEMPTS {
                        tokio::time::sleep(Duration::from_millis(DELETE_RETRY_SLEEP_MS)).await;
                    }
                }
            }
        }
        Err(last_error.unwrap_or_else(|| std::io::Error::other("delete failed")))
    }

    async fn delete_once(path: &Path, is_directory: bool) -> Result<bool, std::io::Error> {
        let delete_result = if is_directory {
            tokio::fs::remove_dir(path).await
        } else {
            tokio::fs::remove_file(path).await
        };
        match delete_result {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    #[cfg(unix)]
    async fn make_writable(path: &Path) -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let metadata = tokio::fs::symlink_metadata(path).await?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(permissions.mode() | 0o200);
        tokio::fs::set_permissions(path, permissions).await
    }

    #[cfg(not(unix))]
    async fn make_writable(path: &Path) -> std::io::Result<()> {
        let metadata = tokio::fs::symlink_metadata(path).await?;
        let mut permissions = metadata.permissions();
        permissions.set_readonly(false);
        tokio::fs::set_permissions(path, permissions).await
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

        let follow_symlinks = input
            .options
            .get("follow_symlinks")
            .map(|v| v == "true")
            .unwrap_or(false);

        for target in &input.source_files {
            if let Err(e) = Self::delete_recursively(target, follow_symlinks, &mut result).await {
                result.success = false;
                result.error_message = e;
                return result;
            }
        }

        result.duration_ms = start.elapsed().as_millis() as u64;
        result
    }
}

pub fn apply_vfs_delta_to_delete(
    _targets: &[std::path::PathBuf],
    delta: &BTreeMap<String, String>, // from DirectorySnapshot child_summaries fp:1229 + watch:766 get_snapshot_delta
) -> bool {
    if delta.is_empty() {
        return false;
    }
    for (changed, _h) in delta.iter() {
        let changed_l = changed.to_lowercase();
        if _targets
            .iter()
            .any(|t| t.to_string_lossy().to_lowercase().contains(&changed_l))
            || changed_l.contains("src")
            || changed_l.contains("build")
        {
            tracing::info!(target: "delete-lowering", vfs_taskexec_cross = true, changed = %changed, "VFS delta affects delete targets; re-execution likely");
            return true;
        }
    }
    false
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
    async fn test_delete_readonly_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("readonly.txt");
        tokio::fs::write(&file, b"data").await.unwrap();
        let mut permissions = tokio::fs::metadata(&file).await.unwrap().permissions();
        permissions.set_readonly(true);
        tokio::fs::set_permissions(&file, permissions)
            .await
            .unwrap();

        let executor = DeleteTaskExecutor::new();
        let mut input = TaskInput::new("Delete");
        input.source_files.push(file.clone());

        let result = executor.execute(&input).await;
        assert!(result.success, "{}", result.error_message);
        assert_eq!(result.files_processed, 1);
        assert!(!file.exists());
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

    #[tokio::test]
    async fn test_delete_directory_symlink_does_not_follow_by_default() {
        let tmp = tempfile::tempdir().unwrap();
        let target_dir = tmp.path().join("target");
        tokio::fs::create_dir_all(&target_dir).await.unwrap();
        let target_file = target_dir.join("file.txt");
        tokio::fs::write(&target_file, b"data").await.unwrap();

        let link = tmp.path().join("link");
        tokio::fs::symlink(&target_dir, &link).await.unwrap();

        let executor = DeleteTaskExecutor::new();
        let mut input = TaskInput::new("Delete");
        input.source_files.push(link.clone());

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert!(!link.exists());
        assert!(target_dir.exists());
        assert!(target_file.exists());
    }

    #[tokio::test]
    async fn test_delete_directory_symlink_follows_when_requested() {
        let tmp = tempfile::tempdir().unwrap();
        let target_dir = tmp.path().join("target");
        let nested_dir = target_dir.join("nested");
        tokio::fs::create_dir_all(&nested_dir).await.unwrap();
        let target_file = nested_dir.join("file.txt");
        tokio::fs::write(&target_file, b"data").await.unwrap();

        let link = tmp.path().join("link");
        tokio::fs::symlink(&target_dir, &link).await.unwrap();

        let executor = DeleteTaskExecutor::new();
        let mut input = TaskInput::new("Delete");
        input.source_files.push(link.clone());
        input
            .options
            .insert("follow_symlinks".to_string(), "true".to_string());

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert!(!link.exists());
        assert!(target_dir.exists());
        assert!(!nested_dir.exists());
        assert!(!target_file.exists());
    }
}
