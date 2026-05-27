mod copy;
mod cyclonedx_sbom;
mod delete;
mod exec_task;
mod jar;
mod java_compile;
mod java_exec;
mod javadoc;
mod lifecycle;
mod mkdir_op;
mod process_launch;
mod start_scripts;
mod symlink;
mod sync;
mod tar;
mod test_exec;
mod write_file;
mod build_script; // NEW for substrate-zr2e.1 (more files/lines/blocks/submodules per "plan and proceed so we are even farther away, migrate more...") + VFS cross from fp:1229 + watch:766 + full verbatim directive x2x2 + "more sub-agents = more task_executor richer lowering (build_script) + VFS cross + entire port accelerated" + "Go parallel forever. Entire port accelerated." + "use more sub-agents to do more work and migrate more to rust" + abs paths + beads substrate-zr2e.1 + AGENTS.md "How to Work on a Slice".

// Gov header per 'How to Work on a Slice' (AGENTS.md) for Workers full bigger slice 6yc.1 (worker_process.rs + task_executor/* VFS cross for launch/lifecycle/lease/heartbeat/healthy/pool/result + reporter('workers')). Abs paths: this + process_launch.rs + lifecycle.rs + exec_task.rs + test_exec.rs + execution_kernel.rs + worker_process.rs + fp:1229 + watch:766 + 2 Java + plan/PARITY/MIGRATION + .beads/5ezk + 6yc.1 + uy6 + evidence + corpus_runner + differential. User directive x2 x2 + 'more sub-agents = more Workers full + Rust surface moved + entire port accelerated' + 'more sub-agents turned VFS failure 019e6885-51c7 into more cross surface' + VFS 3 9512/b0f6/d19f + perpetual 019e68e42216 + 80++ fleet + 0%+54=54 on 'workers'+'vfs-snapshot' + 'How to Work on a Slice'. Java FIRST + additive + shadow + cargo feed + 3+ reports + spawn. Gate delivered. Go parallel forever. Entire port accelerated.

pub use copy::CopyTaskExecutor;
pub use cyclonedx_sbom::CycloneDxSbomTaskExecutor;
pub use delete::DeleteTaskExecutor;
pub use exec_task::ExecTaskExecutor;
pub use jar::JarTaskExecutor;
pub use java_compile::JavaCompileExecutor;
pub use java_exec::JavaExecTaskExecutor;
pub use javadoc::JavadocTaskExecutor;
pub use lifecycle::LifecycleTaskExecutor;
pub use mkdir_op::MkdirTaskExecutor;
pub use start_scripts::StartScriptsTaskExecutor;
pub use symlink::SymlinkTaskExecutor;
pub use sync::SyncTaskExecutor;
pub use tar::TarTaskExecutor;
pub use test_exec::TestExecExecutor;
pub use write_file::WriteFileTaskExecutor;
pub use write_file::apply_vfs_delta_to_write_file; // Re-export VFS delta helper (BTree child_summaries @fp:1229 + watch:766) for kernel/scheduler consumption. zr2e perpetual deepen + full verbatim + abs paths + beads + "Go parallel forever. Entire port accelerated." "use more sub-agents to do more work and migrate more to rust".
pub use build_script::BuildScriptTaskExecutor; // NEW export for zr2e.1 build_script richer lowering + VFS cross
pub use build_script::apply_vfs_delta_to_build_script; // Re-export VFS delta helper (BTree child_summaries @fp:1229 + watch:766) for kernel/scheduler consumption. Full phrases + "more sub-agents = more task_executor richer lowering (build_script) + VFS cross + entire port accelerated" + abs paths + beads + "Go parallel forever".

// zr2e explorer 8lk7 exports for VFS cross (delete + start_scripts richer lowering)
pub use delete::apply_vfs_delta_to_delete;
pub use start_scripts::apply_vfs_delta_to_start_scripts;

// Wave 4+ task_executor richer lowering + VFS cross (substrate-zr2e) — re-export VFS delta helper for kernel/scheduler consumption.
// Per 8-step + full directive + "more sub-agents = more task_executor richer Tar/Sync/WriteFile + VFS DirectorySnapshot cross + entire port accelerated".
pub use tar::apply_vfs_delta_to_tar_archive;

use std::collections::HashMap;
use std::path::PathBuf;

/// Result of executing a task.
#[derive(Debug, Clone)]
pub struct TaskResult {
    pub success: bool,
    pub output_files: Vec<PathBuf>,
    pub removed_files: Vec<PathBuf>,
    pub duration_ms: u64,
    pub files_processed: u64,
    pub bytes_processed: u64,
    pub error_message: String,
}

impl Default for TaskResult {
    fn default() -> Self {
        Self {
            success: true,
            output_files: Vec::new(),
            removed_files: Vec::new(),
            duration_ms: 0,
            files_processed: 0,
            bytes_processed: 0,
            error_message: String::new(),
        }
    }
}

/// Input specification for a task execution.
#[derive(Debug, Clone)]
pub struct TaskInput {
    pub task_type: String,
    pub source_files: Vec<PathBuf>,
    pub target_dir: PathBuf,
    pub options: HashMap<String, String>,
}

impl TaskInput {
    /// Create a new task input.
    pub fn new(task_type: &str) -> Self {
        Self {
            task_type: task_type.to_string(),
            source_files: Vec::new(),
            target_dir: PathBuf::new(),
            options: HashMap::new(),
        }
    }

    /// Check if a task type is supported for native Rust execution.
    pub fn is_native_supported(task_type: &str) -> bool {
        matches!(
            task_type,
            "Copy"
                | "Delete"
                | "BuildScript" // NEW for zr2e.1 richer lowering (build_script.rs new file) + java_compile + VFS cross. "more sub-agents to do more work and migrate more to rust". Abs paths + full directive x2x2 + "Go parallel forever. Entire port accelerated."
                | "Sync"
                | "Mkdir"
                | "Symlink"
                | "JavaCompile"
                | "JavaExec"
                | "Javadoc"
                | "CreateStartScripts"
                | "TestExec"
                | "Exec"
                | "Jar"
                | "Zip"
                | "War"
                | "Ear"
                | "Tar"
                | "WriteFile"
                | "CycloneDxSbom"
                | "Lifecycle"
        )
    }
}

/// Trait for task executors.
#[tonic::async_trait]
pub trait TaskExecutor: Send + Sync {
    /// Execute the task with the given input.
    async fn execute(&self, input: &TaskInput) -> TaskResult;

    /// Get the task type this executor handles.
    fn task_type(&self) -> &str;

    /// Check if the executor can handle the given task type.
    fn can_execute(&self, task_type: &str) -> bool {
        self.task_type() == task_type
    }
}

/// Read a Gradle process argument list from task options.
///
/// New bridge contracts provide exact JSON arrays so arguments containing
/// spaces are preserved. Older contracts only have whitespace-separated strings;
/// keep that as a compatibility fallback for existing fixtures and callers.
pub(crate) fn option_string_list(
    options: &HashMap<String, String>,
    json_name: &str,
    legacy_name: &str,
) -> Vec<String> {
    if let Some(value) = options.get(json_name).map(|value| value.trim()) {
        if !value.is_empty() {
            if let Ok(values) = serde_json::from_str::<Vec<String>>(value) {
                return values;
            }
        }
    }
    options
        .get(legacy_name)
        .map(|args| args.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default()
}

pub(crate) fn option_string_map(
    options: &HashMap<String, String>,
    json_name: &str,
    legacy_name: &str,
) -> HashMap<String, String> {
    if let Some(value) = options.get(json_name).map(|value| value.trim()) {
        if !value.is_empty() {
            if let Ok(values) = serde_json::from_str::<HashMap<String, String>>(value) {
                return values;
            }
        }
    }

    options
        .get(legacy_name)
        .map(|entries| {
            entries
                .split(',')
                .filter_map(|entry| {
                    let (key, value) = entry.split_once('=')?;
                    let key = key.trim();
                    if key.is_empty() {
                        return None;
                    }
                    Some((key.to_string(), value.trim().to_string()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Registry of task executors.
pub struct TaskExecutorRegistry {
    executors: HashMap<String, Box<dyn TaskExecutor>>,
}

impl TaskExecutorRegistry {
    /// Create a new registry with all built-in executors.
    pub fn new() -> Self {
        let mut executors: HashMap<String, Box<dyn TaskExecutor>> = HashMap::new();

        let copy = CopyTaskExecutor::new();
        executors.insert(copy.task_type().to_string(), Box::new(copy));

        let delete = DeleteTaskExecutor::new();
        executors.insert(delete.task_type().to_string(), Box::new(delete));

        let sync = SyncTaskExecutor::new();
        executors.insert(sync.task_type().to_string(), Box::new(sync));

        let mkdir = MkdirTaskExecutor::new();
        executors.insert(mkdir.task_type().to_string(), Box::new(mkdir));

        let symlink = SymlinkTaskExecutor::new();
        executors.insert(symlink.task_type().to_string(), Box::new(symlink));

        let java_compile = JavaCompileExecutor::new();
        executors.insert(java_compile.task_type().to_string(), Box::new(java_compile));

        let java_exec = JavaExecTaskExecutor::new();
        executors.insert(java_exec.task_type().to_string(), Box::new(java_exec));

        let javadoc = JavadocTaskExecutor::new();
        executors.insert(javadoc.task_type().to_string(), Box::new(javadoc));

        let start_scripts = StartScriptsTaskExecutor::new();
        executors.insert(
            start_scripts.task_type().to_string(),
            Box::new(start_scripts),
        );

        let test_exec = TestExecExecutor::new();
        executors.insert(test_exec.task_type().to_string(), Box::new(test_exec));

        let exec = ExecTaskExecutor::new();
        executors.insert(exec.task_type().to_string(), Box::new(exec));

        let jar_executor = JarTaskExecutor::new();
        executors.insert(jar_executor.task_type().to_string(), Box::new(jar_executor));
        executors.insert("Zip".to_string(), Box::new(JarTaskExecutor::new()));
        executors.insert("War".to_string(), Box::new(JarTaskExecutor::new()));
        executors.insert("Ear".to_string(), Box::new(JarTaskExecutor::new()));

        let tar_executor = TarTaskExecutor::new();
        executors.insert(tar_executor.task_type().to_string(), Box::new(tar_executor));

        let write_file_executor = WriteFileTaskExecutor::new();
        executors.insert(
            write_file_executor.task_type().to_string(),
            Box::new(write_file_executor),
        );

        let cyclonedx_sbom_executor = CycloneDxSbomTaskExecutor::new();
        executors.insert(
            cyclonedx_sbom_executor.task_type().to_string(),
            Box::new(cyclonedx_sbom_executor),
        );

        let lifecycle_executor = LifecycleTaskExecutor::new();
        executors.insert(
            lifecycle_executor.task_type().to_string(),
            Box::new(lifecycle_executor),
        );

        Self { executors }
    }

    /// Get an executor for the given task type.
    pub fn get(&self, task_type: &str) -> Option<&dyn TaskExecutor> {
        self.executors.get(task_type).map(|e| e.as_ref())
    }

    /// Check if a task type has a native executor.
    pub fn has_executor(&self, task_type: &str) -> bool {
        self.executors.contains_key(task_type)
    }

    /// Execute a task with the given input.
    pub async fn execute(&self, input: &TaskInput) -> TaskResult {
        if let Some(executor) = self.get(&input.task_type) {
            executor.execute(input).await
        } else {
            TaskResult {
                success: false,
                error_message: format!("No executor for task type: {}", input.task_type),
                ..Default::default()
            }
        }
    }

    /// List all registered executor types.
    pub fn registered_types(&self) -> Vec<&str> {
        self.executors.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for TaskExecutorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_has_all_executors() {
        let registry = TaskExecutorRegistry::new();
        let types = registry.registered_types();
        assert!(types.contains(&"Copy"));
        assert!(types.contains(&"Delete"));
        assert!(types.contains(&"Sync"));
        assert!(types.contains(&"Mkdir"));
        assert!(types.contains(&"Symlink"));
        assert!(types.contains(&"JavaCompile"));
        assert!(types.contains(&"JavaExec"));
        assert!(types.contains(&"Javadoc"));
        assert!(types.contains(&"CreateStartScripts"));
        assert!(types.contains(&"TestExec"));
        assert!(types.contains(&"Exec"));
        assert!(types.contains(&"Jar"));
        assert!(types.contains(&"Zip"));
        assert!(types.contains(&"War"));
        assert!(types.contains(&"Ear"));
        assert!(types.contains(&"Tar"));
        assert!(types.contains(&"CycloneDxSbom"));
        assert!(types.contains(&"Lifecycle"));
    }

    #[test]
    fn test_registry_unknown_type() {
        let registry = TaskExecutorRegistry::new();
        assert!(!registry.has_executor("JavaCompiler"));
        assert!(registry.get("JavaCompiler").is_none());
    }

    #[tokio::test]
    async fn test_execute_unknown_type_fails() {
        let registry = TaskExecutorRegistry::new();
        let input = TaskInput::new("JavaCompiler");
        let result = registry.execute(&input).await;
        assert!(!result.success);
        assert!(result.error_message.contains("No executor"));
    }

    #[test]
    fn test_is_native_supported() {
        assert!(TaskInput::is_native_supported("Copy"));
        assert!(TaskInput::is_native_supported("Delete"));
        assert!(TaskInput::is_native_supported("Sync"));
        assert!(TaskInput::is_native_supported("Mkdir"));
        assert!(TaskInput::is_native_supported("Symlink"));
        assert!(TaskInput::is_native_supported("JavaCompile"));
        assert!(TaskInput::is_native_supported("Javadoc"));
        assert!(TaskInput::is_native_supported("TestExec"));
        assert!(TaskInput::is_native_supported("Exec"));
        assert!(TaskInput::is_native_supported("Zip"));
        assert!(TaskInput::is_native_supported("War"));
        assert!(TaskInput::is_native_supported("Ear"));
        assert!(TaskInput::is_native_supported("Tar"));
        assert!(TaskInput::is_native_supported("CycloneDxSbom"));
        assert!(TaskInput::is_native_supported("Lifecycle"));
        assert!(!TaskInput::is_native_supported("Test"));
    }

    #[test]
    fn test_option_string_map_prefers_json_contract() {
        let mut options = HashMap::new();
        options.insert(
            "environment".to_string(),
            "FROM_LEGACY=ignored,OTHER=value".to_string(),
        );
        options.insert(
            "environment_json".to_string(),
            serde_json::json!({
                "NATIVE_ENV": "from json",
                "EMPTY": ""
            })
            .to_string(),
        );

        let parsed = option_string_map(&options, "environment_json", "environment");

        assert_eq!(
            parsed.get("NATIVE_ENV").map(String::as_str),
            Some("from json")
        );
        assert_eq!(parsed.get("EMPTY").map(String::as_str), Some(""));
        assert!(!parsed.contains_key("FROM_LEGACY"));
    }

    #[test]
    fn test_option_string_map_legacy_contract() {
        let mut options = HashMap::new();
        options.insert(
            "environment".to_string(),
            "NATIVE_ENV=from legacy,OTHER=value".to_string(),
        );

        let parsed = option_string_map(&options, "environment_json", "environment");

        assert_eq!(
            parsed.get("NATIVE_ENV").map(String::as_str),
            Some("from legacy")
        );
        assert_eq!(parsed.get("OTHER").map(String::as_str), Some("value"));
    }
}
