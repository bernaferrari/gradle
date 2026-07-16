use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::server::task_executor::{option_string_list, TaskExecutor, TaskInput, TaskResult};

use super::process_launch::{run_to_output, ProcessLaunchSpec};

/// Result of a Kotlin JVM compilation via `kotlinc`.
#[derive(Debug, Clone, Default)]
pub struct KotlinCompileResult {
    pub success: bool,
    pub exit_code: i32,
    pub output_files: Vec<PathBuf>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub duration_ms: u64,
    pub source_files_compiled: u64,
    pub error_message: String,
}

/// Orchestrates `kotlinc` as a child process for admitted KotlinCompile tasks.
pub struct KotlinCompileExecutor;

impl Default for KotlinCompileExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl KotlinCompileExecutor {
    pub fn new() -> Self {
        Self
    }

    /// Resolve kotlinc: explicit option, KOTLIN_HOME, then PATH.
    pub fn find_kotlinc(input: &TaskInput) -> PathBuf {
        if let Some(path) = input.options.get("kotlinc").filter(|p| !p.is_empty()) {
            return PathBuf::from(path);
        }
        if let Some(home) = input
            .options
            .get("kotlin_home")
            .cloned()
            .or_else(|| std::env::var("KOTLIN_HOME").ok())
        {
            let candidate = if cfg!(target_os = "windows") {
                PathBuf::from(format!(r"{home}\bin\kotlinc.bat"))
            } else {
                PathBuf::from(format!("{home}/bin/kotlinc"))
            };
            if candidate.exists() {
                return candidate;
            }
        }
        PathBuf::from("kotlinc")
    }

    fn build_process_spec(&self, kotlinc: &Path, input: &TaskInput) -> ProcessLaunchSpec {
        let mut args = Vec::new();

        if let Some(classpath) = input.options.get("classpath") {
            if !classpath.is_empty() {
                args.push("-cp".to_string());
                args.push(classpath.clone());
            }
        }

        if !input.target_dir.as_os_str().is_empty() {
            args.push("-d".to_string());
            args.push(input.target_dir.to_string_lossy().to_string());
        }

        let jvm_target = input
            .options
            .get("jvm_target")
            .cloned()
            .or_else(|| input.options.get("target_version").cloned())
            .unwrap_or_else(|| "17".to_string());
        if !jvm_target.is_empty() {
            args.push("-jvm-target".to_string());
            args.push(jvm_target);
        }

        if let Some(module_name) = input.options.get("module_name").filter(|value| !value.is_empty()) {
            args.push("-module-name".to_string());
            args.push(module_name.clone());
        }

        for arg in option_string_list(&input.options, "compiler_args_json", "compiler_args") {
            let trimmed = arg.trim();
            // Capture often serializes empty freeCompilerArgs as the literal "[]".
            if trimmed.is_empty() || trimmed == "[]" || trimmed == "null" {
                continue;
            }
            args.push(arg);
        }

        for source in &input.source_files {
            args.push(source.to_string_lossy().to_string());
        }

        let mut environment = HashMap::new();
        if let Some(java_home) = input.options.get("java_home") {
            if !java_home.is_empty() {
                environment.insert("JAVA_HOME".to_string(), java_home.clone());
            }
        }

        let mut spec = ProcessLaunchSpec::new(kotlinc)
            .args(args)
            .environment(environment);
        if let Some(working_dir) = input
            .options
            .get("working_dir")
            .filter(|path| !path.is_empty())
        {
            spec = spec.working_dir(working_dir);
        }
        spec
    }

    pub fn collect_output_files(output_dir: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        if !output_dir.exists() {
            return files;
        }
        collect_class_files_recursive(output_dir, &mut files);
        files
    }

    pub async fn compile(&self, input: &TaskInput) -> KotlinCompileResult {
        let start = std::time::Instant::now();
        let mut result = KotlinCompileResult::default();

        if input.source_files.is_empty() {
            result.success = true;
            result.duration_ms = start.elapsed().as_millis() as u64;
            return result;
        }

        if !input.target_dir.as_os_str().is_empty() {
            if let Err(error) = tokio::fs::create_dir_all(&input.target_dir).await {
                result.error_message = format!(
                    "failed to create Kotlin output directory {}: {error}",
                    input.target_dir.display()
                );
                result.duration_ms = start.elapsed().as_millis() as u64;
                return result;
            }
        }

        let kotlinc = Self::find_kotlinc(input);
        let spec = self.build_process_spec(&kotlinc, input);

        tracing::debug!(
            kotlinc = %kotlinc.display(),
            sources = input.source_files.len(),
            "Starting kotlinc compilation"
        );

        match run_to_output(&spec).await {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = format!("{stdout}\n{stderr}");
                result.exit_code = output.exit_code;
                result.success = output.exit_code == 0;
                result.source_files_compiled = input.source_files.len() as u64;

                for line in combined.lines() {
                    let lower = line.to_ascii_lowercase();
                    if lower.contains("error:") {
                        result.errors.push(line.to_string());
                    } else if lower.contains("warning:") {
                        result.warnings.push(line.to_string());
                    }
                }

                if result.success {
                    if !input.target_dir.as_os_str().is_empty() {
                        result.output_files = Self::collect_output_files(&input.target_dir);
                    }
                } else {
                    let excerpt = combined
                        .lines()
                        .filter(|line| !line.trim().is_empty())
                        .take(40)
                        .collect::<Vec<_>>()
                        .join("\n");
                    result.error_message = format!(
                        "kotlinc failed with exit code {}: {} errors{}{excerpt}",
                        result.exit_code,
                        result.errors.len(),
                        if excerpt.is_empty() { "" } else { "\n" },
                    );
                }
            }
            Err(error) => {
                result.error_message =
                    format!("failed to launch kotlinc ({}): {error}", kotlinc.display());
            }
        }

        result.duration_ms = start.elapsed().as_millis() as u64;
        result
    }
}

fn collect_class_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_class_files_recursive(&path, files);
            } else if path.extension().is_some_and(|ext| ext == "class") {
                files.push(path);
            }
        }
    }
}

#[tonic::async_trait]
impl TaskExecutor for KotlinCompileExecutor {
    fn task_type(&self) -> &str {
        "KotlinCompile"
    }

    async fn execute(&self, input: &TaskInput) -> TaskResult {
        let compile_result = self.compile(input).await;
        TaskResult {
            success: compile_result.success,
            output_files: compile_result.output_files,
            duration_ms: compile_result.duration_ms,
            files_processed: compile_result.source_files_compiled,
            bytes_processed: 0,
            error_message: compile_result.error_message,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_spec_includes_sources_classpath_and_jvm_target() {
        let mut input = TaskInput::new("KotlinCompile");
        input.source_files = vec![PathBuf::from("src/main/kotlin/Lib.kt")];
        input.target_dir = PathBuf::from("build/classes/kotlin/main");
        input
            .options
            .insert("classpath".to_string(), "lib.jar".to_string());
        input
            .options
            .insert("jvm_target".to_string(), "17".to_string());
        let executor = KotlinCompileExecutor::new();
        let spec = executor.build_process_spec(Path::new("kotlinc"), &input);
        let args = spec.args;
        assert!(args.iter().any(|a| a == "-cp"));
        assert!(args.iter().any(|a| a == "lib.jar"));
        assert!(args.iter().any(|a| a == "-d"));
        assert!(args.iter().any(|a| a == "-jvm-target"));
        assert!(args.iter().any(|a| a == "17"));
        assert!(args.iter().any(|a| a.ends_with("Lib.kt")));
    }

    #[tokio::test]
    async fn empty_sources_succeeds_without_launching_compiler() {
        let input = TaskInput::new("KotlinCompile");
        let result = KotlinCompileExecutor::new().compile(&input).await;
        assert!(result.success);
        assert_eq!(result.source_files_compiled, 0);
    }
}
