use std::sync::atomic::{AtomicI64, Ordering};

use dashmap::DashMap;
use tonic::{Request, Response, Status};

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
/// In production, this would spawn actual JVM processes and manage
/// their lifecycle. For now, it tracks worker state and simulates
/// process management.
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
        let reused = if let Some(mut idle_list) = self.idle_workers.get_mut(&worker_key) {
            if let Some(worker_id) = idle_list.pop() {
                if let Some(mut worker) = self.workers.get_mut(&worker_id) {
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
            false
        } else {
            false
        };

        // No idle worker available — spawn a new one
        let max_pool = *self.max_pool_size.read().unwrap();
        if self.pool_size() >= max_pool {
            return Ok(Response::new(AcquireWorkerResponse {
                worker: None,
                reused: false,
                error_message: format!("Worker pool at capacity ({})", max_pool),
            }));
        }

        let worker_id = self.generate_worker_id();
        let pid = std::process::id(); // In production, this would be the spawned child PID
        let started_at = now;

        self.workers.insert(
            worker_id.clone(),
            TrackedWorker {
                worker_id: worker_id.clone(),
                worker_key: worker_key.clone(),
                pid,
                state: "busy".to_string(),
                started_at_ms: started_at,
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
                started_at_ms: started_at,
                healthy: true,
            }),
            reused,
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
            if req.healthy && worker.spec.daemon {
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
                // Unhealthy or non-daemon — remove
                drop(worker);
                self.workers.remove(&req.worker_id);
                self.workers_stopped.fetch_add(1, Ordering::Relaxed);
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

        if let Some((_key, worker)) = self.workers.remove(&req.worker_id) {
            // In production, send SIGTERM (or SIGKILL if force=true)
            // and wait for the process to exit.

            // Remove from idle pool if present
            if let Some(mut idle_list) = self.idle_workers.get_mut(&worker.worker_key) {
                idle_list.retain(|id| id != &req.worker_id);
            }

            self.workers_stopped.fetch_add(1, Ordering::Relaxed);

            tracing::info!(
                worker_id = %req.worker_id,
                force = req.force,
                "Stopped worker process"
            );

            Ok(Response::new(StopWorkerResponse {
                stopped: true,
                error_message: String::new(),
            }))
        } else {
            Ok(Response::new(StopWorkerResponse {
                stopped: false,
                error_message: format!("Worker {} not found", req.worker_id),
            }))
        }
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

                workers.push(WorkerStatus {
                    worker_id: entry.worker_id.clone(),
                    worker_key: entry.worker_key.clone(),
                    pid: entry.pid as i32,
                    state,
                    started_at_ms: entry.started_at_ms,
                    last_used_ms: entry.last_used_ms,
                    memory_used_bytes: 0, // In production, read from /proc or equivalent
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_spec(key: &str) -> WorkerSpec {
        WorkerSpec {
            worker_key: key.to_string(),
            java_home: "/usr/lib/jvm/java-17".to_string(),
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

        // Acquire again — should reuse
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

        // Release as unhealthy — should not go to idle pool
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
}
