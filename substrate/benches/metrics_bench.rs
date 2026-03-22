use std::collections::HashMap;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use gradle_substrate_daemon::proto::{
    build_metrics_service_server::BuildMetricsService, GetMetricsRequest, MetricEvent,
    RecordMetricRequest,
};
use gradle_substrate_daemon::server::build_metrics::BuildMetricsServiceImpl;
use tonic::Request;

fn bench_record_10k_metrics(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let svc = BuildMetricsServiceImpl::new();

    // Pre-generate metric requests
    let requests: Vec<RecordMetricRequest> = (0..10_000)
        .map(|i| RecordMetricRequest {
            build_id: "bench-build".to_string(),
            event: Some(MetricEvent {
                name: format!("task.{}", i % 20),
                value: (i as i64).to_string(),
                metric_type: "counter".to_string(),
                tags: HashMap::new(),
                timestamp_ms: 1000 + i,
            }),
        })
        .collect();

    c.bench_function("record_10k_metrics", |b| {
        b.iter(|| {
            rt.block_on(async {
                for req in &requests {
                    let _ = svc.record_metric(Request::new(req.clone())).await.unwrap();
                }
            })
        })
    });
}

fn bench_record_metric_direct(c: &mut Criterion) {
    let svc = BuildMetricsServiceImpl::new();
    let build_id = gradle_substrate_daemon::server::scopes::BuildId::from("bench-build".to_string());

    c.bench_function("record_metric_direct_10k", |b| {
        b.iter(|| {
            for i in 0..10_000i32 {
                svc.record_metric_direct(
                    &build_id,
                    black_box(&format!("task.{}", i % 20)),
                    black_box(i as f64),
                    black_box(1000 + i as i64),
                );
            }
        })
    });
}

fn bench_get_filtered_metrics(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let svc = BuildMetricsServiceImpl::new();

    // Seed 1000 metrics across 5 builds
    rt.block_on(async {
        for build in 0..5 {
            for i in 0..200 {
                let _ = svc
                    .record_metric(Request::new(RecordMetricRequest {
                        build_id: format!("build-{}", build),
                        event: Some(MetricEvent {
                            name: format!("metric_{}", i % 10),
                            value: "42".to_string(),
                            metric_type: "counter".to_string(),
                            tags: HashMap::new(),
                            timestamp_ms: 1000 + i,
                        }),
                    }))
                    .await
                    .unwrap();
            }
        }
    });

    let filter_names: Vec<String> = (0..5).map(|i| format!("metric_{}", i)).collect();

    c.bench_function("get_filtered_metrics_5_of_10_names", |b| {
        b.iter(|| {
            rt.block_on(async {
                svc.get_metrics(Request::new(GetMetricsRequest {
                    build_id: "build-2".to_string(),
                    metric_names: filter_names.clone(),
                    since_ms: 0,
                }))
                .await
                .unwrap()
            })
        })
    });
}

fn bench_get_performance_summary(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let svc = BuildMetricsServiceImpl::new();

    // Seed a build with many metrics
    rt.block_on(async {
        for _ in 0..100 {
            let _ = svc
                .record_metric(Request::new(RecordMetricRequest {
                    build_id: "bench-build".to_string(),
                    event: Some(MetricEvent {
                        name: "tasks.total".to_string(),
                        value: "1".to_string(),
                        metric_type: "counter".to_string(),
                        tags: HashMap::new(),
                        timestamp_ms: 1000,
                    }),
                }))
                .await
                .unwrap();
        }
        for _ in 0..60 {
            let _ = svc
                .record_metric(Request::new(RecordMetricRequest {
                    build_id: "bench-build".to_string(),
                    event: Some(MetricEvent {
                        name: "cache.hits".to_string(),
                        value: "1".to_string(),
                        metric_type: "counter".to_string(),
                        tags: HashMap::new(),
                        timestamp_ms: 1000,
                    }),
                }))
                .await
                .unwrap();
        }
        let _ = svc
            .record_metric(Request::new(RecordMetricRequest {
                build_id: "bench-build".to_string(),
                event: Some(MetricEvent {
                    name: "build.start".to_string(),
                    value: "0".to_string(),
                    metric_type: "timer".to_string(),
                    tags: HashMap::new(),
                    timestamp_ms: 1000,
                }),
            }))
            .await
            .unwrap();
        let _ = svc
            .record_metric(Request::new(RecordMetricRequest {
                build_id: "bench-build".to_string(),
                event: Some(MetricEvent {
                    name: "build.end".to_string(),
                    value: "SUCCESS".to_string(),
                    metric_type: "timer".to_string(),
                    tags: HashMap::new(),
                    timestamp_ms: 5000,
                }),
            }))
            .await
            .unwrap();
    });

    use gradle_substrate_daemon::proto::GetPerformanceSummaryRequest;
    c.bench_function("get_performance_summary", |b| {
        b.iter(|| {
            rt.block_on(async {
                svc.get_performance_summary(Request::new(GetPerformanceSummaryRequest {
                    build_id: "bench-build".to_string(),
                }))
                .await
                .unwrap()
            })
        })
    });
}

criterion_group!(
    benches,
    bench_record_10k_metrics,
    bench_record_metric_direct,
    bench_get_filtered_metrics,
    bench_get_performance_summary
);
criterion_main!(benches);
