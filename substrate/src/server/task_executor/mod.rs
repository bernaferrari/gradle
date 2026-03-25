mod copy;
mod delete;
mod java_compile;
mod mkdir_op;
mod symlink;
mod sync;
mod test_exec;

pub use copy::CopyTaskExecutor;
pub use delete::DeleteTaskExecutor;
pub use java_compile::JavaCompileExecutor;
pub use mkdir_op::MkdirTaskExecutor;
pub use symlink::SymlinkTaskExecutor;
pub use sync::SyncTaskExecutor;
pub use test_exec::TestExecExecutor;

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
            "Copy" | "Delete" | "Sync" | "Mkdir" | "Symlink" | "JavaCompile" | "TestExec"
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

        let test_exec = TestExecExecutor::new();
        executors.insert(test_exec.task_type().to_string(), Box::new(test_exec));

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
        assert!(types.contains(&"TestExec"));
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
        assert!(TaskInput::is_native_supported("TestExec"));
        assert!(!TaskInput::is_native_supported("Test"));
    }
}
