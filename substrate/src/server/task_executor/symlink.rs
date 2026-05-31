use crate::server::task_executor::{TaskExecutor, TaskInput, TaskResult};

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub fn apply_vfs_delta_to_symlink(
    target: &std::path::Path,
    delta_child_summaries: &BTreeMap<String, String>, // from DirectorySnapshot fp:1229 child_summaries + watch:766 delta
) -> bool {
    if delta_child_summaries.is_empty() {
        return false;
    }
    let target_str = target.to_string_lossy().to_lowercase();
    for (changed, _h) in delta_child_summaries.iter() {
        let cl = changed.to_lowercase();
        if target_str.contains(&cl)
            || cl.contains("link")
            || cl.contains("src")
            || cl.contains("build")
        {
            tracing::info!(target: "symlink-lowering", vfs_taskexec_cross = true, target = %target.display(), changed = %changed, "VFS delta affects symlink target; re-execution likely");
            return true;
        }
    }
    false
}

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

    fn link_specs(input: &TaskInput) -> Vec<(PathBuf, PathBuf)> {
        if input.source_files.len() == 2 && input.target_dir.as_os_str().is_empty() {
            return vec![(input.source_files[0].clone(), input.source_files[1].clone())];
        }

        input
            .source_files
            .iter()
            .map(|target| {
                (
                    target.clone(),
                    input
                        .target_dir
                        .join(target.file_name().unwrap_or_default()),
                )
            })
            .collect()
    }

    async fn create_link(
        target: &Path,
        link_path: &Path,
        force: bool,
        result: &mut TaskResult,
    ) -> Result<(), String> {
        if link_path.exists() || link_path.is_symlink() {
            if !force {
                return Err(format!("Link path already exists: {}", link_path.display()));
            }
            tokio::fs::remove_file(link_path).await.map_err(|e| {
                format!(
                    "Failed to remove existing link {}: {}",
                    link_path.display(),
                    e
                )
            })?;
        }

        if let Some(parent) = link_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                format!(
                    "Failed to create parent directory {}: {}",
                    parent.display(),
                    e
                )
            })?;
        }

        tokio::fs::symlink(target, link_path).await.map_err(|e| {
            format!(
                "Failed to create symlink {} -> {}: {}",
                link_path.display(),
                target.display(),
                e
            )
        })?;

        result.files_processed += 1;
        result.output_files.push(link_path.to_path_buf());
        Ok(())
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

        if input.source_files.is_empty() {
            result.success = false;
            result.error_message = "Symlink requires at least one source file".to_string();
            return result;
        }

        let force = input
            .options
            .get("force")
            .map(|value| value != "false")
            .unwrap_or(true);

        for (target, link_path) in Self::link_specs(input) {
            if let Err(e) = Self::create_link(&target, &link_path, force, &mut result).await {
                result.success = false;
                result.error_message = e;
                return result;
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
        assert_eq!(result.files_processed, 1);
        assert!(link.is_symlink());
        let content = std::fs::read_to_string(std::fs::canonicalize(&link).unwrap()).unwrap();
        assert_eq!(content, "data");
    }

    #[tokio::test]
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
    async fn test_symlink_force_false_preserves_existing_path() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("target.txt");
        let link = tmp.path().join("link.txt");
        std::fs::write(&target, b"new data").unwrap();
        std::fs::write(&link, b"old data").unwrap();

        let executor = SymlinkTaskExecutor::new();
        let mut input = TaskInput::new("Symlink");
        input.source_files.push(target);
        input.source_files.push(link.clone());
        input
            .options
            .insert("force".to_string(), "false".to_string());

        let result = executor.execute(&input).await;
        assert!(!result.success);
        assert!(result.error_message.contains("already exists"));
        assert_eq!(std::fs::read_to_string(&link).unwrap(), "old data");
    }

    #[tokio::test]
    async fn test_symlink_two_sources_with_target_dir_creates_two_links() {
        let tmp = tempfile::tempdir().unwrap();
        let target_a = tmp.path().join("a.txt");
        let target_b = tmp.path().join("b.txt");
        let link_dir = tmp.path().join("links");
        std::fs::write(&target_a, b"a").unwrap();
        std::fs::write(&target_b, b"b").unwrap();

        let executor = SymlinkTaskExecutor::new();
        let mut input = TaskInput::new("Symlink");
        input.source_files.push(target_a.clone());
        input.source_files.push(target_b.clone());
        input.target_dir = link_dir.clone();

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert_eq!(result.files_processed, 2);
        assert_eq!(
            std::fs::canonicalize(link_dir.join("a.txt")).unwrap(),
            std::fs::canonicalize(&target_a).unwrap()
        );
        assert_eq!(
            std::fs::canonicalize(link_dir.join("b.txt")).unwrap(),
            std::fs::canonicalize(&target_b).unwrap()
        );
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
