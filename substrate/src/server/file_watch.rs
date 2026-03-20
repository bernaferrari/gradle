use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Instant;

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use crate::proto::{
    file_watch_service_server::FileWatchService, FileChangeEvent, GetWatchStatsRequest,
    GetWatchStatsResponse, PollChangesRequest, StartWatchingRequest, StartWatchingResponse,
    StopWatchingRequest, StopWatchingResponse,
};

/// An active file watch session.
#[allow(dead_code)]
struct WatchSession {
    root_path: String,
    include_patterns: Vec<String>,
    exclude_patterns: Vec<String>,
    start_time: Instant,
    files_watched: i64,
    changes_detected: AtomicI64,
    last_poll_ms: AtomicI64,
}

/// Rust-native file watch service.
/// Monitors file system changes for incremental builds.
///
/// In production, this would use the `notify` crate for OS-level
/// file system events (inotify on Linux, FSEvents on macOS, ReadDirectoryChangesW on Windows).
/// For now, it provides the gRPC interface with polling-based stubs.
pub struct FileWatchServiceImpl {
    watches: DashMap<String, WatchSession>,
    next_watch_id: AtomicI64,
}

impl FileWatchServiceImpl {
    pub fn new() -> Self {
        Self {
            watches: DashMap::new(),
            next_watch_id: AtomicI64::new(1),
        }
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    fn count_files(path: &str) -> i64 {
        let mut count = 0i64;
        let path = std::path::Path::new(path);
        if path.exists() {
            if path.is_dir() {
                if let Ok(entries) = std::fs::read_dir(path) {
                    for entry in entries.flatten() {
                        count += 1;
                        if entry.path().is_dir() {
                            if let Some(name) = entry.file_name().to_str() {
                                if name != "node_modules" && name != ".gradle" && !name.starts_with('.') {
                                    count += Self::count_files(&entry.path().to_string_lossy());
                                }
                            }
                        }
                    }
                }
            } else {
                count = 1;
            }
        }
        count
    }
}

#[tonic::async_trait]
impl FileWatchService for FileWatchServiceImpl {
    async fn start_watching(
        &self,
        request: Request<StartWatchingRequest>,
    ) -> Result<Response<StartWatchingResponse>, Status> {
        let req = request.into_inner();
        let root_path = req.root_path.clone();

        let watch_id = format!("watch-{}", self.next_watch_id.fetch_add(1, Ordering::Relaxed));
        let files_watched = Self::count_files(&req.root_path);

        self.watches.insert(
            watch_id.clone(),
            WatchSession {
                root_path: req.root_path,
                include_patterns: req.include_patterns,
                exclude_patterns: req.exclude_patterns,
                start_time: Instant::now(),
                files_watched,
                changes_detected: AtomicI64::new(0),
                last_poll_ms: AtomicI64::new(Self::now_ms()),
            },
        );

        tracing::info!(
            watch_id = %watch_id,
            root = %root_path,
            files = files_watched,
            "File watch started"
        );

        Ok(Response::new(StartWatchingResponse {
            watching: true,
            watch_id,
            files_watched,
        }))
    }

    async fn stop_watching(
        &self,
        request: Request<StopWatchingRequest>,
    ) -> Result<Response<StopWatchingResponse>, Status> {
        let req = request.into_inner();

        let stopped = self.watches.remove(&req.watch_id).is_some();

        tracing::info!(
            watch_id = %req.watch_id,
            stopped,
            "File watch stopped"
        );

        Ok(Response::new(StopWatchingResponse { stopped }))
    }

    type PollChangesStream = std::pin::Pin<Box<dyn tonic::codegen::tokio_stream::Stream<Item = Result<FileChangeEvent, Status>> + Send>>;

    async fn poll_changes(
        &self,
        request: Request<PollChangesRequest>,
    ) -> Result<Response<Self::PollChangesStream>, Status> {
        let req = request.into_inner();

        // In production, this would return actual file system events from the notify crate.
        // For now, return an empty stream.
        if let Some(session) = self.watches.get(&req.watch_id) {
            session.last_poll_ms.store(Self::now_ms(), Ordering::Relaxed);
        }

        let stream = tokio_stream::empty::<Result<FileChangeEvent, Status>>();

        Ok(Response::new(Box::pin(stream) as Self::PollChangesStream))
    }

    async fn get_watch_stats(
        &self,
        request: Request<GetWatchStatsRequest>,
    ) -> Result<Response<GetWatchStatsResponse>, Status> {
        let req = request.into_inner();

        if let Some(session) = self.watches.get(&req.watch_id) {
            let elapsed = session.start_time.elapsed().as_secs() as i64 * 1000;

            Ok(Response::new(GetWatchStatsResponse {
                files_watched: session.files_watched,
                changes_detected: session.changes_detected.load(Ordering::Relaxed),
                last_poll_time_ms: session.last_poll_ms.load(Ordering::Relaxed),
                watch_start_time_ms: Self::now_ms() - elapsed,
            }))
        } else {
            Ok(Response::new(GetWatchStatsResponse {
                files_watched: 0,
                changes_detected: 0,
                last_poll_time_ms: 0,
                watch_start_time_ms: 0,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_start_and_stop() {
        let svc = FileWatchServiceImpl::new();

        let resp = svc
            .start_watching(Request::new(StartWatchingRequest {
                root_path: "/tmp".to_string(),
                include_patterns: vec!["**/*.java".to_string()],
                exclude_patterns: vec!["**/generated/**".to_string()],
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.watching);
        assert!(!resp.watch_id.is_empty());

        let resp2 = svc
            .stop_watching(Request::new(StopWatchingRequest {
                watch_id: resp.watch_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp2.stopped);
    }

    #[tokio::test]
    async fn test_watch_stats() {
        let svc = FileWatchServiceImpl::new();

        let start = svc
            .start_watching(Request::new(StartWatchingRequest {
                root_path: "/tmp".to_string(),
                include_patterns: vec![],
                exclude_patterns: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        let stats = svc
            .get_watch_stats(Request::new(GetWatchStatsRequest {
                watch_id: start.watch_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(stats.files_watched >= 0);
        assert!(stats.watch_start_time_ms > 0);
    }

    #[tokio::test]
    async fn test_stop_nonexistent() {
        let svc = FileWatchServiceImpl::new();

        let resp = svc
            .stop_watching(Request::new(StopWatchingRequest {
                watch_id: "nonexistent".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.stopped);
    }
}
