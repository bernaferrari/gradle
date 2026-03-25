use std::sync::Arc;

use criterion::{criterion_group, criterion_main, Criterion};
use gradle_substrate_daemon::proto::{
    execution_plan_service_server::ExecutionPlanService, PredictOutcomeRequest, ResolvePlanRequest,
    WorkMetadata,
};
use gradle_substrate_daemon::server::execution_plan::ExecutionPlanServiceImpl;
use gradle_substrate_daemon::server::work::WorkerScheduler;
use tonic::Request;

fn make_work_metadata(
    identity: &str,
    props: Vec<(&str, &str)>,
    file_fps: Vec<(&str, &str)>,
) -> WorkMetadata {
    WorkMetadata {
        work_identity: identity.to_string(),
        display_name: identity.to_string(),
        implementation_class: "com.example.FakeTask".to_string(),
        input_properties: props
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        input_file_fingerprints: file_fps
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        caching_enabled: true,
        can_load_from_cache: true,
        has_previous_execution_state: false,
        rebuild_reasons: vec![],
    }
}

fn bench_predict_outcome(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let scheduler = Arc::new(WorkerScheduler::new(4));
    let svc = ExecutionPlanServiceImpl::new(scheduler);

    // Seed history
    let work = make_work_metadata(":compileJava", vec![("source", "src/main/java")], vec![]);
    rt.block_on(async {
        let _ = svc
            .predict_outcome(Request::new(PredictOutcomeRequest { work: Some(work) }))
            .await;
    });

    c.bench_function("predict_outcome_with_history", |b| {
        b.iter(|| {
            let work =
                make_work_metadata(":compileJava", vec![("source", "src/main/java")], vec![]);
            rt.block_on(async {
                svc.predict_outcome(Request::new(PredictOutcomeRequest { work: Some(work) }))
                    .await
                    .unwrap()
            })
        })
    });
}

fn bench_predict_outcome_no_history(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let scheduler = Arc::new(WorkerScheduler::new(4));
    let svc = ExecutionPlanServiceImpl::new(scheduler);

    c.bench_function("predict_outcome_no_history", |b| {
        b.iter(|| {
            let work =
                make_work_metadata(":compileJava", vec![("source", "src/main/java")], vec![]);
            rt.block_on(async {
                svc.predict_outcome(Request::new(PredictOutcomeRequest { work: Some(work) }))
                    .await
                    .unwrap()
            })
        })
    });
}

fn bench_resolve_plan(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let scheduler = Arc::new(WorkerScheduler::new(4));
    let svc = ExecutionPlanServiceImpl::new(scheduler);

    // Seed history
    let work = make_work_metadata(
        ":compileJava",
        vec![("source", "v1")],
        vec![("classpath", "fp1")],
    );
    rt.block_on(async {
        let _ = svc
            .predict_outcome(Request::new(PredictOutcomeRequest { work: Some(work) }))
            .await;
    });

    c.bench_function("resolve_plan_with_cached_fp", |b| {
        b.iter(|| {
            let work = make_work_metadata(
                ":compileJava",
                vec![("source", "v1")],
                vec![("classpath", "fp1")],
            );
            rt.block_on(async {
                svc.resolve_plan(Request::new(ResolvePlanRequest {
                    work: Some(work),
                    authoritative: true,
                }))
                .await
                .unwrap()
            })
        })
    });
}

criterion_group!(
    benches,
    bench_predict_outcome,
    bench_predict_outcome_no_history,
    bench_resolve_plan
);
criterion_main!(benches);
