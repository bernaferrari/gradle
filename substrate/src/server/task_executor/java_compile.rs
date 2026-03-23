use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::process::Command;

use crate::server::task_executor::{TaskExecutor, TaskInput, TaskResult};

/// Result of a Java compilation.
#[derive(Debug, Clone)]
#[derive(Default)]
pub struct JavaCompileResult {
    pub success: bool,
    pub exit_code: i32,
    pub output_files: Vec<PathBuf>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
    pub duration_ms: u64,
    pub source_files_compiled: u64,
    pub error_message: String,
}


/// Orchestrates javac compilation as a child process.
pub struct JavaCompileExecutor;

impl Default for JavaCompileExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl JavaCompileExecutor {
    pub fn new() -> Self {
        Self
    }

    /// Find the javac binary in a JDK installation.
    pub fn find_javac(java_home: &str) -> PathBuf {
        if cfg!(target_os = "windows") {
            PathBuf::from(format!("{}\\bin\\javac.exe", java_home))
        } else {
            PathBuf::from(format!("{}/bin/javac", java_home))
        }
    }

    /// Find the java binary in a JDK installation.
    pub fn find_java(java_home: &str) -> PathBuf {
        if cfg!(target_os = "windows") {
            PathBuf::from(format!("{}\\bin\\java.exe", java_home))
        } else {
            PathBuf::from(format!("{}/bin/java", java_home))
        }
    }

    /// Build the javac command line.
    pub fn build_command(
        &self,
        javac_path: &Path,
        input: &TaskInput,
    ) -> Command {
        let mut cmd = Command::new(javac_path);

        // Source files
        for source in &input.source_files {
            cmd.arg(source);
        }

        // Source path (-sourcepath)
        if let Some(source_path) = input.options.get("source_path") {
            cmd.arg("-sourcepath").arg(source_path);
        }

        // Classpath (-classpath / -cp)
        if let Some(classpath) = input.options.get("classpath") {
            cmd.arg("-classpath").arg(classpath);
        }

        // Output directory (-d)
        if !input.target_dir.as_os_str().is_empty() {
            cmd.arg("-d").arg(&input.target_dir);
        }

        // Annotation processor path (-processorpath)
        if let Some(proc_path) = input.options.get("processor_path") {
            cmd.arg("-processorpath").arg(proc_path);
        }

        // Annotation processors (-processor)
        if let Some(processors) = input.options.get("processors") {
            cmd.arg("-processor").arg(processors);
        }

        // Release version (-release)
        if let Some(release) = input.options.get("release") {
            cmd.arg("-release").arg(release);
        }

        // Source compatibility (-source)
        if let Some(source_ver) = input.options.get("source_version") {
            cmd.arg("-source").arg(source_ver);
        }

        // Target compatibility (-target)
        if let Some(target_ver) = input.options.get("target_version") {
            cmd.arg("-target").arg(target_ver);
        }

        // Encoding
        if let Some(encoding) = input.options.get("encoding") {
            cmd.arg("-encoding").arg(encoding);
        }

        // Generated sources directory (-s)
        if let Some(gen_dir) = input.options.get("generated_sources_dir") {
            cmd.arg("-s").arg(gen_dir);
        }

        // Warnings
        if input.options.get("show_warnings").map(|v| v == "true").unwrap_or(false) {
            cmd.arg("-Xlint:all");
        }

        // Verbose
        if input.options.get("verbose").map(|v| v == "true").unwrap_or(false) {
            cmd.arg("-verbose");
        }

        // Parameters for compilation
        if let Some(params) = input.options.get("parameters") {
            if params == "true" {
                cmd.arg("-parameters");
            }
        }

        // Proc only (generate but don't compile)
        if input.options.get("proc_only").map(|v| v == "true").unwrap_or(false) {
            cmd.arg("-proc:only");
        }

        cmd.stdout(Stdio::piped())
            .stderr(Stdio::piped());

        cmd
    }

    /// Parse javac output for errors, warnings, and notes.
    pub fn parse_javac_output(output: &str) -> (Vec<String>, Vec<String>, Vec<String>) {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        let mut notes = Vec::new();

        for line in output.lines() {
            let lower = line.to_lowercase();
            if lower.contains("error:") {
                errors.push(line.to_string());
            } else if lower.contains("warning:") {
                warnings.push(line.to_string());
            } else if lower.contains("note:") {
                notes.push(line.to_string());
            }
        }

        (errors, warnings, notes)
    }

    /// Collect .class files from the output directory.
    pub fn collect_output_files(output_dir: &Path) -> Vec<PathBuf> {
        let mut files = Vec::new();
        if !output_dir.exists() {
            return files;
        }
        Self::collect_class_files_recursive(output_dir, &mut files);
        files
    }

    pub fn collect_class_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    Self::collect_class_files_recursive(&path, files);
                } else if path.extension().and_then(|e| e.to_str()) == Some("class") {
                    files.push(path);
                }
            }
        }
    }

    /// Compile sources using the given Java home.
    pub async fn compile(
        &self,
        java_home: &str,
        input: &TaskInput,
    ) -> JavaCompileResult {
        let start = std::time::Instant::now();
        let mut result = JavaCompileResult::default();

        if input.source_files.is_empty() {
            result.success = true;
            result.duration_ms = start.elapsed().as_millis() as u64;
            return result;
        }

        let javac = Self::find_javac(java_home);
        if !javac.exists() {
            result.error_message = format!("javac not found at {}", javac.display());
            return result;
        }

        let mut cmd = self.build_command(&javac, input);

        tracing::debug!(
            javac = %javac.display(),
            sources = input.source_files.len(),
            "Starting javac compilation"
        );

        match cmd.output().await {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = format!("{}\n{}", stdout, stderr);

                result.exit_code = output.status.code().unwrap_or(-1);
                result.success = output.status.success();

                let (errors, warnings, notes) = Self::parse_javac_output(&combined);
                result.errors = errors;
                result.warnings = warnings;
                result.notes = notes;
                result.source_files_compiled = input.source_files.len() as u64;

                if !result.success {
                    result.error_message = format!(
                        "javac failed with exit code {}: {} errors",
                        result.exit_code,
                        result.errors.len()
                    );
                } else {
                    // Collect output .class files
                    if !input.target_dir.as_os_str().is_empty() {
                        result.output_files = Self::collect_output_files(&input.target_dir);
                    }

                    tracing::debug!(
                        duration_ms = result.duration_ms,
                        sources = result.source_files_compiled,
                        output_files = result.output_files.len(),
                        warnings = result.warnings.len(),
                        "Compilation succeeded"
                    );
                }
            }
            Err(e) => {
                result.error_message = format!("Failed to execute javac: {}", e);
            }
        }

        result.duration_ms = start.elapsed().as_millis() as u64;
        result
    }
}

#[tonic::async_trait]
impl TaskExecutor for JavaCompileExecutor {
    fn task_type(&self) -> &str {
        "JavaCompile"
    }

    async fn execute(&self, input: &TaskInput) -> TaskResult {
        let java_home = input
            .options
            .get("java_home")
            .cloned()
            .unwrap_or_else(|| {
                std::env::var("JAVA_HOME").unwrap_or_default()
            });

        let compile_result = self.compile(&java_home, input).await;

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

    fn make_compile_input(sources: Vec<PathBuf>) -> TaskInput {
        let mut input = TaskInput::new("JavaCompile");
        input.source_files = sources;
        input
    }

    #[test]
    fn test_find_javac() {
        if let Ok(java_home) = std::env::var("JAVA_HOME") {
            let javac = JavaCompileExecutor::find_javac(&java_home);
            assert!(javac.exists(), "javac should exist at {}", javac.display());
        }
    }

    #[test]
    fn test_find_java() {
        if let Ok(java_home) = std::env::var("JAVA_HOME") {
            let java = JavaCompileExecutor::find_java(&java_home);
            assert!(java.exists(), "java should exist at {}", java.display());
        }
    }

    #[test]
    fn test_parse_javac_output_errors() {
        let output = r#"
src/main/java/Foo.java:3: error: ';' expected
        System.out.println("hello")
                                 ^
1 error
"#;
        let (errors, warnings, _notes) = JavaCompileExecutor::parse_javac_output(output);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("error:"));
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_parse_javac_output_warnings() {
        let output = r#"
src/main/java/Foo.java:5: warning: [unchecked] unchecked call
        List list = new ArrayList();
              ^
1 warning
"#;
        let (errors, warnings, _notes) = JavaCompileExecutor::parse_javac_output(output);
        assert!(errors.is_empty());
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("warning:"));
    }

    #[test]
    fn test_parse_javac_output_notes() {
        let output = r#"
Note: Some input files use unchecked operations.
Note: Recompile with -Xlint:unchecked for details.
"#;
        let (errors, warnings, notes) = JavaCompileExecutor::parse_javac_output(output);
        assert!(errors.is_empty());
        assert!(warnings.is_empty());
        assert_eq!(notes.len(), 2);
    }

    #[test]
    fn test_parse_javac_output_empty() {
        let (errors, warnings, notes) = JavaCompileExecutor::parse_javac_output("");
        assert!(errors.is_empty());
        assert!(warnings.is_empty());
        assert!(notes.is_empty());
    }

    #[test]
    fn test_parse_javac_output_mixed() {
        let output = r#"
src/main/java/Foo.java:1: error: class Foo is public, should be declared in a file named Foo.java
public class Foo {
       ^
src/main/java/Bar.java:5: warning: [serial] serializable class Bar has no definition of serialVersionUID
public class Bar implements java.io.Serializable {
       ^
2 errors, 1 warning
"#;
        let (errors, warnings, notes) = JavaCompileExecutor::parse_javac_output(output);
        assert_eq!(errors.len(), 1);
        assert_eq!(warnings.len(), 1);
        assert!(notes.is_empty());
    }

    #[tokio::test]
    async fn test_compile_missing_javac() {
        let executor = JavaCompileExecutor::new();
        let input = make_compile_input(vec![PathBuf::from("/nonexistent/Foo.java")]);

        let result = executor.compile("/nonexistent/jdk", &input).await;
        assert!(!result.success);
        assert!(result.error_message.contains("javac not found"));
    }

    #[tokio::test]
    async fn test_compile_empty_sources() {
        let executor = JavaCompileExecutor::new();
        let input = make_compile_input(vec![]);

        let result = executor.compile("", &input).await;
        assert!(result.success);
        assert_eq!(result.source_files_compiled, 0);
    }

    #[tokio::test]
    async fn test_compile_real_java() {
        let java_home = match std::env::var("JAVA_HOME") {
            Ok(h) => h,
            Err(_) => return, // Skip if JAVA_HOME not set
        };

        let tmp = tempfile::tempdir().unwrap();
        let src_file = tmp.path().join("Hello.java");
        std::fs::write(
            &src_file,
            r#"
public class Hello {
    public static void main(String[] args) {
        System.out.println("Hello, World!");
    }
}
"#,
        )
        .unwrap();

        let output_dir = tmp.path().join("classes");

        let executor = JavaCompileExecutor::new();
        let mut input = make_compile_input(vec![src_file]);
        input.target_dir = output_dir.clone();

        let result = executor.compile(&java_home, &input).await;
        assert!(result.success, "Compilation should succeed: {}", result.error_message);
        assert_eq!(result.source_files_compiled, 1);
        assert!(result.errors.is_empty());

        // Check output class file exists
        assert!(output_dir.join("Hello.class").exists());
    }

    #[tokio::test]
    async fn test_compile_syntax_error() {
        let java_home = match std::env::var("JAVA_HOME") {
            Ok(h) => h,
            Err(_) => return,
        };

        let tmp = tempfile::tempdir().unwrap();
        let src_file = tmp.path().join("Bad.java");
        std::fs::write(
            &src_file,
            r#"
public class Bad {
    public static void main(String[] args) {
        System.out.println("missing semicolon")
    }
}
"#,
        )
        .unwrap();

        let output_dir = tmp.path().join("classes");

        let executor = JavaCompileExecutor::new();
        let mut input = make_compile_input(vec![src_file]);
        input.target_dir = output_dir;

        let result = executor.compile(&java_home, &input).await;
        assert!(!result.success);
        assert!(!result.errors.is_empty());
    }

    #[tokio::test]
    async fn test_compile_with_classpath() {
        let java_home = match std::env::var("JAVA_HOME") {
            Ok(h) => h,
            Err(_) => return,
        };

        let tmp = tempfile::tempdir().unwrap();
        let src_file = tmp.path().join("Greet.java");
        std::fs::write(
            &src_file,
            r#"
import java.util.List;
import java.util.ArrayList;
public class Greet {
    public List<String> names() {
        List<String> result = new ArrayList<>();
        result.add("hello");
        return result;
    }
}
"#,
        )
        .unwrap();

        let output_dir = tmp.path().join("classes");

        let executor = JavaCompileExecutor::new();
        let mut input = make_compile_input(vec![src_file]);
        input.target_dir = output_dir.clone();
        input
            .options
            .insert("classpath".to_string(), ".".to_string());

        let result = executor.compile(&java_home, &input).await;
        assert!(result.success, "Compilation should succeed: {}", result.error_message);
        assert!(output_dir.join("Greet.class").exists());
    }

    #[tokio::test]
    async fn test_execute_as_task_executor() {
        let java_home = match std::env::var("JAVA_HOME") {
            Ok(h) => h,
            Err(_) => return,
        };

        let tmp = tempfile::tempdir().unwrap();
        let src_file = tmp.path().join("Simple.java");
        std::fs::write(
            &src_file,
            "public class Simple { public int value() { return 42; } }",
        )
        .unwrap();

        let output_dir = tmp.path().join("out");

        let executor = JavaCompileExecutor::new();
        let mut input = TaskInput::new("JavaCompile");
        input.source_files.push(src_file);
        input.target_dir = output_dir.clone();
        input
            .options
            .insert("java_home".to_string(), java_home);

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert_eq!(result.files_processed, 1);
    }

    #[test]
    fn test_task_type() {
        let executor = JavaCompileExecutor::new();
        assert_eq!(executor.task_type(), "JavaCompile");
    }

    #[test]
    fn test_collect_output_files() {
        let tmp = tempfile::tempdir().unwrap();
        let classes_dir = tmp.path().join("classes");
        std::fs::create_dir_all(classes_dir.join("sub")).unwrap();
        std::fs::write(classes_dir.join("A.class"), b"class A").unwrap();
        std::fs::write(classes_dir.join("B.java"), b"class B").unwrap();
        std::fs::write(classes_dir.join("sub/C.class"), b"class C").unwrap();

        let files = JavaCompileExecutor::collect_output_files(&classes_dir);
        assert_eq!(files.len(), 2); // A.class and sub/C.class
    }

    #[test]
    fn test_collect_output_files_empty() {
        let files = JavaCompileExecutor::collect_output_files(Path::new("/nonexistent"));
        assert!(files.is_empty());
    }
}
