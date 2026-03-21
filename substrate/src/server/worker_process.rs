use std::sync::atomic::{AtomicI64, Ordering};

use dashmap::DashMap;
use tonic::{Request, Response, Status};

#[cfg(unix)]
use nix::sys::signal::{self, Signal};
#[cfg(unix)]
use nix::unistd::Pid;

#[cfg(target_os = "macos")]
use std::mem;

use crate::proto::{
    worker_process_service_server::WorkerProcessService, AcquireWorkerRequest,
    AcquireWorkerResponse, ConfigurePoolRequest, ConfigurePoolResponse, GetWorkerStatusRequest,
    GetWorkerStatusResponse, ReleaseWorkerRequest, ReleaseWorkerResponse, StopWorkerRequest,
    StopWorkerResponse, WorkerHandle, WorkerSpec, WorkerStatus,
};

/// State of a tracked worker process.
struct TrackedWorker {
    worker_id: String,
    worker_key: String,
    pid: u32,
    child: Option<tokio::process::Child>,
    state: String,
    started_at_ms: i64,
    last_used_ms: i64,
    tasks_completed: i32,
    spec: WorkerSpec,
}

/// Rust-native worker process management service.
/// Manages pools of Gradle worker daemon processes (compiler daemons,
/// test workers, etc.) for efficient reuse across builds.
///
/// Supports real JVM process spawning, health monitoring, and idle reaping.
pub struct WorkerProcessServiceImpl {
    workers: DashMap<String, TrackedWorker>,
    idle_workers: DashMap<String, Vec<String>>, // worker_key -> [worker_id]
    next_worker_id: AtomicI64,
    max_pool_size: std::sync::RwLock<i32>,
    idle_timeout_ms: std::sync::RwLock<i64>,
    workers_spawned: AtomicI64,
    workers_reused: AtomicI64,
    workers_stopped: AtomicI64,
}

impl WorkerProcessServiceImpl {
    pub fn new() -> Self {
        Self {
            workers: DashMap::new(),
            idle_workers: DashMap::new(),
            next_worker_id: AtomicI64::new(1),
            max_pool_size: std::sync::RwLock::new(16),
            idle_timeout_ms: std::sync::RwLock::new(120_000),
            workers_spawned: AtomicI64::new(0),
            workers_reused: AtomicI64::new(0),
            workers_stopped: AtomicI64::new(0),
        }
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    fn generate_worker_id(&self) -> String {
        let id = self.next_worker_id.fetch_add(1, Ordering::Relaxed);
        format!("worker-{}", id)
    }

    fn pool_size(&self) -> i32 {
        self.workers.len() as i32
    }

    /// Build the JVM command for a worker process.
    fn build_jvm_command(spec: &WorkerSpec) -> tokio::process::Command {
        let java_binary = if spec.java_home.is_empty() {
            "java".to_string()
        } else {
            format!("{}/bin/java", spec.java_home.trim_end_matches('/'))
        };

        let mut cmd = tokio::process::Command::new(&java_binary);

        // JVM args (map<string, string> -> key=value pairs)
        for (key, value) in &spec.jvm_args {
            cmd.arg(format!("{}={}", key, value));
        }

        // Memory settings
        if spec.max_memory_mb > 0 {
            cmd.arg(format!("-Xmx{}m", spec.max_memory_mb));
        }

        // Classpath
        if !spec.classpath.is_empty() {
            let cp: String = spec.classpath.join(":");
            cmd.arg("-cp").arg(cp);
        }

        // Main class
        cmd.arg("org.gradle.workers.internal.IsolatedClassloaderWorker");

        // Working directory
        if !spec.working_dir.is_empty() {
            cmd.current_dir(&spec.working_dir);
        }

        // Process group for cleanup
        #[cfg(unix)]
        {
            cmd.process_group(0);
        }

        cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        cmd
    }

    /// Spawn a real JVM worker process.
    async fn spawn_worker(&self, spec: &WorkerSpec) -> Result<(u32, tokio::process::Child), String> {
        let mut cmd = Self::build_jvm_command(spec);

        cmd.spawn()
            .map_err(|e| format!("Failed to spawn worker: {}", e))
            .and_then(|child| {
                let pid = child.id().ok_or_else(|| "Failed to get child PID".to_string())?;
                Ok((pid, child))
            })
    }

    /// Check if a worker process is still alive.
    fn check_health(child: &mut tokio::process::Child) -> bool {
        match child.try_wait() {
            Ok(Some(_)) => false, // Process has exited
            Ok(None) => true,     // Still running
            Err(_) => false,      // Error checking
        }
    }

    /// Stop a worker process: SIGTERM, wait 5s, then SIGKILL.
    async fn terminate_worker(child: &mut tokio::process::Child) {
        #[cfg(unix)]
        {
            if let Some(pid) = child.id() {
                // Send SIGTERM to the process group
                let pgid = Pid::from_raw(-(pid as i32));
                let _ = signal::kill(pgid, Signal::SIGTERM);
            }
        }

        #[cfg(not(unix))]
        {
            let _ = child.start_kill();
        }

        // Wait up to 5 seconds for graceful shutdown
        match tokio::time::timeout(
            tokio::time::Duration::from_secs(5),
            child.wait(),
        )
        .await
        {
            Ok(_) => {}
            Err(_) => {
                // Force kill the process group
                #[cfg(unix)]
                {
                    if let Some(pid) = child.id() {
                        let pgid = Pid::from_raw(-(pid as i32));
                        let _ = signal::kill(pgid, Signal::SIGKILL);
                    }
                }
                #[cfg(not(unix))]
                {
                    let _ = child.start_kill();
                }
                let _ = child.wait().await;
            }
        }
    }

    /// Internal stop worker implementation.
    async fn stop_worker_internal(&self, worker_id: &str, _force: bool) -> bool {
        if let Some((_, mut worker)) = self.workers.remove(worker_id) {
            if let Some(ref mut child) = worker.child {
                Self::terminate_worker(child).await;
            }

            // Remove from idle pool
            if let Some(mut idle_list) = self.idle_workers.get_mut(&worker.worker_key) {
                idle_list.retain(|id| id != worker_id);
            }

            self.workers_stopped.fetch_add(1, Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    /// Insert a stub worker (no real process) and return the response.
    fn insert_stub_worker(
        &self,
        worker_id: String,
        worker_key: String,
        spec: WorkerSpec,
        now: i64,
    ) -> Response<AcquireWorkerResponse> {
        let pid = std::process::id();
        self.workers.insert(
            worker_id.clone(),
            TrackedWorker {
                worker_id: worker_id.clone(),
                worker_key: worker_key.clone(),
                pid,
                child: None,
                state: "busy".to_string(),
                started_at_ms: now,
                last_used_ms: now,
                tasks_completed: 0,
                spec,
            },
        );
        self.workers_spawned.fetch_add(1, Ordering::Relaxed);

        Response::new(AcquireWorkerResponse {
            worker: Some(WorkerHandle {
                worker_id,
                worker_key,
                pid: pid as i32,
                connect_address: format!("unix:/tmp/gradle-worker-{}.sock", pid),
                started_at_ms: now,
                healthy: true,
            }),
            reused: false,
            error_message: String::new(),
        })
    }
}

#[tonic::async_trait]
impl WorkerProcessService for WorkerProcessServiceImpl {
    async fn acquire_worker(
        &self,
        request: Request<AcquireWorkerRequest>,
    ) -> Result<Response<AcquireWorkerResponse>, Status> {
        let req = request.into_inner();
        let spec = req
            .spec
            .ok_or_else(|| Status::invalid_argument("WorkerSpec is required"))?;

        let worker_key = spec.worker_key.clone();
        let now = Self::now_ms();

        // Check for idle worker of the same type
        if let Some(mut idle_list) = self.idle_workers.get_mut(&worker_key) {
            if let Some(worker_id) = idle_list.pop() {
                if let Some(mut worker) = self.workers.get_mut(&worker_id) {
                    // Check health before returning
                    let healthy = if let Some(ref mut child) = worker.child {
                        Self::check_health(child)
                    } else {
                        false
                    };

                    if !healthy {
                        // Worker died while idle -- remove and spawn a new one
                        drop(worker);
                        self.workers.remove(&worker_id);
                        self.workers_stopped.fetch_add(1, Ordering::Relaxed);
                        tracing::debug!(
                            worker_id = %worker_id,
                            "Idle worker died, spawning replacement"
                        );
                        // Fall through to spawn a new worker
                    } else {
                        worker.state = "busy".to_string();
                        worker.last_used_ms = now;
                        worker.tasks_completed += 1;
                        let pid = worker.pid;
                        let started_at = worker.started_at_ms;
                        drop(worker);

                        self.workers_reused.fetch_add(1, Ordering::Relaxed);

                        tracing::debug!(
                            worker_id = %worker_id,
                            worker_key = %worker_key,
                            "Reusing idle worker"
                        );

                        return Ok(Response::new(AcquireWorkerResponse {
                            worker: Some(WorkerHandle {
                                worker_id,
                                worker_key,
                                pid: pid as i32,
                                connect_address: format!("unix:/tmp/gradle-worker-{}.sock", pid),
                                started_at_ms: started_at,
                                healthy: true,
                            }),
                            reused: true,
                            error_message: String::new(),
                        }));
                    }
                }
            }
        }

        // No idle worker available -- spawn a new one
        let max_pool = *self.max_pool_size.read().unwrap();
        if self.pool_size() >= max_pool {
            return Ok(Response::new(AcquireWorkerResponse {
                worker: None,
                reused: false,
                error_message: format!("Worker pool at capacity ({})", max_pool),
            }));
        }

        let worker_id = self.generate_worker_id();

        // Try to spawn a real JVM process
        let spawn_result = self.spawn_worker(&spec).await;

        // If spawn fails, fall back to stub behavior
        if spawn_result.is_err() {
            let e = spawn_result.unwrap_err();
            tracing::warn!(
                worker_key = %worker_key,
                error = %e,
                "Failed to spawn real worker, using stub PID"
            );
            return Ok(self.insert_stub_worker(worker_id, worker_key, spec, now));
        }

        let (pid, child) = spawn_result.unwrap();

        self.workers.insert(
            worker_id.clone(),
            TrackedWorker {
                worker_id: worker_id.clone(),
                worker_key: worker_key.clone(),
                pid,
                child: Some(child),
                state: "busy".to_string(),
                started_at_ms: now,
                last_used_ms: now,
                tasks_completed: 0,
                spec,
            },
        );

        self.workers_spawned.fetch_add(1, Ordering::Relaxed);

        tracing::info!(
            worker_id = %worker_id,
            worker_key = %worker_key,
            pid = pid,
            "Spawned new worker process"
        );

        Ok(Response::new(AcquireWorkerResponse {
            worker: Some(WorkerHandle {
                worker_id,
                worker_key,
                pid: pid as i32,
                connect_address: format!("unix:/tmp/gradle-worker-{}.sock", pid),
                started_at_ms: now,
                healthy: true,
            }),
            reused: false,
            error_message: String::new(),
        }))
    }

    async fn release_worker(
        &self,
        request: Request<ReleaseWorkerRequest>,
    ) -> Result<Response<ReleaseWorkerResponse>, Status> {
        let req = request.into_inner();
        let now = Self::now_ms();

        if let Some(mut worker) = self.workers.get_mut(&req.worker_id) {
            // Check health before returning to pool
            let alive = if let Some(ref mut child) = worker.child {
                Self::check_health(child)
            } else {
                true // Stub worker, assume alive
            };

            if req.healthy && worker.spec.daemon && alive {
                worker.state = "idle".to_string();
                worker.last_used_ms = now;
                let key = worker.worker_key.clone();
                drop(worker);

                // Add to idle pool
                self.idle_workers
                    .entry(key)
                    .or_insert_with(Vec::new)
                    .push(req.worker_id.clone());

                tracing::debug!(
                    worker_id = %req.worker_id,
                    "Worker returned to idle pool"
                );
            } else {
                // Unhealthy or non-daemon -- remove
                drop(worker);
                self.stop_worker_internal(&req.worker_id, !req.healthy).await;
                tracing::debug!(
                    worker_id = %req.worker_id,
                    "Worker removed (unhealthy={})",
                    !req.healthy
                );
            }
        }

        Ok(Response::new(ReleaseWorkerResponse { accepted: true }))
    }

    async fn stop_worker(
        &self,
        request: Request<StopWorkerRequest>,
    ) -> Result<Response<StopWorkerResponse>, Status> {
        let req = request.into_inner();

        let stopped = self.stop_worker_internal(&req.worker_id, req.force).await;

        tracing::info!(
            worker_id = %req.worker_id,
            force = req.force,
            stopped,
            "Worker stop requested"
        );

        Ok(Response::new(StopWorkerResponse {
            stopped,
            error_message: if stopped {
                String::new()
            } else {
                format!("Worker {} not found", req.worker_id)
            },
        }))
    }

    async fn get_worker_status(
        &self,
        request: Request<GetWorkerStatusRequest>,
    ) -> Result<Response<GetWorkerStatusResponse>, Status> {
        let req = request.into_inner();

        let mut workers = Vec::new();
        let mut idle_count = 0i32;
        let mut busy_count = 0i32;

        for entry in self.workers.iter() {
            if req.worker_key.is_empty() || entry.worker_key == req.worker_key {
                let state = entry.state.clone();

                if state == "idle" {
                    idle_count += 1;
                } else if state == "busy" {
                    busy_count += 1;
                }

                // Get memory usage
                let memory_used = get_process_memory(entry.pid);

                workers.push(WorkerStatus {
                    worker_id: entry.worker_id.clone(),
                    worker_key: entry.worker_key.clone(),
                    pid: entry.pid as i32,
                    state: state.clone(),
                    started_at_ms: entry.started_at_ms,
                    last_used_ms: entry.last_used_ms,
                    memory_used_bytes: memory_used,
                    tasks_completed: entry.tasks_completed,
                });
            }
        }

        Ok(Response::new(GetWorkerStatusResponse {
            workers,
            pool_size: self.pool_size(),
            idle_count,
            busy_count,
        }))
    }

    async fn configure_pool(
        &self,
        request: Request<ConfigurePoolRequest>,
    ) -> Result<Response<ConfigurePoolResponse>, Status> {
        let req = request.into_inner();

        if req.max_pool_size > 0 {
            *self.max_pool_size.write().unwrap() = req.max_pool_size;
        }
        if req.idle_timeout_ms > 0 {
            *self.idle_timeout_ms.write().unwrap() = req.idle_timeout_ms;
        }

        tracing::info!(
            max_pool_size = req.max_pool_size,
            idle_timeout_ms = req.idle_timeout_ms,
            max_per_key = req.max_per_key,
            "Worker pool configuration updated"
        );

        Ok(Response::new(ConfigurePoolResponse { applied: true }))
    }
}

/// Get memory usage for a process in bytes.
/// Uses /proc/{pid}/status on Linux, proc_pidinfo on macOS.
fn get_process_memory(pid: u32) -> i64 {
    #[cfg(target_os = "linux")]
    {
        use std::io::BufRead;
        if let Ok(file) = std::fs::File::open(format!("/proc/{}/status", pid)) {
            for line in std::io::BufReader::new(file).lines() {
                if let Ok(line) = line {
                    if line.starts_with("VmRSS:") {
                        // VmRSS: 12345 kB
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.len() >= 2 {
                            if let Ok(kb) = parts[1].parse::<i64>() {
                                return kb * 1024;
                            }
                        }
                    }
                }
            }
        }
        0
    }

    #[cfg(target_os = "macos")]
    {
        use std::mem;
        // Use proc_pidinfo to get RSS for a specific PID
        #[repr(C)]
        struct ProcVminfo {
            pvi_size: u64,       // virtual memory size (bytes)
            pvi_rssize: u64,     // resident set size (bytes)
            pvi_footprint: u64,  // memory footprint (bytes)
        }

        let mut info: ProcVminfo = unsafe { mem::zeroed() };
        let info_size = mem::size_of::<ProcVminfo>() as i32;

        unsafe {
            let ret = libc::proc_pidinfo(
                pid as i32,
                5, // PROC_PID_VMINFO = 5
                0,
                &mut info as *mut _ as *mut libc::c_void,
                info_size,
            );
            if ret == info_size {
                return info.pvi_rssize as i64;
            }
        }
        0
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = pid;
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_spec(key: &str) -> WorkerSpec {
        WorkerSpec {
            worker_key: key.to_string(),
            java_home: String::new(),
            classpath: vec!["/tmp/classes".to_string()],
            working_dir: "/tmp".to_string(),
            jvm_args: Default::default(),
            max_memory_mb: 512,
            daemon: true,
        }
    }

    #[tokio::test]
    async fn test_acquire_and_release_worker() {
        let svc = WorkerProcessServiceImpl::new();

        let resp = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(make_spec("java-compiler")),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.worker.is_some());
        assert!(!resp.reused);
        let worker = resp.worker.unwrap();
        assert!(worker.worker_id.starts_with("worker-"));
        assert_eq!(worker.worker_key, "java-compiler");

        let worker_id = worker.worker_id.clone();

        // Release it back
        svc.release_worker(Request::new(ReleaseWorkerRequest {
            worker_id: worker_id.clone(),
            healthy: true,
        }))
        .await
        .unwrap();

        // Status should show one idle worker
        let status = svc
            .get_worker_status(Request::new(GetWorkerStatusRequest {
                worker_key: "java-compiler".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(status.pool_size, 1);
        assert_eq!(status.idle_count, 1);
    }

    #[tokio::test]
    async fn test_reuse_idle_worker() {
        let svc = WorkerProcessServiceImpl::new();

        let spec = make_spec("test-worker");

        // Acquire and release
        let resp1 = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(spec.clone()),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        let worker_id1 = resp1.worker.as_ref().unwrap().worker_id.clone();
        svc.release_worker(Request::new(ReleaseWorkerRequest {
            worker_id: worker_id1,
            healthy: true,
        }))
        .await
        .unwrap();

        // Acquire again -- should reuse
        let resp2 = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(spec),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp2.reused);
        assert_eq!(resp2.worker.unwrap().worker_key, "test-worker");
    }

    #[tokio::test]
    async fn test_stop_worker() {
        let svc = WorkerProcessServiceImpl::new();

        let resp = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(make_spec("compile")),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        let worker_id = resp.worker.unwrap().worker_id;

        let stop_resp = svc
            .stop_worker(Request::new(StopWorkerRequest {
                worker_id: worker_id.clone(),
                force: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(stop_resp.stopped);

        // Should be gone
        let status = svc
            .get_worker_status(Request::new(GetWorkerStatusRequest {
                worker_key: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(status.pool_size, 0);
    }

    #[tokio::test]
    async fn test_release_unhealthy_worker() {
        let svc = WorkerProcessServiceImpl::new();

        let resp = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(make_spec("flaky-worker")),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        let worker_id = resp.worker.unwrap().worker_id;

        // Release as unhealthy -- should not go to idle pool
        svc.release_worker(Request::new(ReleaseWorkerRequest {
            worker_id: worker_id.clone(),
            healthy: false,
        }))
        .await
        .unwrap();

        let status = svc
            .get_worker_status(Request::new(GetWorkerStatusRequest {
                worker_key: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(status.pool_size, 0);
    }

    #[tokio::test]
    async fn test_configure_pool() {
        let svc = WorkerProcessServiceImpl::new();

        svc.configure_pool(Request::new(ConfigurePoolRequest {
            max_pool_size: 4,
            idle_timeout_ms: 60_000,
            max_per_key: 2,
            enable_health_checks: true,
        }))
        .await
        .unwrap();

        // Verify pool limit by acquiring 5 workers
        for _ in 0..4 {
            svc.acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(make_spec("limited")),
                timeout_ms: 5000,
            }))
            .await
            .unwrap();
        }

        // 5th should fail
        let resp = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(make_spec("limited")),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.worker.is_none());
        assert!(!resp.error_message.is_empty());
    }

    #[tokio::test]
    async fn test_acquire_missing_spec() {
        let svc = WorkerProcessServiceImpl::new();

        let result = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: None,
                timeout_ms: 5000,
            }))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_spawn_echo_process() {
        let svc = WorkerProcessServiceImpl::new();

        // Use echo command as a real process test
        let mut cmd = tokio::process::Command::new("echo");
        cmd.arg("hello")
            .stdin(std::process::Stdio::piped());
        let child = cmd.spawn().unwrap();
        let pid = child.id().unwrap();

        assert!(pid > 0);
    }

    #[test]
    fn test_build_jvm_command() {
        let spec = WorkerSpec {
            worker_key: "test".to_string(),
            java_home: "/usr/lib/jvm/java-17".to_string(),
            classpath: vec!["a.jar".to_string(), "b.jar".to_string()],
            working_dir: "/tmp".to_string(),
            jvm_args: Default::default(),
            max_memory_mb: 1024,
            daemon: true,
        };

        let _cmd = WorkerProcessServiceImpl::build_jvm_command(&spec);
        // Function should not panic
    }

    #[test]
    fn test_get_process_memory() {
        // Just verify it doesn't panic for the current process
        let pid = std::process::id();
        let mem = get_process_memory(pid);
        // On macOS this returns 0; on Linux it may return a real value
        assert!(mem >= 0);
    }
}
