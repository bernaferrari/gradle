use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use crate::proto::{
    build_metrics_service_server::BuildMetricsService, GetMetricsRequest, GetMetricsResponse,
    GetPerformanceSummaryRequest, GetPerformanceSummaryResponse, MetricSnapshot,
    PerformanceSummary, RecordMetricRequest, RecordMetricResponse, ResetMetricsRequest,
    ResetMetricsResponse,
};
#[cfg(test)]
use crate::proto::MetricEvent;

/// Aggregated metric data for a single metric name.
#[derive(Default)]
struct MetricData {
    count: AtomicI64,
    sum: AtomicU64,
    min: AtomicI64,
    max: AtomicI64,
    last: AtomicI64,
    tags: std::sync::Mutex<HashMap<String, String>>,
}

impl MetricData {
    fn record(&self, value: f64, tags: HashMap<String, String>) {
        self.count.fetch_add(1, Ordering::Relaxed);
        self.sum.fetch_add(value.to_bits(), Ordering::Relaxed);
        let ival = value as i64;
        if ival < self.min.load(Ordering::Relaxed) {
            self.min.store(ival, Ordering::Relaxed);
        }
        if ival > self.max.load(Ordering::Relaxed) {
            self.max.store(ival, Ordering::Relaxed);
        }
        self.last.store(ival, Ordering::Relaxed);
        if !tags.is_empty() {
            if let Ok(mut guard) = self.tags.lock() {
                for (k, v) in &tags {
                    guard.insert(k.clone(), v.clone());
                }
            }
        }
    }
}

/// Tracks build performance metrics for monitoring and optimization.
#[derive(Default)]
pub struct BuildMetricsServiceImpl {
    metrics: DashMap<String, MetricData>,
    builds: DashMap<String, BuildSummaryData>,
}

#[derive(Default)]
struct BuildSummaryData {
    total_tasks: AtomicI64,
    cached_tasks: AtomicI64,
    up_to_date_tasks: AtomicI64,
    executed_tasks: AtomicI64,
    failed_tasks: AtomicI64,
    cache_hits: AtomicI64,
    cache_misses: AtomicI64,
    total_bytes_stored: AtomicI64,
    total_bytes_loaded: AtomicI64,
    config_cache_hits: AtomicI64,
    config_cache_misses: AtomicI64,
    history_entries_stored: AtomicI64,
    history_entries_loaded: AtomicI64,
    workers_used: AtomicI64,
    start_time_ms: AtomicI64,
    end_time_ms: AtomicI64,
    outcome: std::sync::Mutex<String>,
}

impl BuildMetricsServiceImpl {
    pub fn new() -> Self {
        Self {
            metrics: DashMap::new(),
            builds: DashMap::new(),
        }
    }

    fn ensure_metric(&self, name: &str) {
        if !self.metrics.contains_key(name) {
            self.metrics
                .insert(name.to_string(), MetricData::default());
        }
    }

    fn ensure_build(&self, build_id: &str) {
        if !self.builds.contains_key(build_id) {
            self.builds
                .insert(build_id.to_string(), BuildSummaryData::default());
        }
    }
}

#[tonic::async_trait]
impl BuildMetricsService for BuildMetricsServiceImpl {
    async fn record_metric(
        &self,
        request: Request<RecordMetricRequest>,
    ) -> Result<Response<RecordMetricResponse>, Status> {
        let req = request.into_inner();
        let event = req.event.unwrap_or_default();

        let value: f64 = event.value.parse().unwrap_or(0.0);
        let tags: HashMap<String, String> = event.tags.into_iter().collect();

        self.ensure_metric(&event.name);
        if let Some(metric) = self.metrics.get(&event.name) {
            metric.record(value, tags);
        }

        // Also update build summary if this is a known build metric
        if !req.build_id.is_empty() {
            self.ensure_build(&req.build_id);
            if let Some(build) = self.builds.get(&req.build_id) {
                match event.name.as_str() {
                    "tasks.total" => { build.total_tasks.fetch_add(1, Ordering::Relaxed); }
                    "tasks.cached" => { build.cached_tasks.fetch_add(1, Ordering::Relaxed); }
                    "tasks.up_to_date" => { build.up_to_date_tasks.fetch_add(1, Ordering::Relaxed); }
                    "tasks.executed" => { build.executed_tasks.fetch_add(1, Ordering::Relaxed); }
                    "tasks.failed" => { build.failed_tasks.fetch_add(1, Ordering::Relaxed); }
                    "cache.hits" => { build.cache_hits.fetch_add(1, Ordering::Relaxed); }
                    "cache.misses" => { build.cache_misses.fetch_add(1, Ordering::Relaxed); }
                    "cache.bytes_stored" => {
                        build.total_bytes_stored.fetch_add(value as i64, Ordering::Relaxed);
                    }
                    "cache.bytes_loaded" => {
                        build.total_bytes_loaded.fetch_add(value as i64, Ordering::Relaxed);
                    }
                    "config_cache.hits" => { build.config_cache_hits.fetch_add(1, Ordering::Relaxed); }
                    "config_cache.misses" => { build.config_cache_misses.fetch_add(1, Ordering::Relaxed); }
                    "history.stored" => { build.history_entries_stored.fetch_add(1, Ordering::Relaxed); }
                    "history.loaded" => { build.history_entries_loaded.fetch_add(1, Ordering::Relaxed); }
                    "workers.used" => { build.workers_used.fetch_add(1, Ordering::Relaxed); }
                    "build.start" => {
                        build.start_time_ms.store(event.timestamp_ms, Ordering::Relaxed);
                    }
                    "build.end" => {
                        build.end_time_ms.store(event.timestamp_ms, Ordering::Relaxed);
                        if !event.value.is_empty() {
                            if let Ok(mut guard) = build.outcome.lock() {
                                *guard = event.value.clone();
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(Response::new(RecordMetricResponse { recorded: true }))
    }

    async fn get_metrics(
        &self,
        request: Request<GetMetricsRequest>,
    ) -> Result<Response<GetMetricsResponse>, Status> {
        let req = request.into_inner();
        let name_filter: Vec<&str> = req.metric_names.iter().map(|s| s.as_str()).collect();

        let snapshots: Vec<MetricSnapshot> = self
            .metrics
            .iter()
            .filter(|entry| {
                if name_filter.is_empty() {
                    return true;
                }
                name_filter.contains(&entry.key().as_str())
            })
            .map(|entry| {
                let data = entry.value();
                MetricSnapshot {
                    name: entry.key().clone(),
                    metric_type: "counter".to_string(),
                    count: data.count.load(Ordering::Relaxed),
                    sum: f64::from_bits(data.sum.load(Ordering::Relaxed)),
                    min: data.min.load(Ordering::Relaxed) as f64,
                    max: data.max.load(Ordering::Relaxed) as f64,
                    last: data.last.load(Ordering::Relaxed) as f64,
                    tags: data.tags.lock().map(|g| g.clone()).unwrap_or_default(),
                }
            })
            .collect();

        Ok(Response::new(GetMetricsResponse { metrics: snapshots }))
    }

    async fn get_performance_summary(
        &self,
        request: Request<GetPerformanceSummaryRequest>,
    ) -> Result<Response<GetPerformanceSummaryResponse>, Status> {
        let req = request.into_inner();

        if let Some(build) = self.builds.get(&req.build_id) {
            let total_tasks = build.total_tasks.load(Ordering::Relaxed);
            let cache_hits = build.cache_hits.load(Ordering::Relaxed);
            let cache_misses = build.cache_misses.load(Ordering::Relaxed);
            let total = cache_hits + cache_misses;
            let hit_rate = if total > 0 {
                cache_hits as f64 / total as f64
            } else {
                0.0
            };

            let start_ms = build.start_time_ms.load(Ordering::Relaxed);
            let end_ms = build.end_time_ms.load(Ordering::Relaxed);
            let duration = if end_ms > start_ms {
                end_ms - start_ms
            } else {
                0
            };

            let outcome = build.outcome.lock().map(|g| g.clone()).unwrap_or_default();
            if outcome.is_empty() {
                let failed = build.failed_tasks.load(Ordering::Relaxed);
                let inferred = if failed > 0 {
                    "FAILED"
                } else if total_tasks > 0 {
                    "SUCCESS"
                } else {
                    "UNKNOWN"
                };
                if let Ok(mut guard) = build.outcome.lock() {
                    *guard = inferred.to_string();
                }
            }

            Ok(Response::new(GetPerformanceSummaryResponse {
                summary: Some(PerformanceSummary {
                    build_id: req.build_id,
                    start_time_ms: start_ms,
                    end_time_ms: end_ms,
                    duration_ms: duration,
                    total_tasks_executed: total_tasks as i32,
                    tasks_from_cache: build.cached_tasks.load(Ordering::Relaxed) as i32,
                    tasks_up_to_date: build.up_to_date_tasks.load(Ordering::Relaxed) as i32,
                    tasks_executed: build.executed_tasks.load(Ordering::Relaxed) as i32,
                    tasks_failed: build.failed_tasks.load(Ordering::Relaxed) as i32,
                    build_cache_hits: cache_hits,
                    build_cache_misses: cache_misses,
                    build_cache_hit_rate: hit_rate,
                    total_bytes_stored: build.total_bytes_stored.load(Ordering::Relaxed),
                    total_bytes_loaded: build.total_bytes_loaded.load(Ordering::Relaxed),
                    config_cache_hits: build.config_cache_hits.load(Ordering::Relaxed) as i32,
                    config_cache_misses: build.config_cache_misses.load(Ordering::Relaxed) as i32,
                    history_stored: build.history_entries_stored.load(Ordering::Relaxed) as i32,
                    history_loaded: build.history_entries_loaded.load(Ordering::Relaxed) as i32,
                    workers_used: build.workers_used.load(Ordering::Relaxed) as i32,
                    build_outcome: build.outcome.lock().map(|g| g.clone()).unwrap_or_default(),
                }),
            }))
        } else {
            Ok(Response::new(GetPerformanceSummaryResponse {
                summary: Some(PerformanceSummary::default()),
            }))
        }
    }

    async fn reset_metrics(
        &self,
        request: Request<ResetMetricsRequest>,
    ) -> Result<Response<ResetMetricsResponse>, Status> {
        let req = request.into_inner();

        if req.build_id.is_empty() {
            // Reset all metrics
            let count = self.metrics.len();
            self.metrics.clear();
            self.builds.clear();
            Ok(Response::new(ResetMetricsResponse {
                reset: true,
                metrics_cleared: count as i32,
            }))
        } else {
            // Reset only for specific build
            let cleared = if self.builds.remove(&req.build_id).is_some() {
                1
            } else {
                0
            };
            Ok(Response::new(ResetMetricsResponse {
                reset: true,
                metrics_cleared: cleared,
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_svc() -> BuildMetricsServiceImpl {
        BuildMetricsServiceImpl::new()
    }

    fn metric_event(name: &str, value: &str) -> MetricEvent {
        MetricEvent {
            name: name.to_string(),
            value: value.to_string(),
            metric_type: "counter".to_string(),
            tags: HashMap::new(),
            timestamp_ms: 1000,
        }
    }

    #[tokio::test]
    async fn test_record_and_get_metrics() {
        let svc = make_svc();

        svc.record_metric(Request::new(RecordMetricRequest {
            build_id: "build1".to_string(),
            event: Some(metric_event("cache.hits", "1")),
        }))
        .await
        .unwrap();

        svc.record_metric(Request::new(RecordMetricRequest {
            build_id: "build1".to_string(),
            event: Some(metric_event("cache.hits", "1")),
        }))
        .await
        .unwrap();

        svc.record_metric(Request::new(RecordMetricRequest {
            build_id: "build1".to_string(),
            event: Some(metric_event("cache.misses", "1")),
        }))
        .await
        .unwrap();

        let resp = svc
            .get_metrics(Request::new(GetMetricsRequest {
                build_id: "build1".to_string(),
                metric_names: vec!["cache.hits".to_string()],
                since_ms: 0,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.metrics.len(), 1);
        assert_eq!(resp.metrics[0].name, "cache.hits");
        assert_eq!(resp.metrics[0].count, 2);
    }

    #[tokio::test]
    async fn test_build_summary() {
        let svc = make_svc();

        svc.record_metric(Request::new(RecordMetricRequest {
            build_id: "build1".to_string(),
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

        // Record 10 tasks, 5 cached, 3 executed, 1 failed
        for _ in 0..10 {
            svc.record_metric(Request::new(RecordMetricRequest {
                build_id: "build1".to_string(),
                event: Some(metric_event("tasks.total", "1")),
            }))
            .await
            .unwrap();
        }
        for _ in 0..5 {
            svc.record_metric(Request::new(RecordMetricRequest {
                build_id: "build1".to_string(),
                event: Some(metric_event("tasks.cached", "1")),
            }))
            .await
            .unwrap();
        }
        for _ in 0..3 {
            svc.record_metric(Request::new(RecordMetricRequest {
                build_id: "build1".to_string(),
                event: Some(metric_event("tasks.executed", "1")),
            }))
            .await
            .unwrap();
        }
        for _ in 0..1 {
            svc.record_metric(Request::new(RecordMetricRequest {
                build_id: "build1".to_string(),
                event: Some(metric_event("tasks.failed", "1")),
            }))
            .await
            .unwrap();
        }

        svc.record_metric(Request::new(RecordMetricRequest {
            build_id: "build1".to_string(),
            event: Some(MetricEvent {
                name: "build.end".to_string(),
                value: "FAILED".to_string(),
                metric_type: "timer".to_string(),
                tags: HashMap::new(),
                timestamp_ms: 5000,
            }),
        }))
        .await
        .unwrap();

        let resp = svc
            .get_performance_summary(Request::new(GetPerformanceSummaryRequest {
                build_id: "build1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        let summary = resp.summary.unwrap();
        assert_eq!(summary.total_tasks_executed, 10);
        assert_eq!(summary.tasks_from_cache, 5);
        assert_eq!(summary.tasks_executed, 3);
        assert_eq!(summary.tasks_failed, 1);
        assert_eq!(summary.duration_ms, 4000);
        assert_eq!(summary.build_outcome, "FAILED");
    }

    #[tokio::test]
    async fn test_reset_metrics() {
        let svc = make_svc();

        svc.record_metric(Request::new(RecordMetricRequest {
            build_id: "build1".to_string(),
            event: Some(metric_event("test.metric", "42")),
        }))
        .await
        .unwrap();

        let resp = svc
            .reset_metrics(Request::new(ResetMetricsRequest {
                build_id: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.reset);
        assert_eq!(resp.metrics_cleared, 1);
    }
}
