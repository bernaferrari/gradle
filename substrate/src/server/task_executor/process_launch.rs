use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;

#[cfg(unix)]
use nix::sys::signal::{self, Signal};
#[cfg(unix)]
use nix::unistd::Pid;

/// Rust-owned process launch contract used by native task executors.
///
/// This is intentionally smaller than Gradle's full process API. The JVM bridge
/// must lower only exact, preview-safe contracts into this shape.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct ProcessLaunchSpec {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub working_dir: Option<PathBuf>,
    pub environment: HashMap<String, String>,
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct ProcessLaunchOutput {
    pub exit_code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub timed_out: bool,
}

impl ProcessLaunchSpec {
    pub(crate) fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            args: Vec::new(),
            working_dir: None,
            environment: HashMap::new(),
            timeout: None,
        }
    }

    pub(crate) fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub(crate) fn working_dir(mut self, working_dir: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(working_dir.into());
        self
    }

    pub(crate) fn environment(mut self, environment: HashMap<String, String>) -> Self {
        self.environment = environment;
        self
    }

    #[cfg(test)]
    pub(crate) fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    pub(crate) fn to_command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        command.args(&self.args);
        if let Some(working_dir) = &self.working_dir {
            command.current_dir(working_dir);
        }
        if !self.environment.is_empty() {
            command.envs(&self.environment);
        }
        command
    }
}

/// Run a process to completion while keeping cancellation/timeout ownership in
/// Rust instead of using per-task direct `Command::output()` calls.
pub(crate) async fn run_to_output(spec: &ProcessLaunchSpec) -> Result<ProcessLaunchOutput, String> {
    if let Some(working_dir) = &spec.working_dir {
        if !working_dir.exists() {
            return Err(format!(
                "Working directory does not exist: {}",
                working_dir.display()
            ));
        }
    }

    let mut command = spec.to_command();
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    #[cfg(unix)]
    {
        command.process_group(0);
    }

    let mut child = command.spawn().map_err(|error| {
        format!(
            "Failed to execute '{}': {}",
            spec.executable.display(),
            error
        )
    })?;
    let pid = child.id();

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let stdout_task = tokio::spawn(read_pipe(stdout));
    let stderr_task = tokio::spawn(read_pipe(stderr));

    let wait_result = if let Some(timeout) = spec.timeout {
        match tokio::time::timeout(timeout, child.wait()).await {
            Ok(result) => result.map(|status| (status.code().unwrap_or(-1), false)),
            Err(_) => {
                terminate_process_tree(pid, &mut child, true).await;
                let status = child.wait().await.map_err(|error| {
                    format!(
                        "Failed to wait for timed-out '{}': {}",
                        spec.executable.display(),
                        error
                    )
                })?;
                Ok((status.code().unwrap_or(-1), true))
            }
        }
    } else {
        child
            .wait()
            .await
            .map(|status| (status.code().unwrap_or(-1), false))
    };

    let (exit_code, timed_out) = wait_result.map_err(|error| {
        format!(
            "Failed to wait for '{}': {}",
            spec.executable.display(),
            error
        )
    })?;
    let stdout = stdout_task
        .await
        .map_err(|error| format!("Failed to join stdout reader: {error}"))?;
    let stderr = stderr_task
        .await
        .map_err(|error| format!("Failed to join stderr reader: {error}"))?;

    Ok(ProcessLaunchOutput {
        exit_code,
        stdout,
        stderr,
        timed_out,
    })
}

async fn read_pipe<R>(pipe: Option<R>) -> Vec<u8>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let Some(mut pipe) = pipe else {
        return Vec::new();
    };
    let mut bytes = Vec::new();
    let _ = pipe.read_to_end(&mut bytes).await;
    bytes
}

async fn terminate_process_tree(pid: Option<u32>, child: &mut tokio::process::Child, force: bool) {
    #[cfg(unix)]
    {
        if let Some(pid) = pid {
            let signal = if force {
                Signal::SIGKILL
            } else {
                Signal::SIGTERM
            };
            let pgid = Pid::from_raw(-(pid as i32));
            let _ = signal::kill(pgid, signal);
            return;
        }
    }

    let _ = child.start_kill();
}

#[allow(dead_code)]
pub(crate) fn binary_in_java_home(
    java_home: Option<&str>,
    binary_name: &str,
    fallback: &str,
) -> PathBuf {
    match java_home.map(str::trim).filter(|home| !home.is_empty()) {
        Some(home) => Path::new(home).join("bin").join(binary_name),
        None => PathBuf::from(fallback),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn captures_stdout_stderr_exit_code_cwd_and_environment() {
        let tmp = tempfile::tempdir().unwrap();
        let mut env = HashMap::new();
        env.insert("RUST_PROCESS_LAUNCH_ENV".to_string(), "visible".to_string());

        let output = run_to_output(
            &ProcessLaunchSpec::new("/bin/sh")
                .args([
                    "-c",
                    "printf '%s:%s' \"$PWD\" \"$RUST_PROCESS_LAUNCH_ENV\"; printf 'err' >&2; exit 7",
                ])
                .working_dir(tmp.path())
                .environment(env),
        )
        .await
        .unwrap();

        assert_eq!(output.exit_code, 7);
        let expected_cwd = tmp
            .path()
            .canonicalize()
            .unwrap_or_else(|_| tmp.path().to_path_buf());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            format!("{}:visible", expected_cwd.display())
        );
        assert_eq!(String::from_utf8_lossy(&output.stderr), "err");
        assert!(!output.timed_out);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_cancels_process_tree() {
        let output = run_to_output(
            &ProcessLaunchSpec::new("/bin/sh")
                .args(["-c", "sleep 5"])
                .timeout(Duration::from_millis(50)),
        )
        .await
        .unwrap();

        assert!(output.timed_out);
    }

    #[tokio::test]
    async fn missing_working_dir_fails_before_spawn() {
        let missing = tempfile::tempdir().unwrap().path().join("missing");
        let error = run_to_output(&ProcessLaunchSpec::new("echo").working_dir(missing))
            .await
            .unwrap_err();

        assert!(error.contains("Working directory does not exist"));
    }
}
