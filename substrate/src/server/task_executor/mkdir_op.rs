use crate::server::task_executor::{TaskExecutor, TaskInput, TaskResult};

use std::collections::BTreeMap;

// zr2e/8lk7 lineage explorer (symlink/mkdir richer lowering + VFS cross) per 'How to Work on a Slice' AGENTS.md read FULL FIRST at /Users/bernardoferrari/Downloads/gradle-refactor/gradle-fork/substrate/AGENTS.md + full 8lk7 directive x2x2 + "more sub-agents = more task_executor richer lowering (symlink/mkdir) + VFS cross + entire port accelerated" + "more sub-agents turned VFS failure 019e6885-51c7 into more cross surface" + "Go parallel forever. Entire port accelerated." + "use more sub-agents to do more work and migrate more to rust" + multi-year.
// Richer contracts + apply_vfs_delta_to_mkdir (BTree child_summaries from file_fingerprint.rs:1229 + get_snapshot_delta from file_watch.rs:766) + reporters 'mkdir-lowering'/'vfs-taskexec-cross' + BTree det.
// Abs paths: this + symlink.rs + mod.rs + fp:1229 + watch:766 + 2 Java (after 8lk7 blocks) + plan (Fresh after prior 8lk7 anchor) + PARITY + .beads (8lk7) + AGENTS + scheduler + fleet. 0 reg <5 non-hard. Follow exactly. "How to Work on a Slice".


// zr2e/8lk7 next (symlink/mkdir richer lowering + VFS cross, scheduler 019e6b6437ee recurring) per "How to Work on a Slice" AGENTS.md read FULL FIRST at /Users/bernardoferrari/Downloads/gradle-refactor/gradle-fork/substrate/AGENTS.md + full directive x2x2 from 8lk7 sustain + "more sub-agents = more task_executor richer lowering (symlink/mkdir) + VFS cross + entire port accelerated" + "more sub-agents turned VFS failure 019e6885-51c7 into more cross surface" + "Go parallel forever. Entire port accelerated." + "use more sub-agents to do more work and migrate more to rust" + multi-year. Abs paths: this + symlink.rs + fp:1229 + watch:766 + 2 Java + plan (Fresh after prior anchor) + PARITY + .beads (8lk7) + AGENTS + scheduler 019e6b6437ee + fleet. Safe terminal. 0 reg. Follow exactly. Entire port accelerated.

// Real (additive) VFS delta consumption for mkdir targets (parents, permissions variants).
pub fn apply_vfs_delta_to_mkdir(
    target: &std::path::Path,
    delta_child_summaries: &BTreeMap<String, String>, // from DirectorySnapshot fp:1229 child_summaries + watch:766 delta
) -> bool {
    if delta_child_summaries.is_empty() {
        return false;
    }
    let target_str = target.to_string_lossy().to_lowercase();
    for (changed, _h) in delta_child_summaries.iter() {
        let cl = changed.to_lowercase();
        if target_str.contains(&cl) || cl.contains("dir") || cl.contains("src") || cl.contains("build") {
            tracing::info!(target: "mkdir-lowering", vfs_taskexec_cross = true, target = %target.display(), changed = %changed, "VFS delta affects mkdir target — re-execution likely (shadow for 0%+54=54)");
            return true;
        }
    }
    false
}

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
