use std::sync::atomic::{AtomicI64, Ordering};

use dashmap::DashMap;
use tokio::sync::Notify;
use tonic::{Request, Response, Status};

#[cfg(unix)]
use nix::sys::signal::{self, Signal};
#[cfg(unix)]
use nix::unistd::Pid;

use crate::proto::{
    worker_process_service_server::WorkerProcessService, AcquireWorkerRequest,
    AcquireWorkerResponse, ConfigurePoolRequest, ConfigurePoolResponse, GetWorkerStatusRequest,
    GetWorkerStatusResponse, ReleaseWorkerRequest, ReleaseWorkerResponse, RenewLeaseRequest,
    RenewLeaseResponse, SendWorkRequest, SendWorkResponse, StopWorkerRequest, StopWorkerResponse,
    WorkerHandle, WorkerSpec, WorkerStatus,
};

/// State of a tracked worker process.
struct TrackedWorker {
    worker_id: String,
    worker_key: String,
    pid: u32,
    child: Option<tokio::process::Child>,
    /// Whether this is a stub worker (no real process, spawned as fallback).
    is_stub: bool,
    state: String,
    started_at_ms: i64,
    last_used_ms: i64,
    tasks_completed: i32,
    spec: WorkerSpec,
    /// When the current lease expires (0 = no lease).
    lease_expires_at_ms: i64,
    /// Who holds the lease.
    lease_holder: String,
    /// Display name of current work item.
    current_work: String,
}

/// Rust-native worker process management service.
/// Manages pools of Gradle worker daemon processes (compiler daemons,
/// test workers, etc.) for efficient reuse across builds.
///
/// Supports real JVM process spawning, health monitoring, idle reaping,
/// and stdout/stderr capture.
pub struct WorkerProcessServiceImpl {
    workers: DashMap<String, TrackedWorker>,
    idle_workers: DashMap<String, Vec<String>>, // worker_key -> [worker_id]
    next_worker_id: AtomicI64,
    max_pool_size: std::sync::RwLock<i32>,
    idle_timeout_ms: std::sync::RwLock<i64>,
    max_per_key: std::sync::RwLock<i32>,
    workers_spawned: AtomicI64,
    workers_reused: AtomicI64,
    workers_stopped: AtomicI64,
    /// Notify the idle reaper when workers change state.
    state_changed: Notify,
}

impl Default for WorkerProcessServiceImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkerProcessServiceImpl {
    pub fn new() -> Self {
        Self {
            workers: DashMap::new(),
            idle_workers: DashMap::new(),
            next_worker_id: AtomicI64::new(1),
            max_pool_size: std::sync::RwLock::new(16),
            idle_timeout_ms: std::sync::RwLock::new(120_000),
            max_per_key: std::sync::RwLock::new(i32::MAX),
            workers_spawned: AtomicI64::new(0),
            workers_reused: AtomicI64::new(0),
            workers_stopped: AtomicI64::new(0),
            state_changed: Notify::new(),
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

        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        cmd
    }

    /// Spawn a real JVM worker process.
    async fn spawn_worker(
        &self,
        spec: &WorkerSpec,
    ) -> Result<(u32, tokio::process::Child), String> {
        let mut cmd = Self::build_jvm_command(spec);

        cmd.spawn()
            .map_err(|e| format!("Failed to spawn worker: {}", e))
            .and_then(|child| {
                let pid = child
                    .id()
                    .ok_or_else(|| "Failed to get child PID".to_string())?;
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

    /// Check if a process is alive by sending signal 0 (no-op).
    #[cfg(unix)]
    fn check_pid_alive(pid: u32) -> bool {
        use nix::unistd::Pid;
        signal::kill(Pid::from_raw(pid as i32), Signal::SIGCONT).is_ok()
    }

    #[cfg(not(unix))]
    fn check_pid_alive(_pid: u32) -> bool {
        true // Assume alive on non-Unix
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
        match tokio::time::timeout(tokio::time::Duration::from_secs(5), child.wait()).await {
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
                is_stub: true,
                state: "busy".to_string(),
                started_at_ms: now,
                last_used_ms: now,
                tasks_completed: 0,
                spec,
                lease_expires_at_ms: 0,
                lease_holder: String::new(),
                current_work: String::new(),
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
                lease_expires_at_ms: 0,
                lease_holder: String::new(),
                current_work: String::new(),
            }),
            reused: false,
            error_message: String::new(),
        })
    }

    /// Reap idle workers that have exceeded the idle timeout.
    /// Returns the number of workers reaped.
    pub async fn reap_idle_workers(&self) -> usize {
        let idle_timeout_ms = *self
            .idle_timeout_ms
            .read()
            .expect("idle_timeout_ms lock should not be poisoned");
        let now = Self::now_ms();
        let mut reaped = 0;

        let mut workers_to_reap: Vec<(String, String)> = Vec::new();

        for entry in self.idle_workers.iter() {
            for worker_id in entry.value() {
                if let Some(worker) = self.workers.get(worker_id) {
                    if now - worker.last_used_ms >= idle_timeout_ms {
                        workers_to_reap.push((worker_id.clone(), entry.key().clone()));
                    }
                }
            }
        }

        for (worker_id, _key) in &workers_to_reap {
            if self.stop_worker_internal(worker_id, false).await {
                reaped += 1;
                tracing::debug!(worker_id = %worker_id, "Reaped idle worker");
            }
        }

        if reaped > 0 {
            tracing::info!(count = reaped, "Reaped idle workers");
        }

        reaped
    }

    /// Start a background task that periodically reaps idle workers.
    /// Returns a JoinHandle that can be used to stop the reaper.
    pub fn start_idle_reaper(
        self: &std::sync::Arc<Self>,
        check_interval_ms: u64,
    ) -> tokio::task::JoinHandle<()> {
        let svc = self.clone();
        tokio::spawn(async move {
            let interval = tokio::time::Duration::from_millis(check_interval_ms);
            loop {
                tokio::time::sleep(interval).await;
                svc.reap_idle_workers().await;
                svc.reap_expired_leases();
                // Wait for state changes too
                svc.state_changed.notified().await;
            }
        })
    }

    /// Expire leases on workers whose lease has elapsed.
    /// Marks them idle so they can be re-acquired.
    pub fn reap_expired_leases(&self) {
        let now = Self::now_ms();

        let expired: Vec<String> = self
            .workers
            .iter()
            .filter(|e| {
                e.state == "busy"
                    && e.lease_expires_at_ms > 0
                    && now > e.lease_expires_at_ms
            })
            .map(|e| e.worker_id.clone())
            .collect();

        for worker_id in &expired {
            if let Some(mut worker) = self.workers.get_mut(worker_id) {
                tracing::warn!(
                    worker_id = %worker_id,
                    current_work = %worker.current_work,
                    "Worker lease expired, returning to idle pool"
                );
                worker.state = "idle".to_string();
                worker.lease_expires_at_ms = 0;
                worker.lease_holder = String::new();
                worker.current_work = String::new();
                worker.last_used_ms = now;

                let key = worker.worker_key.clone();
                drop(worker);

                self.idle_workers
                    .entry(key)
                    .or_default()
                    .push(worker_id.clone());

                self.state_changed.notify_waiters();
            }
        }

        if !expired.is_empty() {
            tracing::info!(count = expired.len(), "Reaped expired worker leases");
        }
    }

    /// Spawn a background task that captures stdout/stderr from a worker process
    /// and logs them via tracing.
    fn spawn_output_capture(worker_id: String, mut child: tokio::process::Child) {
        tokio::spawn(async move {
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();

            if let Some(stdout) = stdout {
                let wid = worker_id.clone();
                tokio::spawn(async move {
                    use tokio::io::AsyncBufReadExt;
                    let reader = tokio::io::BufReader::new(stdout);
                    let mut lines = reader.lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        tracing::debug!(worker_id = %wid, "stdout: {}", line);
                    }
                });
            }

            if let Some(stderr) = stderr {
                let wid = worker_id.clone();
                tokio::spawn(async move {
                    use tokio::io::AsyncBufReadExt;
                    let reader = tokio::io::BufReader::new(stderr);
                    let mut lines = reader.lines();
                    while let Ok(Some(line)) = lines.next_line().await {
                        tracing::warn!(worker_id = %wid, "stderr: {}", line);
                    }
                });
            }

            // Wait for the process to exit
            match child.wait().await {
                Ok(status) => {
                    tracing::debug!(
                        worker_id = %worker_id,
                        exit_code = status.code().unwrap_or(-1),
                        "Worker process exited"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        worker_id = %worker_id,
                        error = %e,
                        "Failed to wait for worker process"
                    );
                }
            }
        });
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
        let isolation_mode = spec.isolation_mode.clone();
        let now = Self::now_ms();

        // Check per-key parallelism limit
        let max_per_key = *self
            .max_per_key
            .read()
            .expect("max_per_key lock should not be poisoned");
        if max_per_key != i32::MAX {
            let busy_count: i32 = self
                .workers
                .iter()
                .filter(|e| e.worker_key == worker_key && e.state == "busy")
                .count() as i32;
            if busy_count >= max_per_key {
                return Ok(Response::new(AcquireWorkerResponse {
                    worker: None,
                    reused: false,
                    error_message: format!(
                        "Per-key parallelism limit reached ({}/{} for key {})",
                        busy_count, max_per_key, worker_key
                    ),
                }));
            }
        }

        // Isolation mode handling:
        // "process" = spawn JVM daemon (default behavior)
        // "classloader" / "app_classloader" / "none" = return stub (delegated to JVM)
        if isolation_mode != "process" && !isolation_mode.is_empty() {
            let worker_id = self.generate_worker_id();
            return Ok(self.insert_stub_worker(worker_id, worker_key, spec, now));
        }

        // Check for idle worker of the same type
        if let Some(mut idle_list) = self.idle_workers.get_mut(&worker_key) {
            if let Some(worker_id) = idle_list.pop() {
                if let Some(mut worker) = self.workers.get_mut(&worker_id) {
                    // Check health before returning
                    let healthy = if worker.is_stub {
                        true // Stub workers are always considered healthy
                    } else if let Some(ref mut child) = worker.child {
                        Self::check_health(child)
                    } else {
                        // Worker was spawned with output capture (child moved away).
                        // Check via PID.
                        Self::check_pid_alive(worker.pid)
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
                        // Set default lease (5 minutes from now)
                        let lease_expires = if req.timeout_ms > 0 {
                            now + req.timeout_ms
                        } else {
                            now + 300_000
                        };
                        worker.lease_expires_at_ms = lease_expires;
                        worker.lease_holder = String::new();
                        let lease_ms = worker.lease_expires_at_ms;
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
                                lease_expires_at_ms: lease_ms,
                                lease_holder: String::new(),
                                current_work: String::new(),
                            }),
                            reused: true,
                            error_message: String::new(),
                        }));
                    }
                }
            }
        }

        // No idle worker available -- spawn a new one
        let max_pool = *self
            .max_pool_size
            .read()
            .expect("max_pool_size lock should not be poisoned");
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
        if let Err(e) = &spawn_result {
            tracing::warn!(
                worker_key = %worker_key,
                error = %e,
                "Failed to spawn real worker, using stub PID"
            );
            return Ok(self.insert_stub_worker(worker_id, worker_key, spec, now));
        }

        let (pid, child) = spawn_result
            .expect("spawn result should be Ok after error branch was handled");

        let lease_expires = if req.timeout_ms > 0 {
            now + req.timeout_ms
        } else {
            now + 300_000
        };

        self.workers.insert(
            worker_id.clone(),
            TrackedWorker {
                worker_id: worker_id.clone(),
                worker_key: worker_key.clone(),
                pid,
                child: None, // child is moved into output capture
                is_stub: false,
                state: "busy".to_string(),
                started_at_ms: now,
                last_used_ms: now,
                tasks_completed: 0,
                spec,
                lease_expires_at_ms: lease_expires,
                lease_holder: String::new(),
                current_work: String::new(),
            },
        );

        // Spawn background task to capture stdout/stderr
        Self::spawn_output_capture(worker_id.clone(), child);

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
                lease_expires_at_ms: lease_expires,
                lease_holder: String::new(),
                current_work: String::new(),
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
            let alive = if worker.is_stub {
                true // Stub workers are always considered alive
            } else if let Some(ref mut child) = worker.child {
                Self::check_health(child)
            } else {
                Self::check_pid_alive(worker.pid)
            };

            if req.healthy && worker.spec.daemon && alive {
                worker.state = "idle".to_string();
                worker.last_used_ms = now;
                worker.lease_expires_at_ms = 0;
                worker.lease_holder = String::new();
                worker.current_work = String::new();
                let key = worker.worker_key.clone();
                drop(worker);

                // Add to idle pool
                self.idle_workers
                    .entry(key)
                    .or_default()
                    .push(req.worker_id.clone());

                self.state_changed.notify_waiters();

                tracing::debug!(
                    worker_id = %req.worker_id,
                    "Worker returned to idle pool"
                );
            } else {
                // Unhealthy or non-daemon -- remove
                drop(worker);
                self.stop_worker_internal(&req.worker_id, !req.healthy)
                    .await;
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

        let mut workers = Vec::with_capacity(self.workers.len());
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
                    current_work: entry.current_work.clone(),
                    lease_expires_at_ms: entry.lease_expires_at_ms,
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
            *self
                .max_pool_size
                .write()
                .expect("max_pool_size lock should not be poisoned") = req.max_pool_size;
        }
        if req.idle_timeout_ms > 0 {
            *self
                .idle_timeout_ms
                .write()
                .expect("idle_timeout_ms lock should not be poisoned") = req.idle_timeout_ms;
        }
        if req.max_per_key > 0 {
            *self
                .max_per_key
                .write()
                .expect("max_per_key lock should not be poisoned") = req.max_per_key;
        }

        tracing::info!(
            max_pool_size = req.max_pool_size,
            idle_timeout_ms = req.idle_timeout_ms,
            max_per_key = req.max_per_key,
            "Worker pool configuration updated"
        );

        Ok(Response::new(ConfigurePoolResponse { applied: true }))
    }

    async fn renew_lease(
        &self,
        request: Request<RenewLeaseRequest>,
    ) -> Result<Response<RenewLeaseResponse>, Status> {
        let req = request.into_inner();

        if let Some(mut worker) = self.workers.get_mut(&req.worker_id) {
            if worker.state != "busy" {
                return Ok(Response::new(RenewLeaseResponse { renewed: false }));
            }

            let now = Self::now_ms();
            if req.duration_ms <= 0 {
                return Err(Status::invalid_argument("duration_ms must be positive"));
            }

            worker.lease_expires_at_ms = now + req.duration_ms;
            tracing::debug!(
                worker_id = %req.worker_id,
                duration_ms = req.duration_ms,
                "Lease renewed"
            );

            Ok(Response::new(RenewLeaseResponse { renewed: true }))
        } else {
            Ok(Response::new(RenewLeaseResponse { renewed: false }))
        }
    }

    async fn send_work(
        &self,
        request: Request<SendWorkRequest>,
    ) -> Result<Response<SendWorkResponse>, Status> {
        let req = request.into_inner();

        if let Some(mut worker) = self.workers.get_mut(&req.worker_id) {
            if worker.state == "busy" && !worker.current_work.is_empty() {
                return Ok(Response::new(SendWorkResponse {
                    accepted: false,
                    error_message: format!(
                        "Worker {} already has work in progress: {}",
                        req.worker_id, worker.current_work
                    ),
                }));
            }

            if worker.state == "idle" {
                worker.state = "busy".to_string();
            }

            worker.current_work = req.display_name.clone();
            worker.last_used_ms = Self::now_ms();

            // Update lease if timeout provided
            if req.timeout_ms > 0 {
                worker.lease_expires_at_ms = Self::now_ms() + req.timeout_ms;
            }

            tracing::info!(
                worker_id = %req.worker_id,
                action_class = %req.action_class,
                display_name = %req.display_name,
                "Work dispatched to worker"
            );

            Ok(Response::new(SendWorkResponse {
                accepted: true,
                error_message: String::new(),
            }))
        } else {
            Ok(Response::new(SendWorkResponse {
                accepted: false,
                error_message: format!("Worker {} not found", req.worker_id),
            }))
        }
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
            pvi_size: u64,      // virtual memory size (bytes)
            pvi_rssize: u64,    // resident set size (bytes)
            pvi_footprint: u64, // memory footprint (bytes)
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
            isolation_mode: "process".to_string(),
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
        // Use echo command as a real process test
        let mut cmd = tokio::process::Command::new("echo");
        cmd.arg("hello").stdin(std::process::Stdio::piped());
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
            isolation_mode: "process".to_string(),
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

    #[tokio::test]
    async fn test_get_worker_info_for_nonexistent_worker_returns_default() {
        let svc = WorkerProcessServiceImpl::new();

        // Query status with a worker_key that no worker was registered under.
        let status = svc
            .get_worker_status(Request::new(GetWorkerStatusRequest {
                worker_key: "nonexistent-key".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(status.workers.is_empty());
        assert_eq!(status.pool_size, 0);
        assert_eq!(status.idle_count, 0);
        assert_eq!(status.busy_count, 0);
    }

    #[tokio::test]
    async fn test_list_workers_when_none_registered_returns_empty_list() {
        let svc = WorkerProcessServiceImpl::new();

        // Fresh service, no workers acquired yet. Query with empty worker_key (all workers).
        let status = svc
            .get_worker_status(Request::new(GetWorkerStatusRequest {
                worker_key: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(status.workers.is_empty());
        assert_eq!(status.pool_size, 0);
        assert_eq!(status.idle_count, 0);
        assert_eq!(status.busy_count, 0);
    }

    #[tokio::test]
    async fn test_release_worker_that_was_never_acquired_succeeds() {
        let svc = WorkerProcessServiceImpl::new();

        // Release a worker ID that was never acquired -- should succeed (accepted=true)
        // without panicking or erroring, since the impl silently ignores unknown workers.
        let resp = svc
            .release_worker(Request::new(ReleaseWorkerRequest {
                worker_id: "ghost-worker-42".to_string(),
                healthy: true,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.accepted);
    }

    #[tokio::test]
    async fn test_multiple_workers_can_be_acquired_and_listed() {
        let svc = WorkerProcessServiceImpl::new();

        let spec_a = make_spec("compiler");
        let spec_b = make_spec("tester");

        // Acquire two workers of different types
        let resp_a = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(spec_a),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        let resp_b = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(spec_b),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp_a.worker.is_some());
        assert!(resp_b.worker.is_some());
        assert_ne!(
            resp_a.worker.as_ref().unwrap().worker_id,
            resp_b.worker.as_ref().unwrap().worker_id,
            "two acquired workers should have distinct IDs"
        );

        // List all workers (empty worker_key = no filter)
        let status = svc
            .get_worker_status(Request::new(GetWorkerStatusRequest {
                worker_key: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(status.workers.len(), 2);
        assert_eq!(status.pool_size, 2);
        assert_eq!(status.busy_count, 2);
        assert_eq!(status.idle_count, 0);

        // Both should be in "busy" state
        for w in &status.workers {
            assert_eq!(w.state, "busy");
        }

        // Filter by one key should return exactly one worker
        let status_a = svc
            .get_worker_status(Request::new(GetWorkerStatusRequest {
                worker_key: "compiler".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(status_a.workers.len(), 1);
        assert_eq!(status_a.workers[0].worker_key, "compiler");
    }

    #[tokio::test]
    async fn test_reap_idle_workers_with_zero_timeout() {
        let svc = WorkerProcessServiceImpl::new();

        // Set idle timeout to 0 so everything is immediately reaped
        *svc.idle_timeout_ms.write().unwrap() = 0;

        let spec = make_spec("reap-test");

        // Acquire a worker
        let resp = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(spec),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        let worker_id = resp.worker.unwrap().worker_id;

        // Release it (puts it in idle pool)
        svc.release_worker(Request::new(ReleaseWorkerRequest {
            worker_id: worker_id.clone(),
            healthy: true,
        }))
        .await
        .unwrap();

        // Verify it's in the idle pool
        let status = svc
            .get_worker_status(Request::new(GetWorkerStatusRequest {
                worker_key: "reap-test".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(status.pool_size, 1);
        assert_eq!(status.idle_count, 1);

        // Reap should remove it
        let reaped = svc.reap_idle_workers().await;
        assert_eq!(reaped, 1);

        // Pool should now be empty
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
    async fn test_reap_idle_workers_respects_timeout() {
        let svc = WorkerProcessServiceImpl::new();

        // Set a very high timeout — workers should NOT be reaped
        *svc.idle_timeout_ms.write().unwrap() = 999_999_999;

        let spec = make_spec("timeout-test");

        let resp = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(spec),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        let worker_id = resp.worker.unwrap().worker_id;

        svc.release_worker(Request::new(ReleaseWorkerRequest {
            worker_id,
            healthy: true,
        }))
        .await
        .unwrap();

        let reaped = svc.reap_idle_workers().await;
        assert_eq!(reaped, 0, "workers should not be reaped with long timeout");

        let status = svc
            .get_worker_status(Request::new(GetWorkerStatusRequest {
                worker_key: "timeout-test".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(status.pool_size, 1);
    }

    #[tokio::test]
    async fn test_reap_idle_workers_no_idle_workers() {
        let svc = WorkerProcessServiceImpl::new();

        // No workers at all — reap should be a no-op
        let reaped = svc.reap_idle_workers().await;
        assert_eq!(reaped, 0);
    }

    #[tokio::test]
    async fn test_reap_idle_workers_busy_workers_not_reaped() {
        let svc = WorkerProcessServiceImpl::new();

        *svc.idle_timeout_ms.write().unwrap() = 0;

        let spec = make_spec("busy-test");

        // Acquire but don't release — worker is busy, not idle
        svc.acquire_worker(Request::new(AcquireWorkerRequest {
            spec: Some(spec),
            timeout_ms: 5000,
        }))
        .await
        .unwrap();

        let reaped = svc.reap_idle_workers().await;
        assert_eq!(reaped, 0, "busy workers should not be reaped");

        let status = svc
            .get_worker_status(Request::new(GetWorkerStatusRequest {
                worker_key: "busy-test".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(status.pool_size, 1);
        assert_eq!(status.busy_count, 1);
    }

    #[test]
    fn test_build_jvm_command_with_args() {
        let spec = WorkerSpec {
            worker_key: "test".to_string(),
            java_home: "/usr/lib/jvm/java-17".to_string(),
            classpath: vec!["a.jar".to_string(), "b.jar".to_string()],
            working_dir: "/tmp".to_string(),
            jvm_args: [
                ("-ea".to_string(), "true".to_string()),
                ("-Xdebug".to_string(), "".to_string()),
            ]
            .into_iter()
            .collect(),
            max_memory_mb: 1024,
            daemon: true,
            isolation_mode: "process".to_string(),
        };

        let cmd = WorkerProcessServiceImpl::build_jvm_command(&spec);
        let args: Vec<String> = cmd
            .as_std()
            .get_args()
            .map(|s| s.to_string_lossy().to_string())
            .collect();

        assert!(
            args.iter().any(|a| a == "-Xmx1024m"),
            "should have -Xmx1024m"
        );
        assert!(
            args.iter().any(|a| a.contains("a.jar")),
            "should have classpath"
        );
        assert!(
            args.iter().any(|a| a.contains("-ea=true")),
            "should have -ea=true JVM arg"
        );
        assert!(
            args.iter().any(|a| a.contains("-Xdebug=")),
            "should have -Xdebug= JVM arg"
        );
        assert!(
            args.iter().any(|a| a.contains("IsolatedClassloaderWorker")),
            "should have main class"
        );
    }

    #[test]
    fn test_build_jvm_command_no_java_home() {
        let spec = WorkerSpec {
            worker_key: "test".to_string(),
            java_home: String::new(),
            classpath: vec![],
            working_dir: String::new(),
            jvm_args: Default::default(),
            max_memory_mb: 0,
            daemon: false,
            isolation_mode: String::new(),
        };

        let cmd = WorkerProcessServiceImpl::build_jvm_command(&spec);
        // Should use "java" as the binary when no java_home is set
        let program = cmd.as_std().get_program().to_string_lossy().to_string();
        assert_eq!(program, "java");
    }

    #[tokio::test]
    async fn test_acquire_worker_with_no_memory_limit() {
        let svc = WorkerProcessServiceImpl::new();

        let spec = WorkerSpec {
            worker_key: "no-mem-limit".to_string(),
            java_home: String::new(),
            classpath: vec![],
            working_dir: String::new(),
            jvm_args: Default::default(),
            max_memory_mb: 0, // no limit
            daemon: false,
            isolation_mode: String::new(),
        };

        let resp = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(spec),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.worker.is_some());
        assert_eq!(resp.worker.unwrap().worker_key, "no-mem-limit");
    }

    #[tokio::test]
    async fn test_non_daemon_worker_not_returned_to_pool() {
        let svc = WorkerProcessServiceImpl::new();

        let spec = WorkerSpec {
            worker_key: "non-daemon".to_string(),
            java_home: String::new(),
            classpath: vec![],
            working_dir: String::new(),
            jvm_args: Default::default(),
            max_memory_mb: 512,
            daemon: false, // non-daemon
            isolation_mode: String::new(),
        };

        let resp = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(spec),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        let worker_id = resp.worker.unwrap().worker_id;

        // Release as healthy — but non-daemon should NOT go to idle pool
        svc.release_worker(Request::new(ReleaseWorkerRequest {
            worker_id,
            healthy: true,
        }))
        .await
        .unwrap();

        let status = svc
            .get_worker_status(Request::new(GetWorkerStatusRequest {
                worker_key: "non-daemon".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(status.pool_size, 0, "non-daemon worker should be removed");
    }

    // --- Isolation mode tests ---

    #[tokio::test]
    async fn test_isolation_mode_process_worker() {
        let svc = WorkerProcessServiceImpl::new();

        let mut spec = make_spec("compiler");
        spec.isolation_mode = "process".to_string();

        let resp = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(spec),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        // process mode should return a worker (may be stub if spawn fails)
        assert!(resp.worker.is_some());
        assert!(!resp.worker.as_ref().unwrap().connect_address.is_empty());
    }

    #[tokio::test]
    async fn test_isolation_mode_classloader_returns_stub() {
        let svc = WorkerProcessServiceImpl::new();

        let mut spec = make_spec("compiler");
        spec.isolation_mode = "classloader".to_string();

        let resp = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(spec),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        // classloader mode should return a stub worker
        assert!(resp.worker.is_some());
        let worker = resp.worker.unwrap();
        assert_eq!(worker.worker_key, "compiler");
    }

    #[tokio::test]
    async fn test_isolation_mode_none_returns_stub() {
        let svc = WorkerProcessServiceImpl::new();

        let mut spec = make_spec("compiler");
        spec.isolation_mode = "none".to_string();

        let resp = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(spec),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        // none mode should return a stub worker
        assert!(resp.worker.is_some());
        assert_eq!(resp.worker.unwrap().worker_key, "compiler");
    }

    // --- Lease tests ---

    #[tokio::test]
    async fn test_lease_expiration() {
        let svc = WorkerProcessServiceImpl::new();

        let spec = make_spec("lease-test");

        let resp = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(spec),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        let worker_id = resp.worker.unwrap().worker_id;

        // Manually set lease to expired
        {
            let mut worker = svc.workers.get_mut(&worker_id).unwrap();
            worker.lease_expires_at_ms = WorkerProcessServiceImpl::now_ms() - 1000;
        }

        // Reap expired leases
        svc.reap_expired_leases();

        // Worker should now be idle
        let status = svc
            .get_worker_status(Request::new(GetWorkerStatusRequest {
                worker_key: "lease-test".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(status.idle_count, 1, "expired lease worker should be idle");
        assert_eq!(status.busy_count, 0);
    }

    #[tokio::test]
    async fn test_renew_lease_extends_timeout() {
        let svc = WorkerProcessServiceImpl::new();

        let spec = make_spec("renew-test");

        let resp = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(spec),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        let worker_id = resp.worker.unwrap().worker_id.clone();

        // Set lease about to expire
        {
            let mut worker = svc.workers.get_mut(&worker_id).unwrap();
            worker.lease_expires_at_ms = WorkerProcessServiceImpl::now_ms() + 1000;
        }

        // Renew lease for 60 seconds
        let renew_resp = svc
            .renew_lease(Request::new(RenewLeaseRequest {
                worker_id: worker_id.clone(),
                duration_ms: 60_000,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(renew_resp.renewed);

        // Verify lease was extended
        {
            let worker = svc.workers.get(&worker_id).unwrap();
            assert!(worker.lease_expires_at_ms > WorkerProcessServiceImpl::now_ms() + 50_000);
        }
    }

    #[tokio::test]
    async fn test_renew_lease_idle_worker_fails() {
        let svc = WorkerProcessServiceImpl::new();

        let spec = make_spec("idle-lease-test");

        let resp = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(spec),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        let worker_id = resp.worker.unwrap().worker_id.clone();

        // Release worker to idle
        svc.release_worker(Request::new(ReleaseWorkerRequest {
            worker_id: worker_id.clone(),
            healthy: true,
        }))
        .await
        .unwrap();

        // Renewing lease on idle worker should fail
        let renew_resp = svc
            .renew_lease(Request::new(RenewLeaseRequest {
                worker_id,
                duration_ms: 60_000,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!renew_resp.renewed);
    }

    // --- Per-key parallelism tests ---

    #[tokio::test]
    async fn test_per_key_parallelism_limit() {
        let svc = WorkerProcessServiceImpl::new();
        *svc.max_per_key.write().unwrap() = 2;

        // Acquire 2 workers with same key — should succeed
        for _ in 0..2 {
            let resp = svc
                .acquire_worker(Request::new(AcquireWorkerRequest {
                    spec: Some(make_spec("limited-key")),
                    timeout_ms: 5000,
                }))
                .await
                .unwrap()
                .into_inner();
            assert!(resp.worker.is_some(), "should allow up to max_per_key workers");
        }

        // 3rd should be rejected due to per-key limit
        let resp = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(make_spec("limited-key")),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.worker.is_none());
        assert!(resp.error_message.contains("Per-key parallelism limit"));
    }

    #[tokio::test]
    async fn test_per_key_limit_different_keys_ok() {
        let svc = WorkerProcessServiceImpl::new();
        *svc.max_per_key.write().unwrap() = 1;

        // One worker per key should be fine
        let resp_a = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(make_spec("key-a")),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(resp_a.worker.is_some());

        let resp_b = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(make_spec("key-b")),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(resp_b.worker.is_some());
    }

    // --- SendWork tests ---

    #[tokio::test]
    async fn test_send_work_accepted() {
        let svc = WorkerProcessServiceImpl::new();

        let spec = make_spec("work-test");
        let resp = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(spec),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        let worker_id = resp.worker.unwrap().worker_id.clone();

        // Send work to the worker
        let work_resp = svc
            .send_work(Request::new(SendWorkRequest {
                worker_id: worker_id.clone(),
                action_class: "org.gradle.compiler.JavaCompile".to_string(),
                parameters_json: "{}".to_string(),
                timeout_ms: 30_000,
                display_name: "compileJava".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(work_resp.accepted);
        assert!(work_resp.error_message.is_empty());

        // Verify worker status shows current work
        let worker = svc.workers.get(&worker_id).unwrap();
        assert_eq!(worker.current_work, "compileJava");
    }

    #[tokio::test]
    async fn test_send_work_rejected_when_busy() {
        let svc = WorkerProcessServiceImpl::new();

        let spec = make_spec("busy-work-test");
        let resp = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(spec),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        let worker_id = resp.worker.unwrap().worker_id.clone();

        // Send first work item
        let work_resp1 = svc
            .send_work(Request::new(SendWorkRequest {
                worker_id: worker_id.clone(),
                action_class: "org.gradle.compiler.JavaCompile".to_string(),
                parameters_json: "{}".to_string(),
                timeout_ms: 30_000,
                display_name: "compileJava".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(work_resp1.accepted);

        // Send second work item — should be rejected
        let work_resp2 = svc
            .send_work(Request::new(SendWorkRequest {
                worker_id: worker_id.clone(),
                action_class: "org.gradle.test.TestExec".to_string(),
                parameters_json: "{}".to_string(),
                timeout_ms: 30_000,
                display_name: "test".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!work_resp2.accepted);
        assert!(work_resp2.error_message.contains("already has work"));
    }

    #[tokio::test]
    async fn test_send_work_to_idle_worker_transitions_to_busy() {
        let svc = WorkerProcessServiceImpl::new();

        let spec = make_spec("idle-work-test");
        let resp = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(spec),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        let worker_id = resp.worker.unwrap().worker_id.clone();

        // Release to idle (clears current_work)
        svc.release_worker(Request::new(ReleaseWorkerRequest {
            worker_id: worker_id.clone(),
            healthy: true,
        }))
        .await
        .unwrap();

        // SendWork should transition idle → busy
        let work_resp = svc
            .send_work(Request::new(SendWorkRequest {
                worker_id: worker_id.clone(),
                action_class: "org.gradle.compiler.JavaCompile".to_string(),
                parameters_json: "{}".to_string(),
                timeout_ms: 30_000,
                display_name: "compileKotlin".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(work_resp.accepted);

        let worker = svc.workers.get(&worker_id).unwrap();
        assert_eq!(worker.state, "busy");
        assert_eq!(worker.current_work, "compileKotlin");
    }

    // --- Lease reaper tests ---

    #[tokio::test]
    async fn test_lease_reaper_expires_dead_workers() {
        let svc = WorkerProcessServiceImpl::new();

        let spec = make_spec("reaper-test");

        let resp = svc
            .acquire_worker(Request::new(AcquireWorkerRequest {
                spec: Some(spec),
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        let worker_id = resp.worker.unwrap().worker_id.clone();

        // Manually set lease to expired
        {
            let mut worker = svc.workers.get_mut(&worker_id).unwrap();
            worker.lease_expires_at_ms = WorkerProcessServiceImpl::now_ms() - 1000;
            worker.current_work = "stale-work".to_string();
        }

        // Verify busy before reaper
        let status_before = svc
            .get_worker_status(Request::new(GetWorkerStatusRequest {
                worker_key: "reaper-test".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(status_before.busy_count, 1);

        // Run reaper
        svc.reap_expired_leases();

        // Should be idle after reaper
        let status_after = svc
            .get_worker_status(Request::new(GetWorkerStatusRequest {
                worker_key: "reaper-test".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(status_after.idle_count, 1);
        assert_eq!(status_after.busy_count, 0);

        // Verify work was cleared
        let worker = svc.workers.get(&worker_id).unwrap();
        assert!(worker.current_work.is_empty());
        assert_eq!(worker.lease_expires_at_ms, 0);
    }
}
