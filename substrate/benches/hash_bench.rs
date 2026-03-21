use criterion::{black_box, criterion_group, criterion_main, Criterion};

fn bench_small_file_hash(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("small.txt");
    // 1KB of data
    let data = vec![0x42u8; 1024];
    std::fs::write(&file_path, &data).unwrap();

    c.bench_function("hash_small_file_1kb", |b| {
        b.iter(|| {
            gradle_substrate_daemon::server::hash::hash_file_md5(
                black_box(&file_path)
            ).unwrap()
        })
    });
}

fn bench_large_file_hash(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("large.bin");
    // 10MB of data
    let data = vec![0x42u8; 10 * 1024 * 1024];
    std::fs::write(&file_path, &data).unwrap();

    c.bench_function("hash_large_file_10mb", |b| {
        b.iter(|| {
            gradle_substrate_daemon::server::hash::hash_file_md5(
                black_box(&file_path)
            ).unwrap()
        })
    });
}

fn bench_batch_hash(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let paths: Vec<_> = (0..100)
        .map(|i| {
            let path = dir.path().join(format!("file_{}.txt", i));
            let data = format!("content of file {}", i);
            std::fs::write(&path, data).unwrap();
            path
        })
        .collect();

    c.bench_function("hash_batch_100_files", |b| {
        b.iter(|| {
            for path in &paths {
                let _ = gradle_substrate_daemon::server::hash::hash_file_md5(
                    black_box(path)
                ).unwrap();
            }
        })
    });
}

criterion_group!(benches, bench_small_file_hash, bench_large_file_hash, bench_batch_hash);
criterion_main!(benches);
