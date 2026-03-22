use std::sync::Arc;

use criterion::{criterion_group, criterion_main, Criterion};
use gradle_substrate_daemon::proto::{
    build_event_stream_service_server::BuildEventStreamService, SendBuildEventRequest,
};
use gradle_substrate_daemon::server::build_event_stream::BuildEventStreamServiceImpl;
use gradle_substrate_daemon::server::build_metrics::BuildMetricsServiceImpl;
use gradle_substrate_daemon::server::console::ConsoleServiceImpl;
use gradle_substrate_daemon::server::event_dispatcher::EventDispatcher;
use tonic::Request;

fn bench_send_event_no_dispatchers(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let svc = BuildEventStreamServiceImpl::new();

    c.bench_function("send_event_no_dispatchers", |b| {
        b.iter(|| {
            rt.block_on(async {
                svc.send_build_event(Request::new(SendBuildEventRequest {
                    build_id: "bench-build".to_string(),
                    event_type: "task_start".to_string(),
                    event_id: "evt-1".to_string(),
                    properties: Default::default(),
                    display_name: ":compileJava".to_string(),
                    parent_id: String::new(),
                }))
                .await
                .unwrap()
            })
        })
    });
}

fn bench_send_event_with_dispatchers(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dispatchers: Vec<Arc<dyn EventDispatcher>> = vec![
        Arc::new(BuildMetricsServiceImpl::new()) as Arc<dyn EventDispatcher>,
        Arc::new(ConsoleServiceImpl::new()) as Arc<dyn EventDispatcher>,
    ];
    let svc = BuildEventStreamServiceImpl::with_dispatchers(dispatchers);

    c.bench_function("send_event_with_2_dispatchers", |b| {
        b.iter(|| {
            rt.block_on(async {
                svc.send_build_event(Request::new(SendBuildEventRequest {
                    build_id: "bench-build".to_string(),
                    event_type: "task_start".to_string(),
                    event_id: "evt-1".to_string(),
                    properties: Default::default(),
                    display_name: ":compileJava".to_string(),
                    parent_id: String::new(),
                }))
                .await
                .unwrap()
            })
        })
    });
}

fn bench_send_1000_events_with_dispatchers(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dispatchers: Vec<Arc<dyn EventDispatcher>> = vec![
        Arc::new(BuildMetricsServiceImpl::new()) as Arc<dyn EventDispatcher>,
        Arc::new(ConsoleServiceImpl::new()) as Arc<dyn EventDispatcher>,
    ];
    let svc = BuildEventStreamServiceImpl::with_dispatchers(dispatchers);

    // Pre-generate requests
    let requests: Vec<SendBuildEventRequest> = (0..1000)
        .map(|i| SendBuildEventRequest {
            build_id: "bench-build".to_string(),
            event_type: if i % 2 == 0 {
                "task_start".to_string()
            } else {
                "task_finish".to_string()
            },
            event_id: format!("evt-{}", i),
            properties: if i % 2 != 0 {
                let mut props = std::collections::HashMap::new();
                props.insert("outcome".to_string(), "SUCCESS".to_string());
                props
            } else {
                std::collections::HashMap::new()
            },
            display_name: format!(":task_{}", i),
            parent_id: String::new(),
        })
        .collect();

    c.bench_function("send_1000_events_with_dispatchers", |b| {
        b.iter(|| {
            rt.block_on(async {
                for req in &requests {
                    let _ = svc.send_build_event(Request::new(req.clone())).await.unwrap();
                }
            })
        })
    });
}

criterion_group!(
    benches,
    bench_send_event_no_dispatchers,
    bench_send_event_with_dispatchers,
    bench_send_1000_events_with_dispatchers
);
criterion_main!(benches);
