use crate::server::task_executor::{TaskExecutor, TaskInput, TaskResult};

/// Executes no-action lifecycle aggregator tasks.
///
/// Gradle lifecycle tasks such as `classes` or `assemble` often only depend on
/// real work tasks. Once dependencies have completed, the task itself is a
/// successful no-op and should not require JVM forwarding.
pub struct LifecycleTaskExecutor;

impl Default for LifecycleTaskExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl LifecycleTaskExecutor {
    pub fn new() -> Self {
        Self
    }
}

#[tonic::async_trait]
impl TaskExecutor for LifecycleTaskExecutor {
    async fn execute(&self, _input: &TaskInput) -> TaskResult {
        TaskResult::default()
    }

    fn task_type(&self) -> &str {
        "Lifecycle"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lifecycle_task_is_successful_noop() {
        let executor = LifecycleTaskExecutor::new();
        let result = executor.execute(&TaskInput::new("Lifecycle")).await;

        assert!(result.success);
        assert!(result.output_files.is_empty());
        assert_eq!(result.files_processed, 0);
    }
}
