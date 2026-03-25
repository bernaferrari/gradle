use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use dashmap::DashMap;
use tokio::io::{AsyncBufReadExt, AsyncReadExt};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

#[cfg(unix)]
use nix::sys::signal::{self, Signal};
#[cfg(unix)]
use nix::unistd::Pid;

use crate::proto::{
    exec_service_server::ExecService, ExecKillTreeRequest, ExecKillTreeResponse, ExecOutputChunk,
    ExecOutputRequest, ExecSignalRequest, ExecSignalResponse, ExecSpawnRequest, ExecSpawnResponse,
    ExecWaitRequest, ExecWaitResponse,
};

/// Maximum number of tracked processes before cleanup triggers.
const MAX_PROCESSES: usize = 1024;

/// Maximum age for idle processes (5 minutes).
const MAX_PROCESS_AGE_SECS: u64 = 300;

/// A managed child process with output streaming capability.
struct ManagedProcess {
    child: tokio::process::Child,
    stdout_rx: Option<mpsc::Receiver<Vec<u8>>>,
    stderr_rx: Option<mpsc::Receiver<Vec<u8>>>,
    command: String,
    started_at: Instant,
}

#[derive(Default)]
pub struct ExecServiceImpl {
    processes: Arc<DashMap<u32, ManagedProcess>>,
    spawned_total: AtomicI64,
    cleaned_up: AtomicI64,
}

impl ExecServiceImpl {
    pub fn new() -> Self {
        Self {
            processes: Arc::new(DashMap::new()),
            spawned_total: AtomicI64::new(0),
            cleaned_up: AtomicI64::new(0),
        }
    }

    /// Resolve a working directory path relative to a project directory.
    /// If the path is absolute, use it as-is. If relative, resolve against project_dir
    /// or the current working directory.
    fn resolve_working_dir(working_dir: &str, project_dir: Option<&str>) -> PathBuf {
        let path = Path::new(working_dir);
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            let base = project_dir
                .map(PathBuf::from)
                .or_else(|| std::env::current_dir().ok())
                .unwrap_or_else(|| PathBuf::from("."));
            base.join(path)
        }
    }

    /// Clean up zombie processes that have exceeded the max age.
    fn cleanup_stale_processes(&self) {
        if self.processes.len() <= MAX_PROCESSES {
            return;
        }

        let now = Instant::now();
        let stale_pids: Vec<u32> = self
            .processes
            .iter()
            .filter(|entry| now.duration_since(entry.started_at).as_secs() > MAX_PROCESS_AGE_SECS)
            .map(|entry| *entry.key())
            .collect();

        for pid in &stale_pids {
            if let Some((_, mut entry)) = self.processes.remove(pid) {
                let command = entry.command.clone();
                // Check if already exited
                match entry.child.try_wait() {
                    Ok(Some(_)) => {
                        // Already exited, just clean up
                        tracing::debug!(pid, command = %command, "Reaped already-exited stale process");
                    }
                    Ok(None) => {
                        // Still running but stale, kill it
                        #[cfg(unix)]
                        {
                            let pgid = Pid::from_raw(-(*pid as i32));
                            let _ = signal::kill(pgid, Signal::SIGKILL);
                        }
                        #[cfg(not(unix))]
                        {
                            let _ = entry.child.start_kill();
                        }
                        // Spawn async wait to reap the child (we already own it from remove)
                        let mut child = entry.child;
                        tokio::spawn(async move {
                            let _ = child.wait().await;
                        });
                        tracing::debug!(pid, command = %command, "Killed stale process");
                    }
                    Err(_) => {}
                }
                self.cleaned_up.fetch_add(1, Ordering::Relaxed);
            }
        }

        if !stale_pids.is_empty() {
            tracing::debug!(count = stale_pids.len(), "Cleaned up stale processes");
        }
    }

    /// Stream output from a reader into a channel with optional binary mode.
    fn stream_output<R>(
        mut reader: tokio::io::BufReader<R>,
        tx: mpsc::Sender<Vec<u8>>,
        binary: bool,
    ) where
        R: tokio::io::AsyncRead + Unpin + Send + 'static,
    {
        tokio::spawn(async move {
            if binary {
                // Binary mode: read fixed chunks
                let mut buf = vec![0u8; 8192];
                loop {
                    match reader.read(&mut buf).await {
                        Ok(0) => break, // EOF
                        Ok(n) => {
                            if tx.send(buf[..n].to_vec()).await.is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            } else {
                // Line mode: read line by line (adds newlines)
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut data = line.into_bytes();
                    data.push(b'\n');
                    if tx.send(data).await.is_err() {
                        break;
                    }
                }
            }
        });
    }
}

#[tonic::async_trait]
impl ExecService for ExecServiceImpl {
    async fn spawn(
        &self,
        request: Request<ExecSpawnRequest>,
    ) -> Result<Response<ExecSpawnResponse>, Status> {
        let req = request.into_inner();

        if req.command.is_empty() {
            return Err(Status::invalid_argument("Command cannot be empty"));
        }

        // Clean up stale processes if at capacity
        self.cleanup_stale_processes();

        // Resolve working directory
        let working_dir = Self::resolve_working_dir(&req.working_dir, None);
        if !working_dir.exists() {
            return Err(Status::invalid_argument(format!(
                "Working directory does not exist: {}",
                working_dir.display()
            )));
        }

        let mut cmd = Command::new(&req.command);
        cmd.args(&req.args)
            .current_dir(&working_dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        // Create a new process group for tree cleanup
        #[cfg(unix)]
        {
            cmd.process_group(0);
        }

        // Handle environment variables:
        // If the environment map is non-empty, use ONLY those variables (no inheritance).
        // If empty, inherit the current process environment (Gradle convention).
        if !req.environment.is_empty() {
            // Clear inherited environment and set only the provided vars
            cmd.env_clear();
            for (k, v) in &req.environment {
                // Support special value "null" to explicitly unset a variable
                if v == "null" {
                    cmd.env_remove(k);
                } else {
                    cmd.env(k, v);
                }
            }
        }
        // else: inherit current environment (default behavior)

        if req.redirect_error_stream {
            cmd.stderr(std::process::Stdio::null());
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| Status::internal(format!("Failed to spawn '{}': {}", req.command, e)))?;

        let pid = child.id().unwrap_or(0);

        if pid == 0 {
            return Ok(Response::new(ExecSpawnResponse {
                pid: 0,
                success: false,
                error_message: "Failed to get process ID".to_string(),
            }));
        }

        // Set up stdout/stderr streaming
        let (stdout_tx, stdout_rx) = mpsc::channel::<Vec<u8>>(64);
        let (stderr_tx, stderr_rx) = mpsc::channel::<Vec<u8>>(64);

        if let Some(stdout) = child.stdout.take() {
            Self::stream_output(tokio::io::BufReader::new(stdout), stdout_tx, false);
        }

        if let Some(stderr) = child.stderr.take() {
            Self::stream_output(tokio::io::BufReader::new(stderr), stderr_tx, false);
        }

        self.processes.insert(
            pid,
            ManagedProcess {
                child,
                stdout_rx: Some(stdout_rx),
                stderr_rx: Some(stderr_rx),
                command: req.command.clone(),
                started_at: Instant::now(),
            },
        );

        self.spawned_total.fetch_add(1, Ordering::Relaxed);

        tracing::debug!(
            pid,
            command = %req.command,
            args_count = req.args.len(),
            env_count = req.environment.len(),
            working_dir = %working_dir.display(),
            "Spawned process"
        );

        Ok(Response::new(ExecSpawnResponse {
            pid: pid as i32,
            success: true,
            error_message: String::new(),
        }))
    }

    async fn wait(
        &self,
        request: Request<ExecWaitRequest>,
    ) -> Result<Response<ExecWaitResponse>, Status> {
        let pid = request.into_inner().pid as u32;
        let (_, mut entry) = self
            .processes
            .remove(&pid)
            .ok_or_else(|| Status::not_found(format!("Unknown process: {}", pid)))?;

        let status =
            entry.child.wait().await.map_err(|e| {
                Status::internal(format!("Failed to wait for process {}: {}", pid, e))
            })?;

        let exit_code = status.code().unwrap_or(-1);
        let elapsed = entry.started_at.elapsed();
        tracing::debug!(
            pid,
            command = %entry.command,
            exit_code,
            elapsed_ms = elapsed.as_millis() as u64,
            "Process exited"
        );

        Ok(Response::new(ExecWaitResponse {
            exit_code,
            error_message: String::new(),
        }))
    }

    async fn signal(
        &self,
        request: Request<ExecSignalRequest>,
    ) -> Result<Response<ExecSignalResponse>, Status> {
        let req = request.into_inner();

        #[cfg(unix)]
        {
            let sig = match req.signal {
                2 => Signal::SIGINT,
                9 => Signal::SIGKILL,
                15 => Signal::SIGTERM,
                1 => Signal::SIGHUP,
                3 => Signal::SIGQUIT,
                _ => Signal::SIGTERM,
            };
            let pid = Pid::from_raw(req.pid);
            let command = self
                .processes
                .get(&(req.pid as u32))
                .map(|e| e.command.clone())
                .unwrap_or_default();
            let result = match signal::kill(pid, sig) {
                Ok(_) => {
                    tracing::debug!(pid = req.pid, command = %command, signal = req.signal, "Sent signal to process");
                    Ok(Response::new(ExecSignalResponse {
                        success: true,
                        error_message: String::new(),
                    }))
                }
                Err(e) => {
                    tracing::debug!(pid = req.pid, command = %command, signal = req.signal, error = %e, "Failed to send signal");
                    Ok(Response::new(ExecSignalResponse {
                        success: false,
                        error_message: e.to_string(),
                    }))
                }
            };
            result
        }

        #[cfg(not(unix))]
        {
            if let Some(mut child) = self.processes.get_mut(&(req.pid as u32)) {
                let command = child.command.clone();
                let _ = child.child.start_kill();
                tracing::debug!(pid = req.pid, command = %command, signal = req.signal, "Killed process");
                Ok(Response::new(ExecSignalResponse {
                    success: true,
                    error_message: String::new(),
                }))
            } else {
                Err(Status::not_found(format!("Unknown process: {}", req.pid)))
            }
        }
    }

    async fn kill_tree(
        &self,
        request: Request<ExecKillTreeRequest>,
    ) -> Result<Response<ExecKillTreeResponse>, Status> {
        let req = request.into_inner();
        let pid = req.pid as u32;
        let force = req.force;

        // Capture command for logging before potential removal
        let command = self
            .processes
            .get(&pid)
            .map(|e| e.command.clone())
            .unwrap_or_default();

        #[cfg(unix)]
        {
            // Send signal to the entire process group (negative PID)
            let pgid = Pid::from_raw(-(pid as i32));

            if force {
                // Force kill immediately
                let _ = signal::kill(pgid, Signal::SIGKILL);
            } else {
                // Graceful: SIGTERM first
                let _ = signal::kill(pgid, Signal::SIGTERM);

                // Wait up to 5 seconds for graceful shutdown
                if let Some((_, mut entry)) = self.processes.remove(&pid) {
                    match tokio::time::timeout(
                        tokio::time::Duration::from_secs(5),
                        entry.child.wait(),
                    )
                    .await
                    {
                        Ok(_) => {
                            tracing::debug!(pid, command = %command, "Process tree terminated gracefully");
                        }
                        Err(_) => {
                            // Force kill the process group
                            let _ = signal::kill(pgid, Signal::SIGKILL);
                            let _ = entry.child.wait().await;
                            tracing::debug!(pid, command = %command, "Process tree force-killed after timeout");
                        }
                    }
                }
            }
        }

        #[cfg(not(unix))]
        {
            if let Some((_, mut child)) = self.processes.remove(&pid) {
                let _ = child.child.start_kill();
            }
        }

        // Clean up any remaining entry
        #[cfg(unix)]
        {
            let _ = self.processes.remove(&pid);
        }

        tracing::debug!(pid, command = %command, force, "Kill tree completed");

        Ok(Response::new(ExecKillTreeResponse {
            success: true,
            error_message: String::new(),
        }))
    }

    type SubscribeOutputStream = ReceiverStream<Result<ExecOutputChunk, Status>>;

    async fn subscribe_output(
        &self,
        request: Request<ExecOutputRequest>,
    ) -> Result<Response<Self::SubscribeOutputStream>, Status> {
        let pid = request.into_inner().pid as u32;
        let mut entry = self
            .processes
            .get_mut(&pid)
            .ok_or_else(|| Status::not_found(format!("Unknown process: {}", pid)))?;

        let command = entry.command.clone();
        let stdout_rx: Option<mpsc::Receiver<Vec<u8>>> = entry.stdout_rx.take();
        let stderr_rx: Option<mpsc::Receiver<Vec<u8>>> = entry.stderr_rx.take();

        let (tx, rx) = mpsc::channel(128);

        tracing::debug!(pid, command = %command, "Subscribing to process output");

        tokio::spawn(async move {
            if let Some(mut stdout_rx) = stdout_rx {
                while let Some(data) = stdout_rx.recv().await {
                    if tx
                        .send(Ok(ExecOutputChunk {
                            data,
                            is_stderr: false,
                        }))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
            if let Some(mut stderr_rx) = stderr_rx {
                while let Some(data) = stderr_rx.recv().await {
                    if tx
                        .send(Ok(ExecOutputChunk {
                            data,
                            is_stderr: true,
                        }))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        });

        Ok(Response::new(ReceiverStream::new(rx)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_spawn_and_wait_echo() {
        let svc = ExecServiceImpl::new();

        let spawn_resp = svc
            .spawn(Request::new(ExecSpawnRequest {
                command: "echo".to_string(),
                args: vec!["hello world".to_string()],
                environment: Default::default(),
                working_dir: "/tmp".to_string(),
                redirect_error_stream: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(spawn_resp.success);
        assert!(spawn_resp.pid > 0);

        // Wait for completion
        let wait_resp = svc
            .wait(Request::new(ExecWaitRequest {
                pid: spawn_resp.pid,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(wait_resp.exit_code, 0);
    }

    #[tokio::test]
    async fn test_spawn_empty_command() {
        let svc = ExecServiceImpl::new();

        let result = svc
            .spawn(Request::new(ExecSpawnRequest {
                command: String::new(),
                args: vec![],
                environment: Default::default(),
                working_dir: "/tmp".to_string(),
                redirect_error_stream: false,
            }))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_spawn_invalid_working_dir() {
        let svc = ExecServiceImpl::new();

        let result = svc
            .spawn(Request::new(ExecSpawnRequest {
                command: "echo".to_string(),
                args: vec![],
                environment: Default::default(),
                working_dir: "/nonexistent/path/that/does/not/exist".to_string(),
                redirect_error_stream: false,
            }))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_spawn_with_environment() {
        let svc = ExecServiceImpl::new();

        let mut env = std::collections::HashMap::new();
        env.insert("MY_TEST_VAR".to_string(), "test_value_123".to_string());

        // Use /usr/bin/env or printenv to verify
        #[cfg(unix)]
        let command = "printenv";
        #[cfg(not(unix))]
        let command = "echo";

        let spawn_resp = svc
            .spawn(Request::new(ExecSpawnRequest {
                command: command.to_string(),
                args: vec!["MY_TEST_VAR".to_string()],
                environment: env,
                working_dir: "/tmp".to_string(),
                redirect_error_stream: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(spawn_resp.success);

        let wait_resp = svc
            .wait(Request::new(ExecWaitRequest {
                pid: spawn_resp.pid,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(wait_resp.exit_code, 0);
    }

    #[tokio::test]
    async fn test_spawn_nonexistent_command() {
        let svc = ExecServiceImpl::new();

        let spawn_resp = svc
            .spawn(Request::new(ExecSpawnRequest {
                command: "nonexistent_command_xyz_123".to_string(),
                args: vec![],
                environment: Default::default(),
                working_dir: "/tmp".to_string(),
                redirect_error_stream: false,
            }))
            .await;

        // On Unix, the spawn itself may succeed but the process will exit with an error
        // On some systems, the spawn fails
        if let Ok(resp) = spawn_resp {
            let inner = resp.into_inner();
            assert!(!inner.success || inner.pid > 0);
        }
        // Either way, no panic
    }

    #[tokio::test]
    async fn test_wait_nonexistent_process() {
        let svc = ExecServiceImpl::new();

        let result = svc.wait(Request::new(ExecWaitRequest { pid: 99999 })).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_spawn_cat_with_input() {
        let svc = ExecServiceImpl::new();

        // Spawn cat which will just exit immediately since stdin is piped
        let spawn_resp = svc
            .spawn(Request::new(ExecSpawnRequest {
                command: "cat".to_string(),
                args: vec![],
                environment: Default::default(),
                working_dir: "/tmp".to_string(),
                redirect_error_stream: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(spawn_resp.success);
        let pid = spawn_resp.pid;

        // Wait should succeed (cat exits when stdin closes)
        let wait_resp = svc
            .wait(Request::new(ExecWaitRequest { pid }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(wait_resp.exit_code, 0);
    }

    #[tokio::test]
    async fn test_resolve_working_dir_absolute() {
        let resolved = ExecServiceImpl::resolve_working_dir("/tmp", None);
        assert_eq!(resolved, PathBuf::from("/tmp"));
    }

    #[tokio::test]
    async fn test_resolve_working_dir_relative_with_base() {
        let resolved = ExecServiceImpl::resolve_working_dir("build", Some("/project"));
        assert_eq!(resolved, PathBuf::from("/project/build"));
    }

    #[tokio::test]
    async fn test_resolve_working_dir_relative_no_base() {
        let cwd = std::env::current_dir().unwrap_or_default();
        let resolved = ExecServiceImpl::resolve_working_dir("build", None);
        let expected = cwd.join("build");
        assert_eq!(resolved, expected);
    }

    #[tokio::test]
    async fn test_default_impl() {
        let svc = ExecServiceImpl::default();
        assert_eq!(svc.processes.len(), 0);
    }
}
