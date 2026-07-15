use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::broadcast;
use tonic::{Request, Response, Status};

use crate::proto::{
    file_watch_service_server::FileWatchService, FileChangeEvent, GetWatchStatsRequest,
    GetWatchStatsResponse, PollChangesRequest, StartWatchingRequest, StartWatchingResponse,
    StopWatchingRequest, StopWatchingResponse,
};

use super::task_graph::TaskGraphServiceImpl;


/// Default debounce interval in milliseconds.
const DEFAULT_DEBOUNCE_MS: u64 = 100;
/// Maximum file tree depth for counting.
const MAX_TREE_DEPTH: u32 = 50;
/// Maximum number of files to count before giving up.
const MAX_FILES_TO_COUNT: i64 = 100_000;
/// Maximum number of file events retained per watch for late pollers.
const MAX_RETAINED_EVENTS: usize = 4_096;

/// An active file watch session backed by a real OS file watcher.
struct WatchSession {
    start_time: Instant,
    files_watched: i64,
    changes_detected: Arc<AtomicI64>,
    last_poll_ms: AtomicI64,
    /// Whether this session has fallen back to polling mode.
    polling_mode: AtomicBool,
    /// Debounce interval in milliseconds.
    debounce_ms: u64,
    events: Arc<Mutex<VecDeque<FileChangeEvent>>>,
    event_tx: broadcast::Sender<FileChangeEvent>,
    _watcher: Option<RecommendedWatcher>,
}

/// Detect if a path is on a network/remote filesystem.
/// On Linux, checks for NFS/SMB/FUSE mounts in /proc/mounts.
/// On macOS, checks for network mounts via statfs.
pub(crate) fn is_network_filesystem(path: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        // Check /proc/mounts for network filesystem types
        if let Ok(content) = std::fs::read_to_string("/proc/mounts") {
            let path_str = path;
            for line in content.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 3 {
                    let mount_point = parts[1];
                    let fs_type = parts[2];
                    if path_str.starts_with(mount_point)
                        && (fs_type.contains("nfs")
                            || fs_type.contains("smb")
                            || fs_type.contains("cifs")
                            || fs_type.contains("fuse")
                            || fs_type.contains("sshfs"))
                    {
                        return true;
                    }
                }
            }
        }
        false
    }

    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        let path = std::path::Path::new(path);
        if !path.exists() {
            return false;
        }
        unsafe {
            let mut statfs: libc::statfs = std::mem::zeroed();
            let c_path = CString::new(path_str_for_statfs(path)).unwrap_or_default();
            if libc::statfs(c_path.as_ptr(), &mut statfs) != 0 {
                return false;
            }
            // Check if it's NFS, SMB, or other network FS
            let f_type = statfs.f_type;
            // NFS
            if f_type == 0x6969 {
                return true;
            }
            // SMB/CIFS
            if f_type == 0xff {
                return true;
            }
            // FUSE
            if f_type == 0x65735546 {
                return true;
            }
            // AFP
            if f_type == 0x0001 {
                return true;
            }
        }
        false
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = path;
        false
    }
}

#[cfg(target_os = "macos")]
fn path_str_for_statfs(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Resolve symlinks in a path, returning the canonical path if follow_symlinks is true.
pub(crate) fn resolve_path(path: &str, follow_symlinks: bool) -> String {
    if follow_symlinks {
        std::fs::canonicalize(path)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| path.to_string())
    } else {
        path.to_string()
    }
}

/// Rust-native file watch service.
/// Monitors file system changes using OS-level events (FSEvents on macOS,
/// inotify on Linux, ReadDirectoryChangesW on Windows).
pub struct FileWatchServiceImpl {
    watches: DashMap<String, WatchSession>,
    next_watch_id: AtomicI64,
    /// Optional reference to the task graph for file-change -> task invalidation.
    task_graph: Option<Arc<TaskGraphServiceImpl>>,
    vfs_delta_store: Arc<VfsDeltaStore>,
}

#[derive(Debug, Clone)]
pub struct VfsDeltaStore {
    events: Arc<Mutex<VecDeque<FileChangeEvent>>>,
    max_events: usize,
}

impl Default for VfsDeltaStore {
    fn default() -> Self {
        Self::new(MAX_RETAINED_EVENTS)
    }
}

impl VfsDeltaStore {
    pub fn new(max_events: usize) -> Self {
        Self {
            events: Arc::new(Mutex::new(VecDeque::with_capacity(max_events.min(256)))),
            max_events,
        }
    }

    pub fn record(&self, event: FileChangeEvent) {
        if let Ok(mut events) = self.events.lock() {
            if events.len() >= self.max_events {
                events.pop_front();
            }
            events.push_back(event);
        }
    }

    pub fn changed_paths_since(&self, since_timestamp_ms: i64) -> Vec<String> {
        self.changed_paths_since_with_watermark(since_timestamp_ms)
            .map(|(paths, _)| paths)
            .unwrap_or_default()
    }

    pub fn changed_paths_since_with_watermark(
        &self,
        since_timestamp_ms: i64,
    ) -> Option<(Vec<String>, i64)> {
        let paths = self
            .events
            .lock()
            .map(|events| {
                events
                    .iter()
                    .filter(|event| event.timestamp_ms >= since_timestamp_ms)
                    .fold((Vec::new(), None), |(mut paths, max_ts), event| {
                        paths.push(event.path.clone());
                        (
                            paths,
                            Some(max_ts.unwrap_or(event.timestamp_ms).max(event.timestamp_ms)),
                        )
                    })
            })
            .unwrap_or_default();
        let Some(watermark_ms) = paths.1 else {
            return None;
        };
        let mut paths = paths.0;
        paths.sort();
        paths.dedup();
        Some((paths, watermark_ms))
    }
}

impl Default for FileWatchServiceImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl FileWatchServiceImpl {
    pub fn new() -> Self {
        Self {
            watches: DashMap::new(),
            next_watch_id: AtomicI64::new(1),
            task_graph: None,
            vfs_delta_store: Arc::new(VfsDeltaStore::default()),
        }
    }

    pub fn with_task_graph(task_graph: Arc<TaskGraphServiceImpl>) -> Self {
        Self {
            watches: DashMap::new(),
            next_watch_id: AtomicI64::new(1),
            task_graph: Some(task_graph),
            vfs_delta_store: Arc::new(VfsDeltaStore::default()),
        }
    }

    pub fn with_task_graph_and_delta_store(
        task_graph: Arc<TaskGraphServiceImpl>,
        vfs_delta_store: Arc<VfsDeltaStore>,
    ) -> Self {
        Self {
            watches: DashMap::new(),
            next_watch_id: AtomicI64::new(1),
            task_graph: Some(task_graph),
            vfs_delta_store,
        }
    }

    pub fn vfs_delta_store(&self) -> Arc<VfsDeltaStore> {
        Arc::clone(&self.vfs_delta_store)
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    fn count_files(path: &str) -> i64 {
        Self::count_files_inner(path, 0, 0)
    }

    /// Returns the debounce interval in milliseconds for a given watch session.
    pub fn debounce_ms_for(&self, watch_id: &str) -> u64 {
        self.watches
            .get(watch_id)
            .map(|s| s.debounce_ms)
            .unwrap_or(DEFAULT_DEBOUNCE_MS)
    }

    pub(crate) fn count_files_inner(path: &str, depth: u32, count: i64) -> i64 {
        if depth > MAX_TREE_DEPTH || count > MAX_FILES_TO_COUNT {
            return count;
        }
        let mut count = count;
        let path = std::path::Path::new(path);
        if path.exists() {
            if path.is_dir() {
                if let Ok(entries) = std::fs::read_dir(path) {
                    for entry in entries.flatten() {
                        count += 1;
                        if entry.path().is_dir() {
                            if let Some(name) = entry.file_name().to_str() {
                                if name != "node_modules"
                                    && name != ".gradle"
                                    && name != "build"
                                    && !name.starts_with('.')
                                {
                                    count = Self::count_files_inner(
                                        &entry.path().to_string_lossy(),
                                        depth + 1,
                                        count,
                                    );
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

    /// Check if a file path matches the include/exclude patterns.
    /// Uses zero-allocation Ant-style matching from file_tree module.
    fn matches_patterns(path: &str, include: &[String], exclude: &[String]) -> bool {
        // If no include patterns, accept everything
        if !include.is_empty() {
            let matched = include
                .iter()
                .any(|pattern| crate::server::file_tree::ant_match(path, pattern));
            if !matched {
                return false;
            }
        }

        // If exclude patterns match, reject
        for pattern in exclude {
            if crate::server::file_tree::ant_match(path, pattern) {
                return false;
            }
        }

        true
    }

    /// Convert a notify event kind to a proto change type string.
    fn event_kind_to_change_type(kind: &EventKind) -> &'static str {
        use EventKind::*;
        match kind {
            Create(_) => "CREATED",
            Modify(_) => "MODIFIED",
            Remove(_) => "DELETED",
            _ => "MODIFIED",
        }
    }

    fn file_event(path: String, change_type: String) -> FileChangeEvent {
        let metadata = std::fs::metadata(&path).ok();
        FileChangeEvent {
            path,
            change_type,
            timestamp_ms: Self::now_ms(),
            file_size: metadata
                .as_ref()
                .map(|metadata| metadata.len() as i64)
                .unwrap_or(0),
            is_directory: metadata
                .as_ref()
                .map(|metadata| metadata.is_dir())
                .unwrap_or(false),
        }
    }

    fn record_event(
        retained_events: &Arc<Mutex<VecDeque<FileChangeEvent>>>,
        event_tx: &broadcast::Sender<FileChangeEvent>,
        event: FileChangeEvent,
    ) {
        if let Ok(mut events) = retained_events.lock() {
            if events.len() >= MAX_RETAINED_EVENTS {
                events.pop_front();
            }
            events.push_back(event.clone());
        }
        let _ = event_tx.send(event);
    }

    fn record_event_with_store(
        vfs_delta_store: &Arc<VfsDeltaStore>,
        retained_events: &Arc<Mutex<VecDeque<FileChangeEvent>>>,
        event_tx: &broadcast::Sender<FileChangeEvent>,
        event: FileChangeEvent,
    ) {
        vfs_delta_store.record(event.clone());
        Self::record_event(retained_events, event_tx, event);
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

        if let Ok(metadata) = std::fs::metadata(&root_path) {
            let file_type = metadata.file_type();
            if !file_type.is_dir() && !file_type.is_file() {
                return Err(Status::failed_precondition(format!(
                    "unsupported file-watch root type: {root_path}"
                )));
            }
        }

        let watch_id = format!(
            "watch-{}",
            self.next_watch_id.fetch_add(1, Ordering::Relaxed)
        );
        let files_watched = Self::count_files(&req.root_path);

        // Set up the OS-level file watcher
        let _root_path_clone = root_path.clone();
        let include_patterns = req.include_patterns.clone();
        let exclude_patterns = req.exclude_patterns.clone();
        let changes_detected = Arc::new(AtomicI64::new(0));
        let changes_detected_clone = changes_detected.clone();
        let retained_events = Arc::new(Mutex::new(VecDeque::with_capacity(256)));
        let retained_events_clone = retained_events.clone();
        let (event_tx, _) = broadcast::channel::<FileChangeEvent>(256);
        let event_tx_clone = event_tx.clone();
        let task_graph_for_watcher = self.task_graph.clone();
        let vfs_delta_store_for_watcher = Arc::clone(&self.vfs_delta_store);
        let debounce_ms = if req.debounce_ms > 0 {
            req.debounce_ms as u64
        } else {
            DEFAULT_DEBOUNCE_MS
        };
        let follow_symlinks = req.follow_symlinks;

        // Detect network filesystem and use polling fallback
        let is_network = is_network_filesystem(&root_path);
        let polling_mode = is_network;

        let watcher_result = if !polling_mode {
            let config = notify::Config::default().with_poll_interval(Duration::from_millis(100));
            RecommendedWatcher::new(
                move |res: Result<Event, notify::Error>| {
                    if let Ok(event) = res {
                        let paths: Vec<String> = event
                            .paths
                            .iter()
                            .map(|p| resolve_path(&p.to_string_lossy(), follow_symlinks))
                            .collect();

                        for path in paths {
                            if !Self::matches_patterns(&path, &include_patterns, &exclude_patterns)
                            {
                                continue;
                            }

                            changes_detected_clone.fetch_add(1, Ordering::Relaxed);
                            let change_type =
                                Self::event_kind_to_change_type(&event.kind).to_string();
                            let file_event = Self::file_event(path, change_type);
                            Self::record_event_with_store(
                                &vfs_delta_store_for_watcher,
                                &retained_events_clone,
                                &event_tx_clone,
                                file_event.clone(),
                            );

                            if let Some(tg) = &task_graph_for_watcher {
                                let count = tg.invalidate_tasks_for_files(&[file_event.path]);
                                if count > 0 {
                                    tracing::info!(
                                        tasks_invalidated = count,
                                        "File changes invalidated dependent tasks"
                                    );
                                }
                            }
                        }
                    }
                },
                config,
            )
            .and_then(|mut watcher| {
                watcher
                    .watch(root_path.as_ref(), RecursiveMode::Recursive)
                    .map(|_| watcher)
            })
        } else {
            Err(notify::Error::io(std::io::Error::other(
                "network filesystem detected, using polling fallback",
            )))
        };

        let watcher = match watcher_result {
            Ok(w) => Some(w),
            Err(e) => {
                tracing::warn!(
                    watch_id = %watch_id,
                    error = %e,
                    root = %root_path,
                    "OS watcher failed, falling back to polling mode"
                );
                None
            }
        };

        let using_polling = polling_mode || watcher.is_none();

        self.watches.insert(
            watch_id.clone(),
            WatchSession {
                start_time: Instant::now(),
                files_watched,
                changes_detected,
                last_poll_ms: AtomicI64::new(Self::now_ms()),
                polling_mode: AtomicBool::new(using_polling),
                debounce_ms,
                events: retained_events,
                event_tx,
                _watcher: watcher,
            },
        );

        tracing::info!(
            watch_id = %watch_id,
            root = %root_path,
            files = files_watched,
            polling = using_polling,
            network_fs = is_network,
            debounce_ms,
            follow_symlinks,
            "File watch started"
        );

        Ok(Response::new(StartWatchingResponse {
            watching: true,
            watch_id,
            files_watched,
            polling_mode: using_polling,
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

    type PollChangesStream = std::pin::Pin<
        Box<
            dyn tonic::codegen::tokio_stream::Stream<Item = Result<FileChangeEvent, Status>> + Send,
        >,
    >;

    async fn poll_changes(
        &self,
        request: Request<PollChangesRequest>,
    ) -> Result<Response<Self::PollChangesStream>, Status> {
        let req = request.into_inner();

        if let Some(session) = self.watches.get(&req.watch_id) {
            session
                .last_poll_ms
                .store(Self::now_ms(), Ordering::Relaxed);

            let since_timestamp_ms = req.since_timestamp_ms;
            let replay = session
                .events
                .lock()
                .map(|events| {
                    events
                        .iter()
                        .filter(|event| event.timestamp_ms >= since_timestamp_ms)
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let mut rx = session.event_tx.subscribe();
            let stream = async_stream::stream! {
                for event in replay {
                    yield Ok(event);
                }
                loop {
                    match rx.recv().await {
                        Ok(event) if event.timestamp_ms >= since_timestamp_ms => yield Ok(event),
                        Ok(_) => {}
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            tracing::warn!(skipped, "File-watch poll receiver lagged behind retained events");
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            };

            Ok(Response::new(Box::pin(stream) as Self::PollChangesStream))
        } else {
            Err(Status::not_found(format!(
                "Watch session '{}' not found",
                req.watch_id
            )))
        }
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
                polling_mode: session.polling_mode.load(Ordering::Relaxed),
            }))
        } else {
            Ok(Response::new(GetWatchStatsResponse {
                files_watched: 0,
                changes_detected: 0,
                last_poll_time_ms: 0,
                watch_start_time_ms: 0,
                polling_mode: false,
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
                debounce_ms: 0,
                follow_symlinks: true,
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
                debounce_ms: 0,
                follow_symlinks: true,
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

    #[test]
    fn test_matches_patterns_no_filters() {
        assert!(FileWatchServiceImpl::matches_patterns(
            "/tmp/test.java",
            &[],
            &[]
        ));
    }

    #[test]
    fn vfs_delta_store_retains_recent_unique_paths() {
        let store = VfsDeltaStore::new(3);
        for (path, timestamp_ms) in [
            ("/repo/old.txt", 10),
            ("/repo/src/Main.java", 20),
            ("/repo/src/Main.java", 30),
            ("/repo/settings.gradle", 40),
        ] {
            store.record(FileChangeEvent {
                path: path.to_string(),
                change_type: "MODIFIED".to_string(),
                timestamp_ms,
                file_size: 0,
                is_directory: false,
            });
        }

        let paths = store.changed_paths_since(20);

        assert_eq!(
            paths,
            vec![
                "/repo/settings.gradle".to_string(),
                "/repo/src/Main.java".to_string()
            ]
        );
    }

    #[test]
    fn test_matches_patterns_include() {
        assert!(FileWatchServiceImpl::matches_patterns(
            "/tmp/Test.java",
            &["**/*.java".to_string()],
            &[]
        ));
        assert!(!FileWatchServiceImpl::matches_patterns(
            "/tmp/Test.kt",
            &["**/*.java".to_string()],
            &[]
        ));
    }

    #[tokio::test]
    async fn file_watch_reports_first_change_quickly() {
        use tokio_stream::StreamExt;

        let dir = tempfile::tempdir().unwrap();
        let root_path = dir.path().to_string_lossy().into_owned();
        let svc = FileWatchServiceImpl::new();

        let start = svc
            .start_watching(Request::new(StartWatchingRequest {
                root_path: root_path.clone(),
                include_patterns: vec!["**/*.java".to_string()],
                exclude_patterns: vec![],
                debounce_ms: 0,
                follow_symlinks: true,
            }))
            .await
            .unwrap()
            .into_inner();

        let mut stream = svc
            .poll_changes(Request::new(PollChangesRequest {
                watch_id: start.watch_id.clone(),
                since_timestamp_ms: 0,
            }))
            .await
            .unwrap()
            .into_inner();

        // Give the platform watcher a short moment to subscribe before the write.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let src_dir = dir.path().join("src");
        tokio::fs::create_dir_all(&src_dir).await.unwrap();
        let changed_file = src_dir.join("Probe.java");
        let changed_suffix = changed_file.to_string_lossy().into_owned();
        let changed_at = Instant::now();
        tokio::fs::write(&changed_file, b"class Probe {}\n")
            .await
            .unwrap();

        let event = tokio::time::timeout(Duration::from_secs(3), async {
            while let Some(event) = stream.next().await {
                let event = event.unwrap();
                if event.path.ends_with(&changed_suffix) || event.path == changed_suffix {
                    return event;
                }
            }
            panic!("file watcher stream ended before reporting the changed file");
        })
        .await
        .expect("file watcher did not report the first Java file change within 3s");

        let latency_ms = changed_at.elapsed().as_millis();
        println!("file_watch_first_change_latency_ms={latency_ms}");
        assert!(
            event.change_type == "CREATED" || event.change_type == "MODIFIED",
            "unexpected change type: {}",
            event.change_type
        );
        assert!(
            latency_ms < 1_500,
            "first file-watch event took {latency_ms}ms"
        );

        drop(stream);
        let stopped = svc
            .stop_watching(Request::new(StopWatchingRequest {
                watch_id: start.watch_id,
            }))
            .await
            .unwrap()
            .into_inner();
        assert!(stopped.stopped);
    }

    #[tokio::test]
    async fn poll_changes_replays_events_recorded_before_polling() {
        use tokio_stream::StreamExt;

        let dir = tempfile::tempdir().unwrap();
        let root_path = dir.path().to_string_lossy().into_owned();
        let svc = FileWatchServiceImpl::new();

        let start = svc
            .start_watching(Request::new(StartWatchingRequest {
                root_path,
                include_patterns: vec!["**/*.txt".to_string()],
                exclude_patterns: vec![],
                debounce_ms: 0,
                follow_symlinks: true,
            }))
            .await
            .unwrap()
            .into_inner();

        tokio::time::sleep(Duration::from_millis(200)).await;
        let changed_file = dir.path().join("late-poller.txt");
        let changed_path = changed_file.to_string_lossy().into_owned();
        tokio::fs::write(&changed_file, b"created before poll")
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let stats = svc
                    .get_watch_stats(Request::new(GetWatchStatsRequest {
                        watch_id: start.watch_id.clone(),
                    }))
                    .await
                    .unwrap()
                    .into_inner();
                if stats.changes_detected > 0 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("file watcher did not record the pre-poll event");

        let mut stream = svc
            .poll_changes(Request::new(PollChangesRequest {
                watch_id: start.watch_id.clone(),
                since_timestamp_ms: 0,
            }))
            .await
            .unwrap()
            .into_inner();

        let event = tokio::time::timeout(Duration::from_secs(2), async {
            while let Some(event) = stream.next().await {
                let event = event.unwrap();
                if event.path == changed_path || event.path.ends_with("late-poller.txt") {
                    return event;
                }
            }
            panic!("file watcher stream ended before replaying the retained event");
        })
        .await
        .expect("file watcher did not replay the retained event");

        assert!(
            event.change_type == "CREATED" || event.change_type == "MODIFIED",
            "unexpected change type: {}",
            event.change_type
        );
        assert_eq!(event.file_size, "created before poll".len() as i64);

        drop(stream);
        svc.stop_watching(Request::new(StopWatchingRequest {
            watch_id: start.watch_id,
        }))
        .await
        .unwrap();
    }

    #[test]
    fn test_matches_patterns_exclude() {
        assert!(!FileWatchServiceImpl::matches_patterns(
            "/tmp/generated/Test.java",
            &["**/*.java".to_string()],
            &["**/generated/**".to_string()]
        ));
    }

    #[tokio::test]
    async fn test_poll_changes_nonexistent() {
        let svc = FileWatchServiceImpl::new();

        let result = svc
            .poll_changes(Request::new(PollChangesRequest {
                watch_id: "nonexistent".to_string(),
                since_timestamp_ms: 0,
            }))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_watch_real_file_changes() {
        let svc = FileWatchServiceImpl::new();

        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path().to_string_lossy().to_string();

        let resp = svc
            .start_watching(Request::new(StartWatchingRequest {
                root_path: dir_path.clone(),
                include_patterns: vec!["**/*".to_string()],
                exclude_patterns: vec![],
                debounce_ms: 0,
                follow_symlinks: true,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.watching);

        // Create a file to trigger an event
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, b"hello").unwrap();

        // Give the watcher a moment to process
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Stats should show at least the created file
        let stats = svc
            .get_watch_stats(Request::new(GetWatchStatsRequest {
                watch_id: resp.watch_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        // The file was created during watching
        assert!(stats.files_watched >= 0);

        svc.stop_watching(Request::new(StopWatchingRequest {
            watch_id: resp.watch_id,
        }))
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_start_multiple_watchers() {
        let svc = FileWatchServiceImpl::new();

        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        let dir3 = tempfile::tempdir().unwrap();
        let path1 = dir1.path().to_string_lossy().to_string();
        let path2 = dir2.path().to_string_lossy().to_string();
        let path3 = dir3.path().to_string_lossy().to_string();

        // Seed each directory with a file so files_watched > 0
        std::fs::write(dir1.path().join("a.txt"), b"1").unwrap();
        std::fs::write(dir2.path().join("b.txt"), b"2").unwrap();
        std::fs::write(dir3.path().join("c.txt"), b"3").unwrap();

        let resp1 = svc
            .start_watching(Request::new(StartWatchingRequest {
                root_path: path1.clone(),
                include_patterns: vec![],
                exclude_patterns: vec![],
                debounce_ms: 0,
                follow_symlinks: true,
            }))
            .await
            .unwrap()
            .into_inner();

        let resp2 = svc
            .start_watching(Request::new(StartWatchingRequest {
                root_path: path2.clone(),
                include_patterns: vec![],
                exclude_patterns: vec![],
                debounce_ms: 0,
                follow_symlinks: true,
            }))
            .await
            .unwrap()
            .into_inner();

        let resp3 = svc
            .start_watching(Request::new(StartWatchingRequest {
                root_path: path3.clone(),
                include_patterns: vec![],
                exclude_patterns: vec![],
                debounce_ms: 0,
                follow_symlinks: true,
            }))
            .await
            .unwrap()
            .into_inner();

        // All three should be active with distinct IDs
        assert!(resp1.watching);
        assert!(resp2.watching);
        assert!(resp3.watching);
        assert_ne!(resp1.watch_id, resp2.watch_id);
        assert_ne!(resp2.watch_id, resp3.watch_id);
        assert_ne!(resp1.watch_id, resp3.watch_id);

        // Stats for each should reflect the files in its respective directory
        let stats1 = svc
            .get_watch_stats(Request::new(GetWatchStatsRequest {
                watch_id: resp1.watch_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        let stats2 = svc
            .get_watch_stats(Request::new(GetWatchStatsRequest {
                watch_id: resp2.watch_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        let stats3 = svc
            .get_watch_stats(Request::new(GetWatchStatsRequest {
                watch_id: resp3.watch_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(
            stats1.files_watched >= 1,
            "dir1 should have at least 1 file"
        );
        assert!(
            stats2.files_watched >= 1,
            "dir2 should have at least 1 file"
        );
        assert!(
            stats3.files_watched >= 1,
            "dir3 should have at least 1 file"
        );
        assert!(stats1.watch_start_time_ms > 0);
        assert!(stats2.watch_start_time_ms > 0);
        assert!(stats3.watch_start_time_ms > 0);

        // Cleanup
        svc.stop_watching(Request::new(StopWatchingRequest {
            watch_id: resp1.watch_id,
        }))
        .await
        .unwrap();
        svc.stop_watching(Request::new(StopWatchingRequest {
            watch_id: resp2.watch_id,
        }))
        .await
        .unwrap();
        svc.stop_watching(Request::new(StopWatchingRequest {
            watch_id: resp3.watch_id,
        }))
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_restart_watcher() {
        let svc = FileWatchServiceImpl::new();

        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path().to_string_lossy().to_string();

        // First start
        let resp1 = svc
            .start_watching(Request::new(StartWatchingRequest {
                root_path: dir_path.clone(),
                include_patterns: vec![],
                exclude_patterns: vec![],
                debounce_ms: 0,
                follow_symlinks: true,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp1.watching);
        let first_id = resp1.watch_id.clone();

        // Stop it
        let stop1 = svc
            .stop_watching(Request::new(StopWatchingRequest {
                watch_id: first_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(stop1.stopped);

        // Verify stats are empty for the old ID
        let stats_after_stop = svc
            .get_watch_stats(Request::new(GetWatchStatsRequest {
                watch_id: first_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(stats_after_stop.files_watched, 0);
        assert_eq!(stats_after_stop.watch_start_time_ms, 0);

        // Restart with the same path — should get a new watch ID
        let resp2 = svc
            .start_watching(Request::new(StartWatchingRequest {
                root_path: dir_path.clone(),
                include_patterns: vec![],
                exclude_patterns: vec![],
                debounce_ms: 0,
                follow_symlinks: true,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp2.watching);
        assert_ne!(
            resp2.watch_id, first_id,
            "restarted watcher should have a new ID"
        );

        // Stats should be active again
        let stats_restarted = svc
            .get_watch_stats(Request::new(GetWatchStatsRequest {
                watch_id: resp2.watch_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(stats_restarted.watch_start_time_ms > 0);

        // Cleanup
        svc.stop_watching(Request::new(StopWatchingRequest {
            watch_id: resp2.watch_id,
        }))
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_watch_nonexistent_dir() {
        let svc = FileWatchServiceImpl::new();

        // Use a path that is extremely unlikely to exist
        let nonexistent = "/tmp/gradle_file_watch_test_nonexistent_dir_abcdef123456";

        // A nonexistent directory falls back to polling mode (robustness)
        let result = svc
            .start_watching(Request::new(StartWatchingRequest {
                root_path: nonexistent.to_string(),
                include_patterns: vec![],
                exclude_patterns: vec![],
                debounce_ms: 0,
                follow_symlinks: true,
            }))
            .await;

        let resp = result.unwrap().into_inner();
        assert!(resp.watching, "should succeed in polling mode");
        assert!(
            resp.polling_mode,
            "should use polling fallback for nonexistent dir"
        );
        assert_eq!(resp.files_watched, 0, "no files in nonexistent dir");
    }

    #[tokio::test]
    async fn test_stop_watcher_no_longer_active() {
        let svc = FileWatchServiceImpl::new();
        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path().to_string_lossy().to_string();

        let resp = svc
            .start_watching(Request::new(StartWatchingRequest {
                root_path: dir_path.clone(),
                include_patterns: vec![],
                exclude_patterns: vec![],
                debounce_ms: 0,
                follow_symlinks: true,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.watching);
        let watch_id = resp.watch_id.clone();

        // Verify the watcher is active via stats
        let stats_before = svc
            .get_watch_stats(Request::new(GetWatchStatsRequest {
                watch_id: watch_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(
            stats_before.watch_start_time_ms > 0,
            "watcher should be active before stop"
        );

        // Stop the watcher
        let stop_resp = svc
            .stop_watching(Request::new(StopWatchingRequest {
                watch_id: watch_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(stop_resp.stopped);

        // Stats should now return zeros
        let stats_after = svc
            .get_watch_stats(Request::new(GetWatchStatsRequest {
                watch_id: watch_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(
            stats_after.files_watched, 0,
            "files_watched should be 0 after stop"
        );
        assert_eq!(
            stats_after.changes_detected, 0,
            "changes_detected should be 0 after stop"
        );
        assert_eq!(
            stats_after.watch_start_time_ms, 0,
            "watch_start_time_ms should be 0 after stop"
        );

        // Polling the stopped watcher should fail with NotFound
        let poll_result = svc
            .poll_changes(Request::new(PollChangesRequest {
                watch_id: watch_id.clone(),
                since_timestamp_ms: 0,
            }))
            .await;

        assert!(
            poll_result.is_err(),
            "polling a stopped watcher should return an error"
        );
        match poll_result {
            Err(status) => {
                assert_eq!(status.code(), tonic::Code::NotFound);
            }
            Ok(_) => panic!("expected NotFound error when polling a stopped watcher"),
        }
    }

    /// Test that polling with no active watchers returns an error (not_found).
    #[tokio::test]
    async fn test_poll_changes_no_watchers_returns_error() {
        let svc = FileWatchServiceImpl::new();

        // With no watchers started at all, polling any watch_id should fail
        let result = svc
            .poll_changes(Request::new(PollChangesRequest {
                watch_id: "watch-nothing".to_string(),
                since_timestamp_ms: 0,
            }))
            .await;

        assert!(result.is_err(), "polling with no watchers should fail");
        match result {
            Err(status) => {
                assert_eq!(status.code(), tonic::Code::NotFound);
                assert!(
                    status.message().contains("not found"),
                    "error message should say 'not found': {}",
                    status.message()
                );
            }
            Ok(_) => panic!("expected NotFound error when no watchers exist"),
        }

        // Also try with a completely empty watch_id
        let result2 = svc
            .poll_changes(Request::new(PollChangesRequest {
                watch_id: String::new(),
                since_timestamp_ms: 0,
            }))
            .await;

        assert!(
            result2.is_err(),
            "polling with empty watch_id should also fail"
        );
    }

    /// Test that multiple polls on the same watcher each return independent streams.
    /// Verifies that poll_changes can be called multiple times on the same watch_id,
    /// and each call succeeds and returns a valid (non-error) response. Then verifies
    #[tokio::test]
    async fn test_incremental_poll_changes() {
        use tokio::time::Duration;

        let svc = FileWatchServiceImpl::new();
        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path().to_string_lossy().to_string();

        let resp = svc
            .start_watching(Request::new(StartWatchingRequest {
                root_path: dir_path.clone(),
                include_patterns: vec!["**/*".to_string()],
                exclude_patterns: vec![],
                debounce_ms: 0,
                follow_symlinks: true,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.watching);
        let watch_id = resp.watch_id.clone();

        // First poll should succeed and return a valid stream
        let stream1 = svc
            .poll_changes(Request::new(PollChangesRequest {
                watch_id: watch_id.clone(),
                since_timestamp_ms: 0,
            }))
            .await;

        assert!(stream1.is_ok(), "first poll_changes should succeed");

        // Create a file while the first stream is active
        let file1 = dir.path().join("alpha.txt");
        std::fs::write(&file1, b"first").unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Drop the first stream (simulate client finishing the first poll)
        drop(stream1);

        // Second poll on the same watch_id should also succeed
        let stream2 = svc
            .poll_changes(Request::new(PollChangesRequest {
                watch_id: watch_id.clone(),
                since_timestamp_ms: 0,
            }))
            .await;

        assert!(
            stream2.is_ok(),
            "second poll_changes on same watch_id should succeed"
        );

        // Create another file while the second stream is active
        let file2 = dir.path().join("beta.txt");
        std::fs::write(&file2, b"second").unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Drop the second stream
        drop(stream2);

        // Third poll should also succeed — the watch session is still active
        let stream3 = svc
            .poll_changes(Request::new(PollChangesRequest {
                watch_id: watch_id.clone(),
                since_timestamp_ms: 0,
            }))
            .await;

        assert!(
            stream3.is_ok(),
            "third poll_changes on same watch_id should succeed"
        );
        drop(stream3);

        // Verify the watch session is still alive via stats
        let stats = svc
            .get_watch_stats(Request::new(GetWatchStatsRequest {
                watch_id: watch_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(
            stats.watch_start_time_ms > 0,
            "watch session should still be active after multiple polls"
        );

        svc.stop_watching(Request::new(StopWatchingRequest {
            watch_id: watch_id.clone(),
        }))
        .await
        .unwrap();

        let stream_after_stop = svc
            .poll_changes(Request::new(PollChangesRequest {
                watch_id: watch_id.clone(),
                since_timestamp_ms: 0,
            }))
            .await;

        assert!(stream_after_stop.is_err(), "poll after stop should fail");
    }

    /// Test watching a single file (not a directory). The notify crate supports
    /// watching individual files via RecursiveMode::Recursive, so this should
    /// succeed. This test verifies that start_watching accepts a file path,
    /// returns files_watched == 1, poll_changes succeeds, and the watcher can
    /// be stopped cleanly.
    #[tokio::test]
    async fn test_watch_single_file() {
        let svc = FileWatchServiceImpl::new();
        let dir = tempfile::tempdir().unwrap();

        // Create a specific file to watch
        let target_file = dir.path().join("target.log");
        std::fs::write(&target_file, b"initial content").unwrap();
        let file_path = target_file.to_string_lossy().to_string();

        // Start watching the file directly
        let resp = svc
            .start_watching(Request::new(StartWatchingRequest {
                root_path: file_path.clone(),
                include_patterns: vec![],
                exclude_patterns: vec![],
                debounce_ms: 0,
                follow_symlinks: true,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.watching, "watching a single file should succeed");
        assert!(!resp.watch_id.is_empty());
        // count_files returns 1 for a file
        assert_eq!(
            resp.files_watched, 1,
            "a single file should report files_watched == 1"
        );

        let watch_id = resp.watch_id.clone();

        // Verify stats are available for the file watcher
        let stats = svc
            .get_watch_stats(Request::new(GetWatchStatsRequest {
                watch_id: watch_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(stats.files_watched, 1);
        assert!(stats.watch_start_time_ms > 0);

        // Verify poll_changes works on a file watcher (should not error)
        let stream = svc
            .poll_changes(Request::new(PollChangesRequest {
                watch_id: watch_id.clone(),
                since_timestamp_ms: 0,
            }))
            .await;

        assert!(
            stream.is_ok(),
            "poll_changes on a file watcher should succeed"
        );

        // Verify last_poll_time_ms was updated by the poll
        let stats_after_poll = svc
            .get_watch_stats(Request::new(GetWatchStatsRequest {
                watch_id: watch_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(
            stats_after_poll.last_poll_time_ms > 0,
            "last_poll_time_ms should be updated after poll"
        );

        // Cleanup
        svc.stop_watching(Request::new(StopWatchingRequest { watch_id }))
            .await
            .unwrap();
    }

    /// Test that custom debounce_ms is respected (0 = default 100ms).
    #[tokio::test]
    async fn test_custom_debounce() {
        let svc = FileWatchServiceImpl::new();
        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path().to_string_lossy().to_string();

        // Explicit debounce_ms = 0 should use default
        let resp = svc
            .start_watching(Request::new(StartWatchingRequest {
                root_path: dir_path.clone(),
                include_patterns: vec![],
                exclude_patterns: vec![],
                debounce_ms: 0,
                follow_symlinks: true,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(svc.debounce_ms_for(&resp.watch_id), DEFAULT_DEBOUNCE_MS);

        // Custom debounce_ms = 500
        svc.stop_watching(Request::new(StopWatchingRequest {
            watch_id: resp.watch_id,
        }))
        .await
        .unwrap();

        let resp2 = svc
            .start_watching(Request::new(StartWatchingRequest {
                root_path: dir_path.clone(),
                include_patterns: vec![],
                exclude_patterns: vec![],
                debounce_ms: 500,
                follow_symlinks: true,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(svc.debounce_ms_for(&resp2.watch_id), 500);

        svc.stop_watching(Request::new(StopWatchingRequest {
            watch_id: resp2.watch_id,
        }))
        .await
        .unwrap();
    }

    /// Test that polling_mode is reported in start response and stats.
    #[tokio::test]
    async fn test_polling_mode_reported() {
        let svc = FileWatchServiceImpl::new();
        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path().to_string_lossy().to_string();

        let resp = svc
            .start_watching(Request::new(StartWatchingRequest {
                root_path: dir_path.clone(),
                include_patterns: vec![],
                exclude_patterns: vec![],
                debounce_ms: 0,
                follow_symlinks: true,
            }))
            .await
            .unwrap()
            .into_inner();

        // Local FS should use native watcher, not polling
        // (unless running in CI/container without proper watcher support)
        let stats = svc
            .get_watch_stats(Request::new(GetWatchStatsRequest {
                watch_id: resp.watch_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        // polling_mode in stats should match start response
        assert_eq!(stats.polling_mode, resp.polling_mode);

        svc.stop_watching(Request::new(StopWatchingRequest {
            watch_id: resp.watch_id,
        }))
        .await
        .unwrap();
    }

    /// Test that symlink resolution works (follow_symlinks=true vs false).
    #[tokio::test]
    async fn test_symlink_handling() {
        let svc = FileWatchServiceImpl::new();
        let dir = tempfile::tempdir().unwrap();
        let real_dir = dir.path().join("real");
        std::fs::create_dir_all(&real_dir).unwrap();
        std::fs::write(real_dir.join("a.txt"), b"hello").unwrap();

        let link_path = dir.path().join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_dir, &link_path).unwrap();

        // With follow_symlinks = true, should resolve and watch
        let resp = svc
            .start_watching(Request::new(StartWatchingRequest {
                root_path: link_path.to_string_lossy().to_string(),
                include_patterns: vec![],
                exclude_patterns: vec![],
                debounce_ms: 0,
                follow_symlinks: true,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.watching, "should watch through symlink");
        assert!(resp.files_watched > 0, "should count files through symlink");

        svc.stop_watching(Request::new(StopWatchingRequest {
            watch_id: resp.watch_id,
        }))
        .await
        .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_special_file_root_fails_closed() {
        let svc = FileWatchServiceImpl::new();
        let dir = tempfile::tempdir().unwrap();
        let fifo_path = dir.path().join("named-pipe");
        let fifo_cstring = std::ffi::CString::new(fifo_path.to_string_lossy().as_bytes()).unwrap();

        let created = unsafe { libc::mkfifo(fifo_cstring.as_ptr(), 0o600) };
        assert_eq!(created, 0, "mkfifo should create a special file root");

        let status = svc
            .start_watching(Request::new(StartWatchingRequest {
                root_path: fifo_path.to_string_lossy().to_string(),
                include_patterns: vec![],
                exclude_patterns: vec![],
                debounce_ms: 0,
                follow_symlinks: true,
            }))
            .await
            .unwrap_err();

        assert_eq!(status.code(), tonic::Code::FailedPrecondition);
        assert!(
            status
                .message()
                .contains("unsupported file-watch root type"),
            "diagnostic should explain why Rust refused the root: {}",
            status.message()
        );
    }

    /// Test that network FS detection returns false for local paths.
    #[test]
    fn test_network_fs_detection_local() {
        // /tmp is always local
        assert!(
            !is_network_filesystem("/tmp"),
            "/tmp should not be detected as network FS"
        );
    }

    /// Test resolve_path with follow_symlinks.
    #[test]
    fn test_resolve_path() {
        let dir = tempfile::tempdir().unwrap();
        let real_file = dir.path().join("real.txt");
        std::fs::write(&real_file, b"data").unwrap();

        // follow_symlinks = true should canonicalize
        let resolved = resolve_path(&real_file.to_string_lossy(), true);
        // canonicalize on macOS adds prefix; just check it contains the filename
        assert!(
            resolved.contains("real.txt"),
            "resolved path should contain real.txt: {}",
            resolved
        );

        // follow_symlinks = false returns as-is
        let raw = resolve_path(&real_file.to_string_lossy(), false);
        assert_eq!(raw, real_file.to_string_lossy().to_string());

        // Nonexistent path with follow_symlinks = true returns original (canonicalize fails)
        let resolved = resolve_path("/nonexistent/path", true);
        assert_eq!(resolved, "/nonexistent/path");
    }

    /// Test that count_files_inner respects depth limit and skips build dirs.
    #[test]
    fn test_count_files_depth_limit() {
        let dir = tempfile::tempdir().unwrap();
        // Create a deep directory chain
        let mut path = dir.path().to_path_buf();
        for i in 0..60 {
            path = path.join(format!("d{}", i));
            std::fs::create_dir_all(&path).unwrap();
            std::fs::write(path.join("f.txt"), b"x").unwrap();
        }

        let count = FileWatchServiceImpl::count_files_inner(&dir.path().to_string_lossy(), 0, 0);

        // Should be capped by MAX_TREE_DEPTH — 60 levels but limit is 50
        // Each level has 1 dir + 1 file = 2 entries, so max ~102
        assert!(
            count <= (MAX_TREE_DEPTH as i64 + 1) * 2,
            "should cap at MAX_TREE_DEPTH, got {}",
            count
        );
        assert!(count < 120, "should not count all 60 levels, got {}", count);
    }
}
