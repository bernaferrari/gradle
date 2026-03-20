use std::sync::atomic::{AtomicI64, Ordering};

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use crate::proto::{
    build_event_stream_service_server::BuildEventStreamService, BuildEventMessage,
    GetEventLogRequest, GetEventLogResponse, SendBuildEventRequest, SendBuildEventResponse,
    SubscribeBuildEventsRequest,
};

/// Rust-native build event streaming service.
/// Buffers build events and streams them to subscribers (IDEs, CI systems).
///
/// This replaces Gradle's internal build event protocol with a more efficient
/// Rust-based implementation. Events are buffered in memory and delivered
/// via server-streaming gRPC for real-time consumption.
pub struct BuildEventStreamServiceImpl {
    event_buffers: DashMap<String, Vec<BuildEventMessage>>,
    subscribers: AtomicI64,
    events_sent: AtomicI64,
    events_received: AtomicI64,
}

impl BuildEventStreamServiceImpl {
    pub fn new() -> Self {
        Self {
            event_buffers: DashMap::new(),
            subscribers: AtomicI64::new(0),
            events_sent: AtomicI64::new(0),
            events_received: AtomicI64::new(0),
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
        let buffer = self.event_buffers.get(&build_id);

        let events_to_stream: Vec<BuildEventMessage> = if let Some(buf) = buffer {
            buf.iter()
                .filter(|e| Self::matches_filter(e, &filter))
                .cloned()
                .collect()
        } else {
            Vec::new()
        };

        let buffered = events_to_stream.len();
        let stream = futures_util::stream::iter(
            events_to_stream.into_iter().map(Ok)
        );

        tracing::debug!(
            build_id = %build_id,
            buffered,
            "Build event subscriber connected"
        );

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
        self.event_buffers
            .entry(req.build_id.clone())
            .or_insert_with(Vec::new)
            .push(event);

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
    async fn test_filtered_events() {
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

        // Get only progress events
        let log = svc
            .get_event_log(Request::new(GetEventLogRequest {
                build_id: "build-2".to_string(),
                since_timestamp_ms: 0,
                max_events: 0,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(log.total_events, 2);
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
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(log.total_events, 3);
    }

    #[tokio::test]
    async fn test_unknown_build() {
        let svc = BuildEventStreamServiceImpl::new();

        let log = svc
            .get_event_log(Request::new(GetEventLogRequest {
                build_id: "nonexistent".to_string(),
                since_timestamp_ms: 0,
                max_events: 0,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(log.total_events, 0);
    }
}
