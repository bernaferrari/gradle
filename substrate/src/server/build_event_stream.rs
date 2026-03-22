use std::sync::atomic::{AtomicI64, Ordering};

use dashmap::DashMap;
use tokio::sync::broadcast;
use tonic::{Request, Response, Status};

use crate::proto::{
    build_event_stream_service_server::BuildEventStreamService, BuildEventMessage,
    GetEventLogRequest, GetEventLogResponse, SendBuildEventRequest, SendBuildEventResponse,
    SubscribeBuildEventsRequest,
};

/// Maximum events buffered per build to prevent unbounded memory growth.
const MAX_EVENTS_PER_BUILD: usize = 10_000;

/// Channel capacity for broadcast subscribers.
const BROADCAST_CAPACITY: usize = 256;

/// Rust-native build event streaming service.
/// Buffers build events and streams them to subscribers (IDEs, CI systems).
///
/// Uses tokio broadcast channels for real-time fan-out to multiple subscribers.
/// Events are also buffered in memory for historical queries.
#[derive(Default)]
pub struct BuildEventStreamServiceImpl {
    event_buffers: DashMap<String, Vec<BuildEventMessage>>,
    /// Broadcast channels per build_id for real-time streaming.
    build_channels: DashMap<String, broadcast::Sender<BuildEventMessage>>,
    subscribers: AtomicI64,
    events_sent: AtomicI64,
    events_received: AtomicI64,
    events_evicted: AtomicI64,
}

impl BuildEventStreamServiceImpl {
    pub fn new() -> Self {
        Self {
            event_buffers: DashMap::new(),
            build_channels: DashMap::new(),
            subscribers: AtomicI64::new(0),
            events_sent: AtomicI64::new(0),
            events_received: AtomicI64::new(0),
            events_evicted: AtomicI64::new(0),
        }
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    fn matches_filter(event: &BuildEventMessage, filter: &[String]) -> bool {
        filter.is_empty() || filter.iter().any(|f| f == &event.event_type)
    }

    /// Get or create a broadcast sender for a build.
    fn get_or_create_channel(&self, build_id: &str) -> broadcast::Sender<BuildEventMessage> {
        self.build_channels
            .entry(build_id.to_string())
            .or_insert_with(|| {
                let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
                tx
            })
            .clone()
    }

    /// Get the total number of active subscriber channels.
    pub fn active_build_count(&self) -> usize {
        self.build_channels.len()
    }

    /// Remove a build's channel and buffer (cleanup after build completes).
    pub fn cleanup_build(&self, build_id: &str) {
        self.build_channels.remove(build_id);
        self.event_buffers.remove(build_id);
    }
}

#[tonic::async_trait]
impl BuildEventStreamService for BuildEventStreamServiceImpl {
    type SubscribeBuildEventsStream = std::pin::Pin<Box<dyn tonic::codegen::tokio_stream::Stream<Item = Result<BuildEventMessage, Status>> + Send>>;

    async fn subscribe_build_events(
        &self,
        request: Request<SubscribeBuildEventsRequest>,
    ) -> Result<Response<Self::SubscribeBuildEventsStream>, Status> {
        let req = request.into_inner();
        self.subscribers.fetch_add(1, Ordering::Relaxed);

        let build_id = req.build_id.clone();
        let filter = req.event_types;

        // Get the broadcast receiver for this build
        let rx = self.get_or_create_channel(&build_id).subscribe();

        // Also replay buffered events
        let buffered_events: Vec<BuildEventMessage> = if let Some(buf) = self.event_buffers.get(&build_id) {
            buf.iter()
                .filter(|e| Self::matches_filter(e, &filter))
                .cloned()
                .collect()
        } else {
            Vec::new()
        };

        let buffered_count = buffered_events.len();

        tracing::debug!(
            build_id = %build_id,
            buffered = buffered_count,
            "Build event subscriber connected"
        );

        // Create a stream that first emits buffered events, then live events
        let stream = async_stream::stream! {
            // First, replay buffered events
            for event in buffered_events {
                yield Ok(event);
            }

            // Then, stream live events from the broadcast channel
            let mut rx = rx;
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if Self::matches_filter(&event, &filter) {
                            yield Ok(event);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::debug!(skipped = n, "Event subscriber lagged");
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
        };

        Ok(Response::new(Box::pin(stream) as Self::SubscribeBuildEventsStream))
    }

    async fn send_build_event(
        &self,
        request: Request<SendBuildEventRequest>,
    ) -> Result<Response<SendBuildEventResponse>, Status> {
        let req = request.into_inner();
        self.events_received.fetch_add(1, Ordering::Relaxed);

        let event = BuildEventMessage {
            build_id: req.build_id.clone(),
            timestamp_ms: Self::now_ms(),
            event_type: req.event_type,
            event_id: req.event_id,
            properties: req.properties,
            display_name: req.display_name,
            parent_id: req.parent_id,
        };

        // Buffer the event
        if let Some(mut buf) = self.event_buffers.get_mut(&req.build_id) {
            if buf.len() >= MAX_EVENTS_PER_BUILD {
                // Evict oldest events (keep the most recent half)
                let evict_count = buf.len() / 2;
                buf.drain(..evict_count);
                self.events_evicted.fetch_add(evict_count as i64, Ordering::Relaxed);
            }
            buf.push(event.clone());
        } else {
            self.event_buffers
                .entry(req.build_id.clone())
                .or_default()
                .push(event.clone());
        }

        // Broadcast to live subscribers
        if let Some(tx) = self.build_channels.get(&req.build_id) {
            // Ignore send errors — no subscribers or channel full
            let _ = tx.send(event);
        }

        self.events_sent.fetch_add(1, Ordering::Relaxed);

        Ok(Response::new(SendBuildEventResponse { accepted: true }))
    }

    async fn get_event_log(
        &self,
        request: Request<GetEventLogRequest>,
    ) -> Result<Response<GetEventLogResponse>, Status> {
        let req = request.into_inner();

        let events = if let Some(buf) = self.event_buffers.get(&req.build_id) {
            let mut events: Vec<BuildEventMessage> = buf.iter().cloned().collect();

            // Filter by timestamp if requested
            if req.since_timestamp_ms > 0 {
                events.retain(|e| e.timestamp_ms >= req.since_timestamp_ms);
            }

            // Filter by event type if provided
            if !req.event_types.is_empty() {
                events.retain(|e| req.event_types.contains(&e.event_type));
            }

            // Limit
            if req.max_events > 0 && events.len() as i32 > req.max_events {
                let len = events.len();
                events = events.split_off(len - req.max_events as usize);
            }

            events
        } else {
            Vec::new()
        };

        let total = events.len() as i32;

        Ok(Response::new(GetEventLogResponse {
            events,
            total_events: total,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_send_and_get_events() {
        let svc = BuildEventStreamServiceImpl::new();

        // Send some events
        svc.send_build_event(Request::new(SendBuildEventRequest {
            build_id: "build-1".to_string(),
            event_type: "build_start".to_string(),
            event_id: "evt-1".to_string(),
            properties: Default::default(),
            display_name: "Build".to_string(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

        svc.send_build_event(Request::new(SendBuildEventRequest {
            build_id: "build-1".to_string(),
            event_type: "task_start".to_string(),
            event_id: "evt-2".to_string(),
            properties: Default::default(),
            display_name: ":compileJava".to_string(),
            parent_id: "evt-1".to_string(),
        }))
        .await
        .unwrap();

        svc.send_build_event(Request::new(SendBuildEventRequest {
            build_id: "build-1".to_string(),
            event_type: "task_finish".to_string(),
            event_id: "evt-3".to_string(),
            properties: Default::default(),
            display_name: ":compileJava".to_string(),
            parent_id: "evt-1".to_string(),
        }))
        .await
        .unwrap();

        // Get all events
        let log = svc
            .get_event_log(Request::new(GetEventLogRequest {
                build_id: "build-1".to_string(),
                since_timestamp_ms: 0,
                max_events: 0,
                event_types: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(log.total_events, 3);
        assert_eq!(log.events[0].event_type, "build_start");
        assert_eq!(log.events[1].event_type, "task_start");
        assert_eq!(log.events[2].event_type, "task_finish");
    }

    #[tokio::test]
    async fn test_filtered_events_by_type() {
        let svc = BuildEventStreamServiceImpl::new();

        svc.send_build_event(Request::new(SendBuildEventRequest {
            build_id: "build-2".to_string(),
            event_type: "build_start".to_string(),
            event_id: "e1".to_string(),
            properties: Default::default(),
            display_name: String::new(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

        svc.send_build_event(Request::new(SendBuildEventRequest {
            build_id: "build-2".to_string(),
            event_type: "progress".to_string(),
            event_id: "e2".to_string(),
            properties: Default::default(),
            display_name: String::new(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

        svc.send_build_event(Request::new(SendBuildEventRequest {
            build_id: "build-2".to_string(),
            event_type: "progress".to_string(),
            event_id: "e3".to_string(),
            properties: Default::default(),
            display_name: String::new(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

        // Get only progress events
        let log = svc
            .get_event_log(Request::new(GetEventLogRequest {
                build_id: "build-2".to_string(),
                since_timestamp_ms: 0,
                max_events: 0,
                event_types: vec!["progress".to_string()],
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(log.total_events, 2);
        assert!(log.events.iter().all(|e| e.event_type == "progress"));
    }

    #[tokio::test]
    async fn test_max_events() {
        let svc = BuildEventStreamServiceImpl::new();

        for i in 0..10 {
            svc.send_build_event(Request::new(SendBuildEventRequest {
                build_id: "build-3".to_string(),
                event_type: format!("event_{}", i),
                event_id: format!("e{}", i),
                properties: Default::default(),
                display_name: String::new(),
                parent_id: String::new(),
            }))
            .await
            .unwrap();
        }

        let log = svc
            .get_event_log(Request::new(GetEventLogRequest {
                build_id: "build-3".to_string(),
                since_timestamp_ms: 0,
                max_events: 3,
                event_types: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(log.total_events, 3);
        // Should get the last 3 events
        assert_eq!(log.events[0].event_type, "event_7");
        assert_eq!(log.events[1].event_type, "event_8");
        assert_eq!(log.events[2].event_type, "event_9");
    }

    #[tokio::test]
    async fn test_unknown_build() {
        let svc = BuildEventStreamServiceImpl::new();

        let log = svc
            .get_event_log(Request::new(GetEventLogRequest {
                build_id: "nonexistent".to_string(),
                since_timestamp_ms: 0,
                max_events: 0,
                event_types: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(log.total_events, 0);
    }

    #[tokio::test]
    async fn test_buffer_eviction() {
        let svc = BuildEventStreamServiceImpl::new();

        // Send MAX_EVENTS_PER_BUILD events
        for i in 0..MAX_EVENTS_PER_BUILD {
            svc.send_build_event(Request::new(SendBuildEventRequest {
                build_id: "build-evict".to_string(),
                event_type: format!("event_{}", i),
                event_id: format!("e{}", i),
                properties: Default::default(),
                display_name: String::new(),
                parent_id: String::new(),
            }))
            .await
            .unwrap();
        }

        // Buffer should be at capacity
        let log = svc
            .get_event_log(Request::new(GetEventLogRequest {
                build_id: "build-evict".to_string(),
                since_timestamp_ms: 0,
                max_events: 0,
                event_types: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(log.total_events, MAX_EVENTS_PER_BUILD as i32);

        // Send one more — should trigger eviction
        svc.send_build_event(Request::new(SendBuildEventRequest {
            build_id: "build-evict".to_string(),
            event_type: "overflow".to_string(),
            event_id: "e-overflow".to_string(),
            properties: Default::default(),
            display_name: String::new(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

        // Should have evicted old events
        assert!(svc.events_evicted.load(Ordering::Relaxed) > 0);

        let log2 = svc
            .get_event_log(Request::new(GetEventLogRequest {
                build_id: "build-evict".to_string(),
                since_timestamp_ms: 0,
                max_events: 0,
                event_types: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        // Buffer should have shrunk, and the oldest event should be gone
        assert!(log2.total_events < MAX_EVENTS_PER_BUILD as i32);
        assert_eq!(log2.events[0].event_type, format!("event_{}", MAX_EVENTS_PER_BUILD / 2));
    }

    #[tokio::test]
    async fn test_broadcast_to_subscriber() {
        let svc = BuildEventStreamServiceImpl::new();

        // Subscribe first
        let resp = svc
            .subscribe_build_events(Request::new(SubscribeBuildEventsRequest {
                build_id: "build-live".to_string(),
                event_types: vec![],
            }))
            .await
            .unwrap();

        let mut stream = resp.into_inner();

        // Send events while subscribed
        svc.send_build_event(Request::new(SendBuildEventRequest {
            build_id: "build-live".to_string(),
            event_type: "live_event".to_string(),
            event_id: "live-1".to_string(),
            properties: Default::default(),
            display_name: "Live".to_string(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

        // Read from stream — should get the live event
        use futures_util::StreamExt;
        // The stream first replays buffered events (none), then live events
        if let Some(Ok(event)) = stream.next().await {
            assert_eq!(event.event_type, "live_event");
        }
    }

    #[tokio::test]
    async fn test_send_event_auto_timestamp() {
        let svc = BuildEventStreamServiceImpl::new();

        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        svc.send_build_event(Request::new(SendBuildEventRequest {
            build_id: "build-ts".to_string(),
            event_type: "test".to_string(),
            event_id: "ts-1".to_string(),
            properties: Default::default(),
            display_name: String::new(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

        let after = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        let log = svc
            .get_event_log(Request::new(GetEventLogRequest {
                build_id: "build-ts".to_string(),
                since_timestamp_ms: 0,
                max_events: 0,
                event_types: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(log.events[0].timestamp_ms >= before);
        assert!(log.events[0].timestamp_ms <= after);
    }

    #[tokio::test]
    async fn test_filtered_subscription() {
        let svc = BuildEventStreamServiceImpl::new();

        // Subscribe with filter
        let resp = svc
            .subscribe_build_events(Request::new(SubscribeBuildEventsRequest {
                build_id: "build-filter".to_string(),
                event_types: vec!["important".to_string()],
            }))
            .await
            .unwrap();

        let mut stream = resp.into_inner();

        // Send events of different types
        svc.send_build_event(Request::new(SendBuildEventRequest {
            build_id: "build-filter".to_string(),
            event_type: "noise".to_string(),
            event_id: "n1".to_string(),
            properties: Default::default(),
            display_name: String::new(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

        svc.send_build_event(Request::new(SendBuildEventRequest {
            build_id: "build-filter".to_string(),
            event_type: "important".to_string(),
            event_id: "i1".to_string(),
            properties: Default::default(),
            display_name: String::new(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

        use futures_util::StreamExt;
        // Should get the important event (noise filtered out)
        if let Some(Ok(event)) = stream.next().await {
            assert_eq!(event.event_type, "important");
        }
    }

    #[tokio::test]
    async fn test_counters_tracked() {
        let svc = BuildEventStreamServiceImpl::new();

        svc.send_build_event(Request::new(SendBuildEventRequest {
            build_id: "build-ct".to_string(),
            event_type: "e".to_string(),
            event_id: "c1".to_string(),
            properties: Default::default(),
            display_name: String::new(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

        svc.send_build_event(Request::new(SendBuildEventRequest {
            build_id: "build-ct".to_string(),
            event_type: "e".to_string(),
            event_id: "c2".to_string(),
            properties: Default::default(),
            display_name: String::new(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

        assert_eq!(svc.events_received.load(Ordering::Relaxed), 2);
        assert_eq!(svc.events_sent.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn test_cleanup_build() {
        let svc = BuildEventStreamServiceImpl::new();

        svc.send_build_event(Request::new(SendBuildEventRequest {
            build_id: "build-cleanup".to_string(),
            event_type: "test".to_string(),
            event_id: "e1".to_string(),
            properties: Default::default(),
            display_name: String::new(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

        assert_eq!(svc.event_buffers.len(), 1);

        // Subscribe to create the broadcast channel
        let _ = svc
            .subscribe_build_events(Request::new(SubscribeBuildEventsRequest {
                build_id: "build-cleanup".to_string(),
                event_types: vec![],
            }))
            .await;

        assert_eq!(svc.active_build_count(), 1);

        svc.cleanup_build("build-cleanup");

        assert_eq!(svc.event_buffers.len(), 0);
        assert_eq!(svc.active_build_count(), 0);
    }

    #[tokio::test]
    async fn test_multiple_builds_isolated() {
        let svc = BuildEventStreamServiceImpl::new();

        svc.send_build_event(Request::new(SendBuildEventRequest {
            build_id: "build-a".to_string(),
            event_type: "a_event".to_string(),
            event_id: "a1".to_string(),
            properties: Default::default(),
            display_name: String::new(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

        svc.send_build_event(Request::new(SendBuildEventRequest {
            build_id: "build-b".to_string(),
            event_type: "b_event".to_string(),
            event_id: "b1".to_string(),
            properties: Default::default(),
            display_name: String::new(),
            parent_id: String::new(),
        }))
        .await
        .unwrap();

        let log_a = svc
            .get_event_log(Request::new(GetEventLogRequest {
                build_id: "build-a".to_string(),
                since_timestamp_ms: 0,
                max_events: 0,
                event_types: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        let log_b = svc
            .get_event_log(Request::new(GetEventLogRequest {
                build_id: "build-b".to_string(),
                since_timestamp_ms: 0,
                max_events: 0,
                event_types: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(log_a.total_events, 1);
        assert_eq!(log_a.events[0].event_type, "a_event");
        assert_eq!(log_b.total_events, 1);
        assert_eq!(log_b.events[0].event_type, "b_event");
    }
}
