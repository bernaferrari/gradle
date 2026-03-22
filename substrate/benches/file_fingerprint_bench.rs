use criterion::{criterion_group, criterion_main, Criterion};
use gradle_substrate_daemon::proto::{
    file_fingerprint_service_server::FileFingerprintService,
    FileToFingerprint, FingerprintFilesRequest, FingerprintType,
};
use gradle_substrate_daemon::server::file_fingerprint::FileFingerprintServiceImpl;
use tonic::Request;

fn bench_fingerprint_single_file(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let svc = FileFingerprintServiceImpl::new();

    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("single.txt");
    std::fs::write(&file_path, b"content for fingerprint benchmark test").unwrap();
    let file_path_str = file_path.to_string_lossy().to_string();

    c.bench_function("fingerprint_single_file", |b| {
        b.iter(|| {
            rt.block_on(async {
                svc.fingerprint_files(Request::new(FingerprintFilesRequest {
                    files: vec![FileToFingerprint {
                        absolute_path: file_path_str.clone(),
                        r#type: FingerprintType::FingerprintFile as i32,
                    }],
                    normalization_strategy: "ABSOLUTE_PATH".to_string(),
                    ignore_patterns: vec![],
                })).await.unwrap()
            })
        })
    });
}

fn bench_fingerprint_directory(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let svc = FileFingerprintServiceImpl::new();

    let dir = tempfile::tempdir().unwrap();
    for i in 0..50 {
        let file_path = dir.path().join(format!("file_{}.txt", i));
        std::fs::write(&file_path, format!("content of file {}", i)).unwrap();
    }
    let dir_str = dir.path().to_string_lossy().to_string();

    c.bench_function("fingerprint_directory_50_files", |b| {
        b.iter(|| {
            rt.block_on(async {
                svc.fingerprint_files(Request::new(FingerprintFilesRequest {
                    files: vec![FileToFingerprint {
                        absolute_path: dir_str.clone(),
                        r#type: FingerprintType::FingerprintDirectory as i32,
                    }],
                    normalization_strategy: "ABSOLUTE_PATH".to_string(),
                    ignore_patterns: vec![],
                })).await.unwrap()
            })
        })
    });
}

criterion_group!(benches, bench_fingerprint_single_file, bench_fingerprint_directory);
criterion_main!(benches);
