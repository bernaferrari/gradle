use std::sync::Arc;

use dashmap::DashMap;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};

#[cfg(unix)]
use nix::sys::signal::{self, Signal};
#[cfg(unix)]
use nix::unistd::Pid;

use crate::proto::{
    exec_service_server::ExecService, ExecOutputChunk, ExecOutputRequest, ExecKillTreeRequest,
    ExecKillTreeResponse, ExecSignalRequest, ExecSignalResponse, ExecSpawnRequest, ExecSpawnResponse,
    ExecWaitRequest, ExecWaitResponse,
};

/// A managed child process with output streaming capability.
struct ManagedProcess {
    child: tokio::process::Child,
    stdout_rx: Option<mpsc::Receiver<Vec<u8>>>,
    stderr_rx: Option<mpsc::Receiver<Vec<u8>>>,
}

pub struct ExecServiceImpl {
    processes: Arc<DashMap<u32, ManagedProcess>>,
}

impl ExecServiceImpl {
    pub fn new() -> Self {
        Self {
            processes: Arc::new(DashMap::new()),
        }
    }
}

#[tonic::async_trait]
impl ExecService for ExecServiceImpl {
    async fn spawn(
        &self,
        request: Request<ExecSpawnRequest>,
    ) -> Result<Response<ExecSpawnResponse>, Status> {
        let req = request.into_inner();

        let mut cmd = Command::new(&req.command);
        cmd.args(&req.args)
            .current_dir(&req.working_dir)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        // Create a new process group for tree cleanup
        #[cfg(unix)]
        {
            cmd.process_group(0);
        }

        if req.redirect_error_stream {
            cmd.stderr(std::process::Stdio::null());
        }

        for (k, v) in &req.environment {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn().map_err(|e| {
            Status::internal(format!("Failed to spawn '{}': {}", req.command, e))
        })?;

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
            let tx = stdout_tx;
            tokio::spawn(async move {
                let reader = tokio::io::BufReader::new(stdout);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut data = line.into_bytes();
                    data.push(b'\n');
                    if tx.send(data).await.is_err() {
                        break;
                    }
                }
            });
        }

        if let Some(stderr) = child.stderr.take() {
            let tx = stderr_tx;
            tokio::spawn(async move {
                let reader = tokio::io::BufReader::new(stderr);
                let mut lines = reader.lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let mut data = line.into_bytes();
                    data.push(b'\n');
                    if tx.send(data).await.is_err() {
                        break;
                    }
                }
            });
        }

        self.processes.insert(
            pid,
            ManagedProcess {
                child,
                stdout_rx: Some(stdout_rx),
                stderr_rx: Some(stderr_rx),
            },
        );

        tracing::debug!(pid, command = %req.command, "Spawned process");

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

        let status = entry
            .child
            .wait()
            .await
            .map_err(|e| Status::internal(format!("Failed to wait for process {}: {}", pid, e)))?;

        let exit_code = status.code().unwrap_or(-1);
        tracing::debug!(pid, exit_code, "Process exited");

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
                _ => Signal::SIGTERM,
            };
            let pid = Pid::from_raw(req.pid as i32);
            match signal::kill(pid, sig) {
                Ok(_) => Ok(Response::new(ExecSignalResponse {
                    success: true,
                    error_message: String::new(),
                })),
                Err(e) => Ok(Response::new(ExecSignalResponse {
                    success: false,
                    error_message: e.to_string(),
                })),
            }
        }

        #[cfg(not(unix))]
        {
            if let Some(mut child) = self.processes.get_mut(&(req.pid as u32)) {
                let _ = child.child.start_kill();
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
                            tracing::debug!(pid, "Process tree terminated gracefully");
                        }
                        Err(_) => {
                            // Force kill the process group
                            let _ = signal::kill(pgid, Signal::SIGKILL);
                            let _ = entry.child.wait().await;
                            tracing::debug!(pid, "Process tree force-killed after timeout");
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

        tracing::debug!(pid, force, "Kill tree completed");

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

        let stdout_rx: Option<mpsc::Receiver<Vec<u8>>> = entry.stdout_rx.take();
        let stderr_rx: Option<mpsc::Receiver<Vec<u8>>> = entry.stderr_rx.take();

        let (tx, rx) = mpsc::channel(128);

        tokio::spawn(async move {
            if let Some(mut rx) = stdout_rx {
                while let Some(data) = rx.recv().await {
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
            if let Some(mut rx) = stderr_rx {
                while let Some(data) = rx.recv().await {
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
    async fn test_spawn_echo() {
        let svc = ExecServiceImpl::new();

        let spawn_resp = svc
            .spawn(Request::new(ExecSpawnRequest {
                command: "echo".to_string(),
                args: vec!["hello".to_string()],
                environment: Default::default(),
                working_dir: "/tmp".to_string(),
                redirect_error_stream: false,
            }))
            .await
            .unwrap();

        assert!(spawn_resp.into_inner().success);
    }
}
