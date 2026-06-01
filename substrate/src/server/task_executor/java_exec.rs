use std::path::{Path, PathBuf};

use crate::server::task_executor::{
    option_string_list, option_string_map, TaskExecutor, TaskInput, TaskResult,
};

use super::process_launch::{run_to_output, ProcessLaunchSpec};

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

    fn system_property_args(input: &TaskInput) -> Vec<String> {
        option_string_map(
            &input.options,
            "system_properties_json",
            "system_properties",
        )
        .into_iter()
        .map(|(key, value)| format!("-D{}={}", key, value))
        .collect()
    }

    fn max_heap_arg(max_heap_size: Option<&str>) -> Option<String> {
        let value = max_heap_size
            .map(str::trim)
            .filter(|value| !value.is_empty())?;
        if value.starts_with("-Xmx") {
            Some(value.to_string())
        } else {
            Some(format!("-Xmx{}", value))
        }
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

        let mut args = Vec::new();
        if let Some(max_heap) =
            Self::max_heap_arg(input.options.get("max_heap_size").map(String::as_str))
        {
            args.push(max_heap);
        }
        args.extend(option_string_list(
            &input.options,
            "jvm_args_json",
            "jvm_args",
        ));
        args.extend(Self::system_property_args(input));
        args.push("-cp".to_string());
        args.push(classpath.to_string());
        args.push(main_class.to_string());
        args.extend(option_string_list(&input.options, "args_json", "args"));
        let mut spec = ProcessLaunchSpec::new(&java)
            .args(args)
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
                    "JavaExec task '{}' failed with exit code {}",
                    main_class, output.exit_code
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
            "args_json".to_string(),
            serde_json::to_string(&vec![
                output.to_string_lossy().to_string(),
                "native javaexec with spaces".to_string(),
            ])
            .unwrap(),
        );

        let result = executor.execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        assert_eq!(
            std::fs::read_to_string(output).unwrap(),
            "native javaexec with spaces"
        );
    }

    #[test]
    fn test_java_exec_system_property_args() {
        let mut input = TaskInput::new("JavaExec");
        input.options.insert(
            "system_properties".to_string(),
            "key1=val1,key2=val2".to_string(),
        );
        let mut args = JavaExecTaskExecutor::system_property_args(&input);
        args.sort();
        assert_eq!(
            args,
            vec!["-Dkey1=val1".to_string(), "-Dkey2=val2".to_string()]
        );
        input.options.insert(
            "system_properties".to_string(),
            "empty,nope,key=value".to_string(),
        );
        let mut args = JavaExecTaskExecutor::system_property_args(&input);
        args.sort();
        assert_eq!(args, vec!["-Dkey=value".to_string()]);
        input
            .options
            .insert("system_properties".to_string(), " ".to_string());
        assert!(JavaExecTaskExecutor::system_property_args(&input).is_empty());
    }

    #[test]
    fn test_java_exec_system_property_args_prefer_json_contract() {
        let mut input = TaskInput::new("JavaExec");
        input.options.insert(
            "system_properties".to_string(),
            "BROKEN=legacy,value".to_string(),
        );
        input.options.insert(
            "system_properties_json".to_string(),
            serde_json::json!({
                "complex.prop": "value,with=punctuation",
                "empty.prop": ""
            })
            .to_string(),
        );

        let mut args = JavaExecTaskExecutor::system_property_args(&input);
        args.sort();

        assert_eq!(
            args,
            vec![
                "-Dcomplex.prop=value,with=punctuation".to_string(),
                "-Dempty.prop=".to_string()
            ]
        );
    }

    #[test]
    fn test_java_exec_max_heap_arg_normalizes_gradle_contract() {
        assert_eq!(
            JavaExecTaskExecutor::max_heap_arg(Some("256m")),
            Some("-Xmx256m".to_string())
        );
        assert_eq!(
            JavaExecTaskExecutor::max_heap_arg(Some("-Xmx1g")),
            Some("-Xmx1g".to_string())
        );
        assert_eq!(JavaExecTaskExecutor::max_heap_arg(Some(" ")), None);
    }

    #[tokio::test]
    async fn test_java_exec_passes_system_properties() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("Tool.java");
        let output = tmp.path().join("result.txt");
        let mut file = std::fs::File::create(&source).unwrap();
        writeln!(
            file,
            "public class Tool {{ public static void main(String[] args) throws Exception {{ java.nio.file.Files.writeString(java.nio.file.Path.of(args[0]), System.getProperty(args[1], \"missing\")); }} }}"
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
            "args_json".to_string(),
            serde_json::to_string(&vec![
                output.to_string_lossy().to_string(),
                "native.prop".to_string(),
            ])
            .unwrap(),
        );
        input.options.insert(
            "system_properties".to_string(),
            "native.prop=from-rust".to_string(),
        );

        let result = executor.execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        assert_eq!(std::fs::read_to_string(output).unwrap(), "from-rust");
    }

    #[tokio::test]
    async fn test_java_exec_passes_environment_json() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("Tool.java");
        let output = tmp.path().join("result.txt");
        let mut file = std::fs::File::create(&source).unwrap();
        writeln!(
            file,
            "public class Tool {{ public static void main(String[] args) throws Exception {{ java.nio.file.Files.writeString(java.nio.file.Path.of(args[0]), System.getenv(args[1])); }} }}"
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
            "args_json".to_string(),
            serde_json::to_string(&vec![
                output.to_string_lossy().to_string(),
                "NATIVE_JAVA_EXEC_ENV".to_string(),
            ])
            .unwrap(),
        );
        input.options.insert(
            "environment_json".to_string(),
            serde_json::json!({"NATIVE_JAVA_EXEC_ENV": "from-rust-env"}).to_string(),
        );

        let result = executor.execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        assert_eq!(std::fs::read_to_string(output).unwrap(), "from-rust-env");
    }
}
