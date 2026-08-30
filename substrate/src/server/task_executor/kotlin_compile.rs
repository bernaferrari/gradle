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

    fn validate_supported_contract(input: &TaskInput) -> Result<(), String> {
        if let Some(source) = input
            .source_files
            .iter()
            .find(|source| source.extension().is_some_and(|extension| extension == "kts"))
        {
            return Err(format!(
                "unsupported Kotlin scripting source '{}': the native KotlinCompile contract only supports .kt JVM sources",
                source.display()
            ));
        }

        for (option, capability) in [
            ("plugin_classpath", "compiler plugins"),
            ("compiler_plugin_classpath", "compiler plugins"),
            ("compiler_plugin_options", "compiler plugin options"),
            ("friend_paths", "friend paths"),
            ("friend_modules", "friend modules"),
            ("script_definitions", "Kotlin scripting"),
            ("script_extensions", "Kotlin scripting"),
            ("multiplatform_structure", "multiplatform compilation"),
            ("common_sources", "multiplatform common sources"),
        ] {
            if input
                .options
                .get(option)
                .is_some_and(|value| option_value_is_configured(value))
            {
                return Err(format!(
                    "unsupported KotlinCompile option '{option}': capability '{capability}' is outside the native host-kotlinc contract"
                ));
            }
        }

        const SUPPORTED_OPTIONS: &[&str] = &[
            "classpath",
            "compiler_args",
            "compiler_args_json",
            "java_home",
            "jvm_target",
            "kotlin_compiler_identity",
            "kotlin_compiler_version",
            "kotlin_home",
            "kotlinc",
            "module_name",
            "target_version",
            "working_dir",
        ];
        if let Some((option, _)) = input.options.iter().find(|(option, value)| {
            option_value_is_configured(value) && !SUPPORTED_OPTIONS.contains(&option.as_str())
        }) {
            return Err(format!(
                "unsupported KotlinCompile option '{option}': it is outside the native host-kotlinc contract"
            ));
        }

        if let Some(arguments_json) = configured_option(input, "compiler_args_json") {
            serde_json::from_str::<Vec<String>>(arguments_json).map_err(|error| {
                format!(
                    "invalid Kotlin compiler_args_json contract: expected a JSON string array: {error}"
                )
            })?;
        }

        for argument in option_string_list(
            &input.options,
            "compiler_args_json",
            "compiler_args",
        ) {
            if let Some(capability) = unsupported_compiler_argument(&argument) {
                return Err(format!(
                    "unsupported Kotlin compiler argument '{argument}': capability '{capability}' is outside the native host-kotlinc contract"
                ));
            }
        }

        Ok(())
    }

    async fn validate_compiler_identity(
        kotlinc: &Path,
        input: &TaskInput,
    ) -> Result<(), String> {
        let expected_identity = configured_option(input, "kotlin_compiler_identity");
        let expected_version = configured_option(input, "kotlin_compiler_version");
        if expected_identity.is_none() && expected_version.is_none() {
            return Ok(());
        }

        let mut environment = HashMap::new();
        if let Some(java_home) = configured_option(input, "java_home") {
            environment.insert("JAVA_HOME".to_string(), java_home.to_string());
        }
        let output = run_to_output(
            &ProcessLaunchSpec::new(kotlinc)
                .args(["-version"])
                .environment(environment),
        )
        .await
        .map_err(|error| {
            format!(
                "failed to verify captured Kotlin compiler identity using '{}': {error}",
                kotlinc.display()
            )
        })?;
        if output.exit_code != 0 {
            return Err(format!(
                "failed to verify captured Kotlin compiler identity using '{}': kotlinc -version exited with code {}",
                kotlinc.display(),
                output.exit_code
            ));
        }

        let report = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        validate_compiler_version_report(expected_identity, expected_version, &report)
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

        if let Err(error) = Self::validate_supported_contract(input) {
            result.error_message = error;
            result.duration_ms = start.elapsed().as_millis() as u64;
            return result;
        }

        if input.source_files.is_empty() {
            result.success = true;
            result.duration_ms = start.elapsed().as_millis() as u64;
            return result;
        }

        let kotlinc = Self::find_kotlinc(input);
        if let Err(error) = Self::validate_compiler_identity(&kotlinc, input).await {
            result.error_message = error;
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

fn configured_option<'a>(input: &'a TaskInput, name: &str) -> Option<&'a str> {
    input
        .options
        .get(name)
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn option_value_is_configured(value: &str) -> bool {
    !matches!(value.trim(), "" | "[]" | "{}" | "false" | "null")
}

fn unsupported_compiler_argument(argument: &str) -> Option<&'static str> {
    let argument = argument.trim();
    let option_end = argument
        .find(|character: char| character == '=' || character.is_ascii_whitespace())
        .unwrap_or(argument.len());
    let option = &argument[..option_end];

    if option == "-P"
        || argument.starts_with("-Pplugin:")
        || argument.starts_with("plugin:")
        || option == "-Xplugin"
        || option.starts_with("-Xcompiler-plugin")
    {
        return Some("Kotlin compiler plugins and plugin options");
    }
    if option == "-Xfriend-paths" || option == "-Xfriend-modules" {
        return Some("Kotlin friend paths/modules");
    }
    if matches!(
        option,
        "-script" | "-script-templates" | "-expression" | "-e"
    ) || option.starts_with("-Xscript-")
        || option == "-Xallow-any-scripts-in-source-roots"
        || option == "-Xdisable-standard-script"
        || option == "-Xdefault-script-extension"
    {
        return Some("Kotlin scripting");
    }
    if option == "-Xmulti-platform"
        || option == "-Xcommon-sources"
        || option == "-Xexpect-actual-classes"
        || option.starts_with("-Xfragment")
        || option.starts_with("-Xmetadata-klib")
        || option.starts_with("-Xklib")
    {
        return Some("Kotlin multiplatform compilation");
    }

    None
}

fn reported_compiler_identity_and_version(report: &str) -> Option<(String, String)> {
    let tokens = report
        .split_whitespace()
        .map(|token| {
            token.trim_matches(|character: char| {
                !character.is_ascii_alphanumeric()
                    && !matches!(character, '.' | '-' | '+' | '_')
            })
        })
        .collect::<Vec<_>>();
    for pair in tokens.windows(2) {
        if pair[0].eq_ignore_ascii_case("kotlinc-jvm")
            && pair[1]
                .chars()
                .next()
                .is_some_and(|character| character.is_ascii_digit())
        {
            return Some((pair[0].to_ascii_lowercase(), pair[1].to_string()));
        }
    }
    None
}

fn validate_compiler_version_report(
    expected_identity: Option<&str>,
    expected_version: Option<&str>,
    report: &str,
) -> Result<(), String> {
    let (actual_identity, actual_version) = reported_compiler_identity_and_version(report)
        .ok_or_else(|| {
            "kotlinc -version did not report a recognizable kotlinc-jvm identity and version"
                .to_string()
        })?;

    if let Some(expected) = expected_identity {
        if !actual_identity.eq_ignore_ascii_case(expected.trim()) {
            return Err(format!(
                "Kotlin compiler identity mismatch: captured '{}', host reported '{}'",
                expected.trim(),
                actual_identity
            ));
        }
    }
    if let Some(expected) = expected_version {
        if actual_version != expected.trim() {
            return Err(format!(
                "Kotlin compiler version mismatch: captured '{}', host reported '{}'",
                expected.trim(),
                actual_version
            ));
        }
    }
    Ok(())
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

    #[test]
    fn rejects_compiler_plugin_friend_scripting_and_multiplatform_arguments() {
        for argument in [
            "-Xplugin=/tmp/compiler-plugin.jar",
            "-P",
            "-P plugin:sample:enabled=true",
            "plugin:sample:enabled=true",
            "-Xfriend-paths=/tmp/main-classes",
            "-script-templates=sample.Template",
            "-Xmulti-platform",
            "-Xfragment-sources=commonMain:Common.kt",
        ] {
            let mut input = TaskInput::new("KotlinCompile");
            input.options.insert(
                "compiler_args_json".to_string(),
                serde_json::json!([argument]).to_string(),
            );

            let error = KotlinCompileExecutor::validate_supported_contract(&input).unwrap_err();

            assert!(error.contains(argument));
            assert!(error.contains("outside the native host-kotlinc contract"));
        }
    }

    #[test]
    fn rejects_kotlin_script_sources() {
        let mut input = TaskInput::new("KotlinCompile");
        input.source_files = vec![PathBuf::from("src/main/kotlin/BuildLogic.kts")];

        let error = KotlinCompileExecutor::validate_supported_contract(&input).unwrap_err();

        assert!(error.contains("scripting source"));
        assert!(error.contains("BuildLogic.kts"));
    }

    #[test]
    fn rejects_explicit_unsupported_kotlin_option_surface() {
        for option in [
            "plugin_classpath",
            "compiler_plugin_options",
            "friend_paths",
            "script_definitions",
            "multiplatform_structure",
            "common_sources",
            "future_semantic_option",
        ] {
            let mut input = TaskInput::new("KotlinCompile");
            input
                .options
                .insert(option.to_string(), "configured".to_string());

            let error = KotlinCompileExecutor::validate_supported_contract(&input).unwrap_err();

            assert!(error.contains(option));
        }
    }

    #[test]
    fn accepts_the_plain_kotlin_jvm_fixture_contract() {
        let mut input = TaskInput::new("KotlinCompile");
        input.source_files = vec![PathBuf::from("src/main/kotlin/Library.kt")];
        input.options.insert(
            "compiler_args_json".to_string(),
            serde_json::json!(["-Xjsr305=strict"]).to_string(),
        );

        assert!(KotlinCompileExecutor::validate_supported_contract(&input).is_ok());
    }

    #[test]
    fn rejects_malformed_exact_compiler_argument_contract() {
        let mut input = TaskInput::new("KotlinCompile");
        input.options.insert(
            "compiler_args_json".to_string(),
            "not-json".to_string(),
        );

        let error = KotlinCompileExecutor::validate_supported_contract(&input).unwrap_err();

        assert!(error.contains("invalid Kotlin compiler_args_json contract"));
    }

    #[test]
    fn validates_captured_compiler_identity_and_exact_version() {
        let report = "info: kotlinc-jvm 2.4.0 (JRE 17.0.12+7)";

        assert!(validate_compiler_version_report(
            Some("kotlinc-jvm"),
            Some("2.4.0"),
            report
        )
        .is_ok());

        let error = validate_compiler_version_report(
            Some("kotlinc-jvm"),
            Some("2.3.21"),
            report,
        )
        .unwrap_err();
        assert!(error.contains("version mismatch"));
        assert!(error.contains("2.3.21"));
        assert!(error.contains("2.4.0"));
    }
}
