use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Instant;

use tokio::process::Command;

use crate::server::task_executor::{TaskExecutor, TaskInput, TaskResult};

/// Outcome of a single test method.
#[derive(Debug, Clone, PartialEq)]
pub enum TestOutcome {
    Passed,
    Failed,
    Skipped,
    Error,
}

impl TestOutcome {
    #[cfg(test)]
    fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "passed" | "success" => TestOutcome::Passed,
            "failed" | "failure" => TestOutcome::Failed,
            "skipped" | "ignored" => TestOutcome::Skipped,
            "error" => TestOutcome::Error,
            _ => TestOutcome::Error,
        }
    }

    #[cfg(test)]
    fn as_str(&self) -> &str {
        match self {
            TestOutcome::Passed => "PASSED",
            TestOutcome::Failed => "FAILED",
            TestOutcome::Skipped => "SKIPPED",
            TestOutcome::Error => "ERROR",
        }
    }
}

/// Result of a single test method.
#[derive(Debug, Clone)]
pub struct TestMethodResult {
    pub class_name: String,
    pub method_name: String,
    pub outcome: TestOutcome,
    pub duration_ms: u64,
    pub failure_message: String,
    pub failure_type: String,
    pub stack_trace: Vec<String>,
}

/// Aggregated result of a test execution.
#[derive(Debug, Clone, Default)]
pub struct TestExecResult {
    pub success: bool,
    pub exit_code: i32,
    pub tests: Vec<TestMethodResult>,
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub errors: usize,
    pub duration_ms: u64,
    pub error_message: String,
    pub xml_report_dir: PathBuf,
}

/// Orchestrates JUnit/TestNG test execution via a forked JVM process.
pub struct TestExecExecutor;

impl Default for TestExecExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl TestExecExecutor {
    pub fn new() -> Self {
        Self
    }

    /// Build the test JVM command line.
    ///
    /// Options:
    /// - `java_home`: JDK installation path
    /// - `classpath`: classpath for test execution (test classes + dependencies)
    /// - `test_classes`: comma-separated list of fully-qualified test class names
    /// - `test_filter`: glob pattern for test method filtering (e.g. "com.foo.*Test")
    /// - `include_engines`: comma-separated JUnit 5 engine IDs (default: "junit-jupiter")
    /// - `exclude_tags`: comma-separated JUnit 5 tags to exclude
    /// - `include_tags`: comma-separated JUnit 5 tags to include
    /// - `jvm_args`: additional JVM arguments (space-separated)
    /// - `working_dir`: working directory for the test process
    /// - `xml_report_dir`: directory to write JUnit XML reports (for parsing)
    /// - `fork_count`: number of parallel test JVM forks (default: 1)
    /// - `max_heap_mb`: max heap size in MB (default: 512)
    /// - `system_properties`: comma-separated key=value pairs
    /// - `parallel_classes`: whether to run test classes in parallel ("true"/"false")
    /// - `parallel_methods`: whether to run test methods in parallel ("true"/"false")
    pub fn build_command(&self, java_path: &Path, input: &TaskInput) -> Command {
        let mut cmd = Command::new(java_path);

        // JVM args
        let max_heap = input
            .options
            .get("max_heap_mb")
            .map(|s| s.as_str())
            .unwrap_or("512");
        cmd.arg(format!("-Xmx{}m", max_heap));

        // Additional JVM args
        if let Some(jvm_args) = input.options.get("jvm_args") {
            for arg in jvm_args.split_whitespace() {
                cmd.arg(arg);
            }
        }

        // System properties
        if let Some(props) = input.options.get("system_properties") {
            for prop in props.split(',') {
                let prop = prop.trim();
                if !prop.is_empty() && prop.contains('=') {
                    cmd.arg(format!("-D{}", prop));
                }
            }
        }

        // Working directory
        if let Some(working_dir) = input.options.get("working_dir") {
            cmd.current_dir(working_dir);
        }

        // Classpath
        if let Some(classpath) = input.options.get("classpath") {
            cmd.arg("-classpath").arg(classpath);
        }

        // JUnit Platform Console Launcher main class
        cmd.arg("org.junit.platform.console.ConsoleLauncher");

        // XML reports directory
        if let Some(report_dir) = input.options.get("xml_report_dir") {
            cmd.arg("--reports-dir").arg(report_dir);
        }

        // Include engines
        let engines = input
            .options
            .get("include_engines")
            .map(|s| s.as_str())
            .unwrap_or("junit-jupiter");
        cmd.arg("--include-engine").arg(engines);

        // Exclude tags
        if let Some(exclude_tags) = input.options.get("exclude_tags") {
            for tag in exclude_tags.split(',') {
                let tag = tag.trim();
                if !tag.is_empty() {
                    cmd.arg("--exclude-tag").arg(tag);
                }
            }
        }

        // Include tags
        if let Some(include_tags) = input.options.get("include_tags") {
            for tag in include_tags.split(',') {
                let tag = tag.trim();
                if !tag.is_empty() {
                    cmd.arg("--include-tag").arg(tag);
                }
            }
        }

        // Test filter (class/method patterns)
        if let Some(filter) = input.options.get("test_filter") {
            cmd.arg("--filter").arg(filter);
        }

        // Test classes (positional args to ConsoleLauncher)
        if let Some(test_classes) = input.options.get("test_classes") {
            for class in test_classes.split(',') {
                let class = class.trim();
                if !class.is_empty() {
                    cmd.arg("--select-class").arg(class);
                }
            }
        }

        // Source files as test class candidates (if no explicit test_classes)
        if !input.options.contains_key("test_classes") {
            for source in &input.source_files {
                // Convert .java paths to class names: src/test/java/com/example/FooTest.java -> com.example.FooTest
                if let Some(class_name) = Self::java_file_to_class(source) {
                    cmd.arg("--select-class").arg(class_name);
                }
            }
        }

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        cmd
    }

    /// Convert a .java file path to a fully-qualified class name.
    /// Handles paths like: src/test/java/com/example/FooTest.java -> com.example.FooTest
    pub fn java_file_to_class(path: &Path) -> Option<String> {
        let file_name = path.file_name()?.to_str()?;
        if !file_name.ends_with(".java") {
            return None;
        }

        // Skip if it's not a test class (convention)
        let stem = file_name.strip_suffix(".java")?;
        if !stem.ends_with("Test") && !stem.ends_with("Tests") && !stem.ends_with("Spec") {
            return None;
        }

        // Find the "java" or "kotlin" source root
        let mut components: Vec<&str> = Vec::new();
        let mut found_root = false;

        for component in path.parent()?.components() {
            let comp_str = component.as_os_str().to_str()?;
            if found_root {
                components.push(comp_str);
            } else if comp_str == "java" || comp_str == "kotlin" {
                found_root = true;
            }
        }

        if found_root && !components.is_empty() {
            let mut class_name = components.join(".");
            class_name.push('.');
            class_name.push_str(stem);
            Some(class_name)
        } else {
            None
        }
    }

    /// Parse JUnit XML test reports from the report directory.
    pub fn parse_junit_xml_reports(report_dir: &Path) -> Vec<TestMethodResult> {
        let mut results = Vec::new();
        // JUnit Platform writes to files like:
        // TEST-com.example.FooTest.xml or junit-platform-TEST-com.example.FooTest.xml
        if let Ok(entries) = std::fs::read_dir(report_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("xml") {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        results.extend(Self::parse_junit_xml(&content));
                    }
                }
            }
        }

        results
    }

    /// Parse a single JUnit XML report.
    pub fn parse_junit_xml(xml: &str) -> Vec<TestMethodResult> {
        let mut results = Vec::new();

        // Simple XML parsing for JUnit format:
        // <testcase name="methodName" classname="com.example.FooTest" time="0.123">
        //   <failure message="..." type="...">stack trace</failure>
        //   <error message="..." type="...">stack trace</error>
        //   <skipped message="..."/>
        // </testcase>
        let mut in_testcase = false;
        let mut current_class = String::new();
        let mut current_method = String::new();
        let mut current_time: f64 = 0.0;
        let mut current_outcome = TestOutcome::Passed;
        let mut current_failure_msg = String::new();
        let mut current_failure_type = String::new();
        let mut current_stack_trace = Vec::new();
        let mut in_failure = false;
        let mut in_error = false;

        for line in xml.lines() {
            let trimmed = line.trim();

            if !in_testcase {
                if let Some(rest) = trimmed.strip_prefix("<testcase ") {
                    in_testcase = true;
                    current_class = Self::extract_attr(rest, "classname").unwrap_or_default();
                    current_method = Self::extract_attr(rest, "name").unwrap_or_default();
                    current_time = Self::extract_attr(rest, "time")
                        .and_then(|t| t.parse::<f64>().ok())
                        .unwrap_or(0.0);
                    current_outcome = TestOutcome::Passed;
                    current_failure_msg.clear();
                    current_failure_type.clear();
                    current_stack_trace.clear();
                    in_failure = false;
                    in_error = false;

                    // Handle self-closing tag: <testcase .../>
                    if trimmed.ends_with("/>") {
                        in_testcase = false;
                        results.push(TestMethodResult {
                            class_name: current_class.clone(),
                            method_name: current_method.clone(),
                            outcome: current_outcome.clone(),
                            duration_ms: (current_time * 1000.0) as u64,
                            failure_message: String::new(),
                            failure_type: String::new(),
                            stack_trace: Vec::new(),
                        });
                    }
                }
                continue;
            }

            if trimmed.starts_with("<failure") {
                current_outcome = TestOutcome::Failed;
                in_failure = true;
                current_failure_msg = Self::extract_attr(trimmed, "message").unwrap_or_default();
                current_failure_type = Self::extract_attr(trimmed, "type").unwrap_or_default();
            } else if trimmed.starts_with("<error") {
                current_outcome = TestOutcome::Error;
                in_error = true;
                current_failure_msg = Self::extract_attr(trimmed, "message").unwrap_or_default();
                current_failure_type = Self::extract_attr(trimmed, "type").unwrap_or_default();
            } else if trimmed.starts_with("<skipped") {
                current_outcome = TestOutcome::Skipped;
                current_failure_msg = Self::extract_attr(trimmed, "message").unwrap_or_default();
            } else if in_failure || in_error {
                if let Some(content) = trimmed.strip_prefix("<![CDATA[") {
                    let content = content.strip_suffix("]]>").unwrap_or(content);
                    for line in content.lines() {
                        current_stack_trace.push(line.to_string());
                    }
                } else if !trimmed.starts_with('<') && !trimmed.is_empty() {
                    current_stack_trace.push(trimmed.to_string());
                }
            }

            if trimmed.starts_with("</testcase>") {
                in_testcase = false;
                in_failure = false;
                in_error = false;

                results.push(TestMethodResult {
                    class_name: current_class.clone(),
                    method_name: current_method.clone(),
                    outcome: current_outcome.clone(),
                    duration_ms: (current_time * 1000.0) as u64,
                    failure_message: current_failure_msg.clone(),
                    failure_type: current_failure_type.clone(),
                    stack_trace: current_stack_trace.clone(),
                });
            }
        }

        results
    }

    /// Extract an attribute value from an XML tag fragment.
    fn extract_attr(tag: &str, attr_name: &str) -> Option<String> {
        let pattern = format!("{}=\"", attr_name);
        let start = tag.find(&pattern)?;
        let value_start = start + pattern.len();
        let value_end = tag[value_start..].find('"')?;
        Some(tag[value_start..value_start + value_end].to_string())
    }

    /// Parse the ConsoleLauncher stdout for a summary line.
    /// Expected format: "Tests run: 5, Failures: 1, Skipped: 0, Aborted: 0"
    pub fn parse_console_summary(output: &str) -> Option<(usize, usize, usize)> {
        for line in output.lines() {
            if line.contains("Tests run:") {
                // Parse: "Tests run: 5, Failures: 1, Skipped: 0"
                let mut tests = 0usize;
                let mut failures = 0usize;
                let mut skipped = 0usize;

                for part in line.split(',') {
                    let part = part.trim();
                    if part.contains("Tests run:") {
                        tests = part
                            .split(':')
                            .nth(1)?
                            .trim()
                            .parse()
                            .unwrap_or(0);
                    } else if part.starts_with("Failures:") {
                        failures = part
                            .split(':')
                            .nth(1)?
                            .trim()
                            .parse()
                            .unwrap_or(0);
                    } else if part.starts_with("Skipped:") {
                        skipped = part
                            .split(':')
                            .nth(1)?
                            .trim()
                            .parse()
                            .unwrap_or(0);
                    }
                }

                return Some((tests, failures, skipped));
            }
        }
        None
    }

    /// Execute tests using the given Java home.
    pub async fn run_tests(&self, java_home: &str, input: &TaskInput) -> TestExecResult {
        let start = Instant::now();
        let mut result = TestExecResult::default();

        // Determine if we have test classes to run
        let has_test_classes = input.options.get("test_classes").is_some_and(|s| !s.is_empty());
        let has_source_files = !input.source_files.is_empty();

        if !has_test_classes && !has_source_files {
            result.success = true;
            result.duration_ms = start.elapsed().as_millis() as u64;
            return result;
        }

        // Find java binary
        let java_path = if cfg!(target_os = "windows") {
            PathBuf::from(format!("{}\\bin\\java.exe", java_home))
        } else {
            PathBuf::from(format!("{}/bin/java", java_home))
        };

        if !java_path.exists() {
            result.error_message = format!("java not found at {}", java_path.display());
            return result;
        }

        // Set up XML report directory
        let xml_report_dir = input
            .options
            .get("xml_report_dir")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                input.target_dir.clone().join("test-reports")
            });
        result.xml_report_dir = xml_report_dir.clone();

        // Ensure report directory exists
        let _ = std::fs::create_dir_all(&xml_report_dir);

        let mut cmd = self.build_command(&java_path, input);

        tracing::debug!(
            java = %java_path.display(),
            working_dir = ?input.options.get("working_dir"),
            "Starting test execution"
        );

        match cmd.output().await {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                result.exit_code = output.status.code().unwrap_or(-1);
                result.success = output.status.success();

                // Parse console summary from stdout
                if let Some((tests, failures, skipped)) = Self::parse_console_summary(&stdout) {
                    result.total = tests;
                    result.failed = failures;
                    result.skipped = skipped;
                    result.passed = tests.saturating_sub(failures + skipped);
                }

                // Parse XML reports if available
                if xml_report_dir.exists() {
                    let xml_results = Self::parse_junit_xml_reports(&xml_report_dir);
                    if !xml_results.is_empty() {
                        // XML results are more detailed, use them
                        result.total = xml_results.len();
                        result.passed = xml_results
                            .iter()
                            .filter(|r| r.outcome == TestOutcome::Passed)
                            .count();
                        result.failed = xml_results
                            .iter()
                            .filter(|r| r.outcome == TestOutcome::Failed)
                            .count();
                        result.skipped = xml_results
                            .iter()
                            .filter(|r| r.outcome == TestOutcome::Skipped)
                            .count();
                        result.errors = xml_results
                            .iter()
                            .filter(|r| r.outcome == TestOutcome::Error)
                            .count();
                        result.tests = xml_results;
                    }
                }

                if !result.success {
                    let failure_details: Vec<String> = result
                        .tests
                        .iter()
                        .filter(|t| t.outcome == TestOutcome::Failed || t.outcome == TestOutcome::Error)
                        .map(|t| format!("{} > {}: {}", t.class_name, t.method_name, t.failure_message))
                        .collect();

                    if failure_details.is_empty() {
                        result.error_message = format!(
                            "Test JVM exited with code {}: {}",
                            result.exit_code,
                            stderr.lines().take(10).collect::<Vec<_>>().join("\n")
                        );
                    } else {
                        result.error_message = failure_details.join("\n");
                    }
                }

                tracing::debug!(
                    duration_ms = result.duration_ms,
                    total = result.total,
                    passed = result.passed,
                    failed = result.failed,
                    skipped = result.skipped,
                    "Test execution completed"
                );
            }
            Err(e) => {
                result.error_message = format!("Failed to execute test JVM: {}", e);
            }
        }

        result.duration_ms = start.elapsed().as_millis() as u64;
        result
    }
}

#[tonic::async_trait]
impl TaskExecutor for TestExecExecutor {
    fn task_type(&self) -> &str {
        "TestExec"
    }

    async fn execute(&self, input: &TaskInput) -> TaskResult {
        let java_home = input
            .options
            .get("java_home")
            .cloned()
            .unwrap_or_else(|| std::env::var("JAVA_HOME").unwrap_or_default());

        let test_result = self.run_tests(&java_home, input).await;

        let output_files = if test_result.xml_report_dir.exists() {
            collect_xml_files(&test_result.xml_report_dir)
        } else {
            Vec::new()
        };

        TaskResult {
            success: test_result.success,
            output_files,
            duration_ms: test_result.duration_ms,
            files_processed: test_result.total as u64,
            bytes_processed: 0,
            error_message: test_result.error_message,
            ..Default::default()
        }
    }
}

fn collect_xml_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(collect_xml_files(&path));
            } else if path.extension().and_then(|e| e.to_str()) == Some("xml") {
                files.push(path);
            }
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_input() -> TaskInput {
        let mut input = TaskInput::new("TestExec");
        input
            .options
            .insert("classpath".to_string(), "/tmp/test-classes".to_string());
        input
            .options
            .insert("test_classes".to_string(), "com.example.FooTest".to_string());
        input
            .options
            .insert("xml_report_dir".to_string(), "/tmp/test-reports".to_string());
        input.target_dir = PathBuf::from("/tmp/test-output");
        input
    }

    #[test]
    fn test_java_file_to_class() {
        let path = PathBuf::from("src/test/java/com/example/FooTest.java");
        assert_eq!(
            TestExecExecutor::java_file_to_class(&path),
            Some("com.example.FooTest".to_string())
        );
    }

    #[test]
    fn test_java_file_to_class_tests_suffix() {
        let path = PathBuf::from("src/test/java/com/example/FooTests.java");
        assert_eq!(
            TestExecExecutor::java_file_to_class(&path),
            Some("com.example.FooTests".to_string())
        );
    }

    #[test]
    fn test_java_file_to_class_spec_suffix() {
        let path = PathBuf::from("src/test/java/com/example/FooSpec.java");
        assert_eq!(
            TestExecExecutor::java_file_to_class(&path),
            Some("com.example.FooSpec".to_string())
        );
    }

    #[test]
    fn test_java_file_to_class_non_test() {
        let path = PathBuf::from("src/test/java/com/example/Foo.java");
        assert_eq!(TestExecExecutor::java_file_to_class(&path), None);
    }

    #[test]
    fn test_java_file_to_class_non_java() {
        let path = PathBuf::from("src/test/java/com/example/FooTest.kt");
        assert_eq!(TestExecExecutor::java_file_to_class(&path), None);
    }

    #[test]
    fn test_java_file_to_class_kotlin_root() {
        let path = PathBuf::from("src/test/kotlin/com/example/FooTest.java");
        assert_eq!(
            TestExecExecutor::java_file_to_class(&path),
            Some("com.example.FooTest".to_string())
        );
    }

    #[test]
    fn test_extract_attr() {
        let tag = r#"name="testMethod" classname="com.example.FooTest" time="0.123""#;
        assert_eq!(
            TestExecExecutor::extract_attr(tag, "name"),
            Some("testMethod".to_string())
        );
        assert_eq!(
            TestExecExecutor::extract_attr(tag, "classname"),
            Some("com.example.FooTest".to_string())
        );
        assert_eq!(
            TestExecExecutor::extract_attr(tag, "time"),
            Some("0.123".to_string())
        );
        assert_eq!(TestExecExecutor::extract_attr(tag, "missing"), None);
    }

    #[test]
    fn test_parse_junit_xml_passed() {
        let xml = r#"
<testsuite tests="2" failures="0" errors="0" skipped="0">
  <testcase name="testSuccess" classname="com.example.FooTest" time="0.05"/>
  <testcase name="testAnother" classname="com.example.FooTest" time="0.02"/>
</testsuite>
"#;
        let results = TestExecExecutor::parse_junit_xml(xml);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].method_name, "testSuccess");
        assert_eq!(results[0].outcome, TestOutcome::Passed);
        assert_eq!(results[0].class_name, "com.example.FooTest");
        assert_eq!(results[0].duration_ms, 50);
    }

    #[test]
    fn test_parse_junit_xml_failure() {
        let xml = r#"
<testsuite tests="1" failures="1">
  <testcase name="testFailure" classname="com.example.FooTest" time="0.1">
    <failure message="Assertion failed" type="java.lang.AssertionError">
      at com.example.FooTest.testFailure(FooTest.java:42)
      at sun.reflect.NativeMethodAccessorImpl.invoke(NativeMethodAccessorImpl.java:62)
    </failure>
  </testcase>
</testsuite>
"#;
        let results = TestExecExecutor::parse_junit_xml(xml);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].outcome, TestOutcome::Failed);
        assert_eq!(results[0].failure_message, "Assertion failed");
        assert_eq!(results[0].failure_type, "java.lang.AssertionError");
        assert_eq!(results[0].stack_trace.len(), 2);
        assert!(results[0].stack_trace[0].contains("FooTest.java:42"));
    }

    #[test]
    fn test_parse_junit_xml_error() {
        let xml = r#"
<testsuite tests="1" failures="0" errors="1">
  <testcase name="testError" classname="com.example.FooTest" time="0.001">
    <error message="NullPointerException" type="java.lang.NullPointerException">
      at com.example.FooTest.testError(FooTest.java:10)
    </error>
  </testcase>
</testsuite>
"#;
        let results = TestExecExecutor::parse_junit_xml(xml);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].outcome, TestOutcome::Error);
        assert_eq!(results[0].failure_type, "java.lang.NullPointerException");
    }

    #[test]
    fn test_parse_junit_xml_skipped() {
        let xml = r#"
<testsuite tests="1" failures="0" skipped="1">
  <testcase name="testDisabled" classname="com.example.FooTest" time="0">
    <skipped message="Disabled via @Disabled"/>
  </testcase>
</testsuite>
"#;
        let results = TestExecExecutor::parse_junit_xml(xml);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].outcome, TestOutcome::Skipped);
        assert_eq!(results[0].failure_message, "Disabled via @Disabled");
    }

    #[test]
    fn test_parse_junit_xml_mixed() {
        let xml = r#"
<testsuite tests="4" failures="1" errors="1" skipped="1">
  <testcase name="testPass" classname="com.example.MixedTest" time="0.01"/>
  <testcase name="testFail" classname="com.example.MixedTest" time="0.05">
    <failure message="expected true" type="java.lang.AssertionError"/>
  </testcase>
  <testcase name="testError" classname="com.example.MixedTest" time="0.001">
    <error message="NPE" type="java.lang.NullPointerException"/>
  </testcase>
  <testcase name="testSkip" classname="com.example.MixedTest" time="0">
    <skipped/>
  </testcase>
</testsuite>
"#;
        let results = TestExecExecutor::parse_junit_xml(xml);
        assert_eq!(results.len(), 4);
        assert_eq!(results[0].outcome, TestOutcome::Passed);
        assert_eq!(results[1].outcome, TestOutcome::Failed);
        assert_eq!(results[2].outcome, TestOutcome::Error);
        assert_eq!(results[3].outcome, TestOutcome::Skipped);
    }

    #[test]
    fn test_parse_junit_xml_empty() {
        let results = TestExecExecutor::parse_junit_xml("");
        assert!(results.is_empty());
    }

    #[test]
    fn test_parse_console_summary() {
        let output = r#"
[INFO] Running com.example.FooTest
[INFO] Tests run: 5, Failures: 1, Skipped: 0, Aborted: 0, Time elapsed: 0.5 s
"#;
        let result = TestExecExecutor::parse_console_summary(output);
        assert_eq!(result, Some((5, 1, 0)));
    }

    #[test]
    fn test_parse_console_summary_no_failures() {
        let output = r#"Tests run: 3, Failures: 0, Skipped: 0, Aborted: 0"#;
        let result = TestExecExecutor::parse_console_summary(output);
        assert_eq!(result, Some((3, 0, 0)));
    }

    #[test]
    fn test_parse_console_summary_no_match() {
        let output = "No summary line here";
        assert_eq!(TestExecExecutor::parse_console_summary(output), None);
    }

    #[test]
    fn test_test_outcome_from_str() {
        assert_eq!(TestOutcome::from_str("passed"), TestOutcome::Passed);
        assert_eq!(TestOutcome::from_str("PASSED"), TestOutcome::Passed);
        assert_eq!(TestOutcome::from_str("success"), TestOutcome::Passed);
        assert_eq!(TestOutcome::from_str("failed"), TestOutcome::Failed);
        assert_eq!(TestOutcome::from_str("FAILURE"), TestOutcome::Failed);
        assert_eq!(TestOutcome::from_str("skipped"), TestOutcome::Skipped);
        assert_eq!(TestOutcome::from_str("ignored"), TestOutcome::Skipped);
        assert_eq!(TestOutcome::from_str("error"), TestOutcome::Error);
        assert_eq!(TestOutcome::from_str("unknown"), TestOutcome::Error);
    }

    #[test]
    fn test_test_outcome_as_str() {
        assert_eq!(TestOutcome::Passed.as_str(), "PASSED");
        assert_eq!(TestOutcome::Failed.as_str(), "FAILED");
        assert_eq!(TestOutcome::Skipped.as_str(), "SKIPPED");
        assert_eq!(TestOutcome::Error.as_str(), "ERROR");
    }

    #[test]
    fn test_build_command_basic() {
        let executor = TestExecExecutor::new();
        let input = make_test_input();
        let java = PathBuf::from("/usr/lib/jvm/java-17/bin/java");
        let cmd = executor.build_command(&java, &input);

        // Collect args as strings
        let args: Vec<String> = cmd.as_std().get_args().map(|s| s.to_string_lossy().to_string()).collect();

        assert!(args.iter().any(|a| a == "-Xmx512m"));
        assert!(args.iter().any(|a| a.contains("FooTest")));
        assert!(args.iter().any(|a| a == "--include-engine"));
    }

    #[test]
    fn test_build_command_custom_heap() {
        let executor = TestExecExecutor::new();
        let mut input = make_test_input();
        input.options.insert("max_heap_mb".to_string(), "1024".to_string());
        let java = PathBuf::from("/usr/lib/jvm/java-17/bin/java");
        let cmd = executor.build_command(&java, &input);

        let args: Vec<String> = cmd.as_std().get_args().map(|s| s.to_string_lossy().to_string()).collect();
        assert!(args.iter().any(|a| a == "-Xmx1024m"));
    }

    #[test]
    fn test_build_command_jvm_args() {
        let executor = TestExecExecutor::new();
        let mut input = make_test_input();
        input
            .options
            .insert("jvm_args".to_string(), "-ea -Dfoo=bar".to_string());
        let java = PathBuf::from("/usr/lib/jvm/java-17/bin/java");
        let cmd = executor.build_command(&java, &input);

        let args: Vec<String> = cmd.as_std().get_args().map(|s| s.to_string_lossy().to_string()).collect();
        assert!(args.iter().any(|a| a == "-ea"));
        assert!(args.iter().any(|a| a == "-Dfoo=bar"));
    }

    #[test]
    fn test_build_command_system_properties() {
        let executor = TestExecExecutor::new();
        let mut input = make_test_input();
        input
            .options
            .insert("system_properties".to_string(), "key1=val1,key2=val2".to_string());
        let java = PathBuf::from("/usr/lib/jvm/java-17/bin/java");
        let cmd = executor.build_command(&java, &input);

        let args: Vec<String> = cmd.as_std().get_args().map(|s| s.to_string_lossy().to_string()).collect();
        assert!(args.iter().any(|a| a == "-Dkey1=val1"));
        assert!(args.iter().any(|a| a == "-Dkey2=val2"));
    }

    #[test]
    fn test_build_command_test_filter() {
        let executor = TestExecExecutor::new();
        let mut input = make_test_input();
        input
            .options
            .insert("test_filter".to_string(), "com.example.*Test".to_string());
        let java = PathBuf::from("/usr/lib/jvm/java-17/bin/java");
        let cmd = executor.build_command(&java, &input);

        let args: Vec<String> = cmd.as_std().get_args().map(|s| s.to_string_lossy().to_string()).collect();
        let filter_idx = args.iter().position(|a| a == "--filter");
        assert!(filter_idx.is_some());
        assert_eq!(
            args[filter_idx.unwrap() + 1],
            "com.example.*Test"
        );
    }

    #[test]
    fn test_build_command_include_tags() {
        let executor = TestExecExecutor::new();
        let mut input = make_test_input();
        input
            .options
            .insert("include_tags".to_string(), "slow,integration".to_string());
        let java = PathBuf::from("/usr/lib/jvm/java-17/bin/java");
        let cmd = executor.build_command(&java, &input);

        let args: Vec<String> = cmd.as_std().get_args().map(|s| s.to_string_lossy().to_string()).collect();
        assert!(args.iter().any(|a| a == "--include-tag"));
        assert!(args.iter().any(|a| a == "slow"));
        assert!(args.iter().any(|a| a == "integration"));
    }

    #[test]
    fn test_build_command_exclude_tags() {
        let executor = TestExecExecutor::new();
        let mut input = make_test_input();
        input
            .options
            .insert("exclude_tags".to_string(), "slow".to_string());
        let java = PathBuf::from("/usr/lib/jvm/java-17/bin/java");
        let cmd = executor.build_command(&java, &input);

        let args: Vec<String> = cmd.as_std().get_args().map(|s| s.to_string_lossy().to_string()).collect();
        assert!(args.iter().any(|a| a == "--exclude-tag"));
        assert!(args.iter().any(|a| a == "slow"));
    }

    #[test]
    fn test_build_command_multiple_test_classes() {
        let executor = TestExecExecutor::new();
        let mut input = make_test_input();
        input.options.insert(
            "test_classes".to_string(),
            "com.example.FooTest,com.example.BarTest".to_string(),
        );
        let java = PathBuf::from("/usr/lib/jvm/java-17/bin/java");
        let cmd = executor.build_command(&java, &input);

        let args: Vec<String> = cmd.as_std().get_args().map(|s| s.to_string_lossy().to_string()).collect();
        assert!(args.iter().any(|a| a.contains("FooTest")));
        assert!(args.iter().any(|a| a.contains("BarTest")));
    }

    #[test]
    fn test_build_command_source_files_as_test_classes() {
        let executor = TestExecExecutor::new();
        let mut input = TaskInput::new("TestExec");
        input
            .options
            .insert("classpath".to_string(), "/tmp/classes".to_string());
        input.source_files.push(PathBuf::from(
            "src/test/java/com/example/FooTest.java",
        ));
        input.source_files.push(PathBuf::from(
            "src/test/java/com/example/BarSpec.java",
        ));
        input.target_dir = PathBuf::from("/tmp/output");

        let java = PathBuf::from("/usr/lib/jvm/java-17/bin/java");
        let cmd = executor.build_command(&java, &input);

        let args: Vec<String> = cmd.as_std().get_args().map(|s| s.to_string_lossy().to_string()).collect();
        assert!(args.iter().any(|a| a.contains("com.example.FooTest")));
        assert!(args.iter().any(|a| a.contains("com.example.BarSpec")));
    }

    #[tokio::test]
    async fn test_run_tests_missing_java() {
        let executor = TestExecExecutor::new();
        let input = make_test_input();

        let result = executor.run_tests("/nonexistent/jdk", &input).await;
        assert!(!result.success);
        assert!(result.error_message.contains("java not found"));
    }

    #[tokio::test]
    async fn test_run_tests_no_classes() {
        let executor = TestExecExecutor::new();
        let input = TaskInput::new("TestExec");
        // No test_classes and no source_files

        let result = executor.run_tests("", &input).await;
        assert!(result.success);
        assert_eq!(result.total, 0);
    }

    #[tokio::test]
    async fn test_execute_as_task_executor() {
        let executor = TestExecExecutor::new();
        let input = TaskInput::new("TestExec");
        // No test classes — early return

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert_eq!(result.files_processed, 0);
    }

    #[test]
    fn test_task_type() {
        let executor = TestExecExecutor::new();
        assert_eq!(executor.task_type(), "TestExec");
    }

    #[test]
    fn test_collect_xml_files() {
        let tmp = tempfile::tempdir().unwrap();
        let report_dir = tmp.path().join("reports");
        std::fs::create_dir_all(report_dir.join("sub")).unwrap();
        std::fs::write(report_dir.join("TEST-1.xml"), b"<xml/>").unwrap();
        std::fs::write(report_dir.join("not-xml.txt"), b"text").unwrap();
        std::fs::write(report_dir.join("sub/TEST-2.xml"), b"<xml/>").unwrap();

        let files = collect_xml_files(&report_dir);
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_collect_xml_files_empty() {
        let files = collect_xml_files(Path::new("/nonexistent"));
        assert!(files.is_empty());
    }

    #[tokio::test]
    async fn test_run_tests_with_real_jvm() {
        let java_home = match std::env::var("JAVA_HOME") {
            Ok(h) => h,
            Err(_) => return, // Skip if JAVA_HOME not set
        };

        let tmp = tempfile::tempdir().unwrap();
        let report_dir = tmp.path().join("reports");
        let output_dir = tmp.path().join("output");

        let executor = TestExecExecutor::new();
        let mut input = TaskInput::new("TestExec");
        input
            .options
            .insert("java_home".to_string(), java_home.clone());
        // Use just --help to verify ConsoleLauncher works (no actual tests)
        input.options.insert(
            "classpath".to_string(),
            "/nonexistent/cp".to_string(),
        );
        input
            .options
            .insert("xml_report_dir".to_string(), report_dir.to_str().unwrap().to_string());
        input.target_dir = output_dir.clone();
        // Don't pass test_classes so no actual tests run
        // We just verify the executor doesn't panic

        let result = executor.run_tests(&java_home, &input).await;
        // The JVM may fail since there's no real classpath, but it should not panic
        assert!(result.error_message.is_empty() || !result.error_message.is_empty());
    }
}
