use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::server::task_executor::{TaskExecutor, TaskInput, TaskResult};

/// Executes Gradle Javadoc from a conservative source/output contract.
pub struct JavadocTaskExecutor;

impl Default for JavadocTaskExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl JavadocTaskExecutor {
    pub fn new() -> Self {
        Self
    }

    fn javadoc_executable(java_home: Option<&str>) -> PathBuf {
        match java_home.map(str::trim).filter(|home| !home.is_empty()) {
            Some(home) => Path::new(home).join("bin").join(javadoc_binary_name()),
            None => PathBuf::from(javadoc_binary_name()),
        }
    }
}

#[cfg(windows)]
fn javadoc_binary_name() -> &'static str {
    "javadoc.exe"
}

#[cfg(not(windows))]
fn javadoc_binary_name() -> &'static str {
    "javadoc"
}

#[tonic::async_trait]
impl TaskExecutor for JavadocTaskExecutor {
    fn task_type(&self) -> &str {
        "Javadoc"
    }

    async fn execute(&self, input: &TaskInput) -> TaskResult {
        let start = std::time::Instant::now();
        let mut result = TaskResult::default();

        if input.source_files.is_empty() {
            result.success = false;
            result.error_message = "Javadoc task is missing source files".to_string();
            return result;
        }
        let target_dir = input
            .options
            .get("destination_dir")
            .map(PathBuf::from)
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| input.target_dir.clone());
        if target_dir.as_os_str().is_empty() {
            result.success = false;
            result.error_message = "Javadoc task is missing destination directory".to_string();
            return result;
        }

        if let Err(error) = std::fs::create_dir_all(&target_dir) {
            result.success = false;
            result.error_message = format!(
                "Failed to create Javadoc destination '{}': {}",
                target_dir.display(),
                error
            );
            return result;
        }

        let javadoc = Self::javadoc_executable(input.options.get("java_home").map(String::as_str));
        let mut command = Command::new(&javadoc);
        command.arg("-d").arg(&target_dir);
        command.arg("-quiet");
        if input
            .options
            .get("no_timestamp")
            .map(|value| value == "true")
            .unwrap_or(false)
        {
            command.arg("-notimestamp");
        }
        if let Some(encoding) = input
            .options
            .get("encoding")
            .filter(|value| !value.is_empty())
        {
            command.arg("-encoding").arg(encoding);
        }
        if let Some(title) = input.options.get("title").filter(|value| !value.is_empty()) {
            command.arg("-doctitle").arg(title);
            command.arg("-windowtitle").arg(title);
        }
        if let Some(classpath) = input
            .options
            .get("classpath")
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            command.arg("-classpath").arg(classpath);
        }
        for source in &input.source_files {
            command.arg(source);
        }

        match command.output().await {
            Ok(output) if output.status.success() => {
                result.output_files.push(target_dir);
                result.files_processed = input.source_files.len() as u64;
                result.bytes_processed = (output.stdout.len() + output.stderr.len()) as u64;
            }
            Ok(output) => {
                result.success = false;
                result.error_message = format!(
                    "Javadoc task failed with exit code {}: {}",
                    output.status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&output.stderr).trim()
                );
            }
            Err(error) => {
                result.success = false;
                result.error_message =
                    format!("Failed to execute '{}': {}", javadoc.display(), error);
            }
        }

        result.duration_ms = start.elapsed().as_millis() as u64;
        result
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn current_java_home() -> Option<String> {
        std::env::var("JAVA_HOME").ok().or_else(|| {
            let java_home = std::env::var("java_home").ok();
            java_home.filter(|value| !value.trim().is_empty())
        })
    }

    #[tokio::test]
    async fn test_javadoc_missing_sources_fails() {
        let executor = JavadocTaskExecutor::new();
        let mut input = TaskInput::new("Javadoc");
        input.target_dir = PathBuf::from("/tmp/docs");

        let result = executor.execute(&input).await;

        assert!(!result.success);
        assert!(result.error_message.contains("missing source files"));
    }

    #[tokio::test]
    async fn test_javadoc_generates_docs() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("Tool.java");
        let docs = tmp.path().join("docs");
        let mut file = std::fs::File::create(&source).unwrap();
        writeln!(
            file,
            "/** Tool docs. */ public class Tool {{ /** Runs. */ public void run() {{ }} }}"
        )
        .unwrap();

        let executor = JavadocTaskExecutor::new();
        let mut input = TaskInput::new("Javadoc");
        input.source_files.push(source);
        input.target_dir = docs.clone();
        if let Some(java_home) = current_java_home() {
            input.options.insert("java_home".to_string(), java_home);
        }
        input
            .options
            .insert("no_timestamp".to_string(), "true".to_string());

        let result = executor.execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        assert!(docs.join("Tool.html").exists());
    }
}
