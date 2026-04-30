use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::server::task_executor::{TaskExecutor, TaskInput, TaskResult};

/// Executes a Gradle JavaExec task from an explicit, native-ready contract.
pub struct JavaExecTaskExecutor;

impl Default for JavaExecTaskExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl JavaExecTaskExecutor {
    pub fn new() -> Self {
        Self
    }

    fn java_executable(java_home: Option<&str>) -> PathBuf {
        match java_home.map(str::trim).filter(|home| !home.is_empty()) {
            Some(home) => Path::new(home).join("bin").join(java_binary_name()),
            None => PathBuf::from(java_binary_name()),
        }
    }

    fn split_args(value: Option<&String>) -> Vec<&str> {
        value
            .map(|args| args.split_whitespace().collect())
            .unwrap_or_default()
    }
}

#[cfg(windows)]
fn java_binary_name() -> &'static str {
    "java.exe"
}

#[cfg(not(windows))]
fn java_binary_name() -> &'static str {
    "java"
}

#[tonic::async_trait]
impl TaskExecutor for JavaExecTaskExecutor {
    fn task_type(&self) -> &str {
        "JavaExec"
    }

    async fn execute(&self, input: &TaskInput) -> TaskResult {
        let start = std::time::Instant::now();
        let mut result = TaskResult::default();

        let classpath = match input.options.get("classpath").map(|value| value.trim()) {
            Some(value) if !value.is_empty() => value,
            _ => {
                result.success = false;
                result.error_message = "JavaExec task is missing classpath".to_string();
                return result;
            }
        };
        let main_class = match input.options.get("main_class").map(|value| value.trim()) {
            Some(value) if !value.is_empty() => value,
            _ => {
                result.success = false;
                result.error_message = "JavaExec task is missing main_class".to_string();
                return result;
            }
        };
        let ignore_exit_value = input
            .options
            .get("ignore_exit_value")
            .map(|value| value == "true")
            .unwrap_or(false);
        let java = Self::java_executable(input.options.get("java_home").map(String::as_str));

        let mut command = Command::new(&java);
        command.args(Self::split_args(input.options.get("jvm_args")));
        command.arg("-cp");
        command.arg(classpath);
        command.arg(main_class);
        command.args(Self::split_args(input.options.get("args")));
        if let Some(working_dir) = input.options.get("working_dir") {
            if !working_dir.trim().is_empty() {
                command.current_dir(working_dir);
            }
        }

        match command.output().await {
            Ok(output) if output.status.success() || ignore_exit_value => {
                result.files_processed = 1;
                result.bytes_processed = (output.stdout.len() + output.stderr.len()) as u64;
            }
            Ok(output) => {
                result.success = false;
                result.error_message = format!(
                    "JavaExec task '{}' failed with exit code {}",
                    main_class,
                    output.status.code().unwrap_or(-1)
                );
            }
            Err(error) => {
                result.success = false;
                result.error_message = format!("Failed to execute '{}': {}", java.display(), error);
            }
        }

        result.duration_ms = start.elapsed().as_millis() as u64;
        result
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::process::Command as StdCommand;

    use super::*;

    fn current_java_home() -> Option<String> {
        std::env::var("JAVA_HOME").ok().or_else(|| {
            let java_home = std::env::var("java_home").ok();
            java_home.filter(|value| !value.trim().is_empty())
        })
    }

    #[tokio::test]
    async fn test_java_exec_missing_main_class_fails() {
        let executor = JavaExecTaskExecutor::new();
        let mut input = TaskInput::new("JavaExec");
        input
            .options
            .insert("classpath".to_string(), "/tmp/classes".to_string());

        let result = executor.execute(&input).await;

        assert!(!result.success);
        assert!(result.error_message.contains("missing main_class"));
    }

    #[tokio::test]
    async fn test_java_exec_missing_classpath_fails() {
        let executor = JavaExecTaskExecutor::new();
        let mut input = TaskInput::new("JavaExec");
        input
            .options
            .insert("main_class".to_string(), "Tool".to_string());

        let result = executor.execute(&input).await;

        assert!(!result.success);
        assert!(result.error_message.contains("missing classpath"));
    }

    #[tokio::test]
    async fn test_java_exec_runs_main_class() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("Tool.java");
        let output = tmp.path().join("result.txt");
        let mut file = std::fs::File::create(&source).unwrap();
        writeln!(
            file,
            "public class Tool {{ public static void main(String[] args) throws Exception {{ java.nio.file.Files.writeString(java.nio.file.Path.of(args[0]), args[1]); }} }}"
        )
        .unwrap();

        let javac = current_java_home()
            .map(|home| {
                Path::new(&home)
                    .join("bin")
                    .join(if cfg!(windows) { "javac.exe" } else { "javac" })
            })
            .unwrap_or_else(|| PathBuf::from(if cfg!(windows) { "javac.exe" } else { "javac" }));
        let compile = StdCommand::new(javac)
            .arg(&source)
            .current_dir(tmp.path())
            .output()
            .unwrap();
        assert!(
            compile.status.success(),
            "{}",
            String::from_utf8_lossy(&compile.stderr)
        );

        let executor = JavaExecTaskExecutor::new();
        let mut input = TaskInput::new("JavaExec");
        if let Some(java_home) = current_java_home() {
            input.options.insert("java_home".to_string(), java_home);
        }
        input.options.insert(
            "classpath".to_string(),
            tmp.path().to_string_lossy().into_owned(),
        );
        input
            .options
            .insert("main_class".to_string(), "Tool".to_string());
        input.options.insert(
            "args".to_string(),
            format!("{} native-javaexec", output.to_string_lossy()),
        );

        let result = executor.execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        assert_eq!(std::fs::read_to_string(output).unwrap(), "native-javaexec");
    }
}
