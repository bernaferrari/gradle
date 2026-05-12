use crate::server::task_executor::{
    option_string_list, option_string_map, TaskExecutor, TaskInput, TaskResult,
};

use super::process_launch::{run_to_output, ProcessLaunchSpec};

/// Executes a simple external process task.
///
/// This intentionally supports only a narrow, deterministic Exec contract:
/// executable, exact argument vector, working directory, and ignore-exit-value.
pub struct ExecTaskExecutor;

impl Default for ExecTaskExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecTaskExecutor {
    pub fn new() -> Self {
        Self
    }
}

#[tonic::async_trait]
impl TaskExecutor for ExecTaskExecutor {
    fn task_type(&self) -> &str {
        "Exec"
    }

    async fn execute(&self, input: &TaskInput) -> TaskResult {
        let start = std::time::Instant::now();
        let mut result = TaskResult::default();

        let executable = match input.options.get("executable").map(|value| value.trim()) {
            Some(value) if !value.is_empty() => value,
            _ => {
                result.success = false;
                result.error_message = "Exec task is missing executable".to_string();
                return result;
            }
        };
        let ignore_exit_value = input
            .options
            .get("ignore_exit_value")
            .map(|value| value == "true")
            .unwrap_or(false);

        let mut spec = ProcessLaunchSpec::new(executable)
            .args(option_string_list(&input.options, "args_json", "args"))
            .environment(option_string_map(
                &input.options,
                "environment_json",
                "environment",
            ));
        if let Some(working_dir) = input.options.get("working_dir") {
            if !working_dir.trim().is_empty() {
                spec = spec.working_dir(working_dir);
            }
        }

        match run_to_output(&spec).await {
            Ok(output) if output.exit_code == 0 || ignore_exit_value => {
                result.files_processed = 1;
                result.bytes_processed = (output.stdout.len() + output.stderr.len()) as u64;
            }
            Ok(output) => {
                result.success = false;
                result.error_message = format!(
                    "Exec task '{}' failed with exit code {}",
                    executable, output.exit_code
                );
            }
            Err(error) => {
                result.success = false;
                result.error_message = error;
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
    async fn test_exec_missing_executable_fails() {
        let executor = ExecTaskExecutor::new();
        let input = TaskInput::new("Exec");

        let result = executor.execute(&input).await;

        assert!(!result.success);
        assert!(result.error_message.contains("missing executable"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_exec_touch_creates_file() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("created.txt");
        let executor = ExecTaskExecutor::new();
        let mut input = TaskInput::new("Exec");
        input
            .options
            .insert("executable".to_string(), "/usr/bin/touch".to_string());
        input
            .options
            .insert("args".to_string(), target.to_string_lossy().into_owned());

        let result = executor.execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        assert!(target.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_exec_honors_ignore_exit_value() {
        let executor = ExecTaskExecutor::new();
        let mut input = TaskInput::new("Exec");
        input
            .options
            .insert("executable".to_string(), "/usr/bin/false".to_string());
        input
            .options
            .insert("ignore_exit_value".to_string(), "true".to_string());

        let result = executor.execute(&input).await;

        assert!(result.success, "{}", result.error_message);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_exec_preserves_json_args_with_spaces() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("created with space.txt");
        let executor = ExecTaskExecutor::new();
        let mut input = TaskInput::new("Exec");
        input
            .options
            .insert("executable".to_string(), "/usr/bin/touch".to_string());
        input.options.insert(
            "args_json".to_string(),
            serde_json::to_string(&vec![target.to_string_lossy().to_string()]).unwrap(),
        );

        let result = executor.execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        assert!(target.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_exec_passes_environment_json() {
        let tmp = tempfile::tempdir().unwrap();
        let target = tmp.path().join("env.txt");
        let executor = ExecTaskExecutor::new();
        let mut input = TaskInput::new("Exec");
        input
            .options
            .insert("executable".to_string(), "/bin/sh".to_string());
        input.options.insert(
            "args_json".to_string(),
            serde_json::to_string(&vec![
                "-c".to_string(),
                format!("printf %s \"$NATIVE_EXEC_ENV\" > {}", target.display()),
            ])
            .unwrap(),
        );
        input.options.insert(
            "environment_json".to_string(),
            serde_json::json!({"NATIVE_EXEC_ENV": "from-rust"}).to_string(),
        );

        let result = executor.execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        assert_eq!(std::fs::read_to_string(target).unwrap(), "from-rust");
    }
}
