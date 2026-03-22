use std::collections::HashMap;

use criterion::{criterion_group, criterion_main, Criterion};
use gradle_substrate_daemon::proto::{
    build_cache_orchestration_service_server::BuildCacheOrchestrationService,
    ComputeCacheKeyRequest,
};
use gradle_substrate_daemon::server::cache_orchestration::BuildCacheOrchestrationServiceImpl;
use tonic::Request;

fn make_props(n: usize) -> HashMap<String, String> {
    (0..n).map(|i| (format!("prop_{}", i), format!("hash_{}", i))).collect()
}

fn make_file_hashes(n: usize) -> HashMap<String, String> {
    (0..n).map(|i| (format!("file_{}", i), format!("fhash_{}", i))).collect()
}

fn bench_compute_cache_key(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let svc = BuildCacheOrchestrationServiceImpl::new();

    c.bench_function("compute_cache_key_10props_5files", |b| {
        b.iter(|| {
            rt.block_on(async {
                svc.compute_cache_key(Request::new(ComputeCacheKeyRequest {
                    work_identity: ":compileJava".to_string(),
                    implementation_hash: "impl-abc-123".to_string(),
                    input_property_hashes: make_props(10),
                    input_file_hashes: make_file_hashes(5),
                    output_property_names: vec!["classes".to_string(), "resources".to_string()],
                })).await.unwrap()
            })
        })
    });
}

fn bench_compute_cache_key_large(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let svc = BuildCacheOrchestrationServiceImpl::new();

    c.bench_function("compute_cache_key_100props_50files", |b| {
        b.iter(|| {
            rt.block_on(async {
                svc.compute_cache_key(Request::new(ComputeCacheKeyRequest {
                    work_identity: ":compileJava".to_string(),
                    implementation_hash: "impl-abc-123".to_string(),
                    input_property_hashes: make_props(100),
                    input_file_hashes: make_file_hashes(50),
                    output_property_names: (0..20).map(|i| format!("output_{}", i)).collect(),
                })).await.unwrap()
            })
        })
    });
}

criterion_group!(benches, bench_compute_cache_key, bench_compute_cache_key_large);
criterion_main!(benches);
