use crate::proto::BuildEventMessage;

/// Trait for services that consume build events forwarded from the event stream.
///
/// When `BuildEventStreamServiceImpl` receives a build event, it dispatches to all
/// registered `EventDispatcher` implementations. This enables automatic fan-out
/// (e.g., build events → console logs, build events → metrics) without the JVM
/// needing to make additional gRPC calls.
pub trait EventDispatcher: Send + Sync {
    /// Called when the event stream receives a build event.
    /// Implementations should match on `event.event_type` and handle only the
    /// event types they care about.
    fn dispatch_event(&self, event: &BuildEventMessage);
}
