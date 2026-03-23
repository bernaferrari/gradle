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

    c.bench_function("hash_batch_100_files_sequential", |b| {
        b.iter(|| {
            for path in &paths {
                let _ = gradle_substrate_daemon::server::hash::hash_file_md5(
                    black_box(path)
                ).unwrap();
            }
        })
    });
}

fn bench_batch_hash_parallel(c: &mut Criterion) {
    use gradle_substrate_daemon::server::hash::{hash_batch_parallel, HashAlgorithm};

    let dir = tempfile::tempdir().unwrap();
    let paths: Vec<_> = (0..100)
        .map(|i| {
            let path = dir.path().join(format!("file_{}.txt", i));
            let data = format!("content of file {}", i);
            std::fs::write(&path, data).unwrap();
            path
        })
        .collect();

    c.bench_function("hash_batch_100_files_parallel", |b| {
        b.iter(|| {
            let _ = hash_batch_parallel(black_box(&paths), HashAlgorithm::Md5);
        })
    });
}

fn bench_blake3_large_file(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("blake3_large.bin");
    let data = vec![0x42u8; 10 * 1024 * 1024];
    std::fs::write(&file_path, &data).unwrap();

    c.bench_function("blake3_hash_large_file_10mb", |b| {
        b.iter(|| {
            gradle_substrate_daemon::server::hash::hash_file_blake3(
                black_box(&file_path)
            ).unwrap()
        })
    });
}

fn bench_sha3_256_large_file(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("sha3_large.bin");
    let data = vec![0x42u8; 10 * 1024 * 1024];
    std::fs::write(&file_path, &data).unwrap();

    c.bench_function("sha3_256_hash_large_file_10mb", |b| {
        b.iter(|| {
            gradle_substrate_daemon::server::hash::hash_file_sha3_256(
                black_box(&file_path)
            ).unwrap()
        })
    });
}

fn bench_all_algorithms_small(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("algo_test.bin");
    let data = vec![0x42u8; 1024];
    std::fs::write(&file_path, &data).unwrap();

    let mut group = c.benchmark_group("algorithm_comparison_1kb");
    group.bench_function("md5", |b| {
        b.iter(|| gradle_substrate_daemon::server::hash::hash_file_md5(black_box(&file_path)).unwrap())
    });
    group.bench_function("sha1", |b| {
        b.iter(|| gradle_substrate_daemon::server::hash::hash_file_sha1(black_box(&file_path)).unwrap())
    });
    group.bench_function("sha256", |b| {
        b.iter(|| gradle_substrate_daemon::server::hash::hash_file_sha256(black_box(&file_path)).unwrap())
    });
    group.bench_function("sha3_256", |b| {
        b.iter(|| gradle_substrate_daemon::server::hash::hash_file_sha3_256(black_box(&file_path)).unwrap())
    });
    group.bench_function("sha3_512", |b| {
        b.iter(|| gradle_substrate_daemon::server::hash::hash_file_sha3_512(black_box(&file_path)).unwrap())
    });
    group.bench_function("blake3", |b| {
        b.iter(|| gradle_substrate_daemon::server::hash::hash_file_blake3(black_box(&file_path)).unwrap())
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_small_file_hash,
    bench_large_file_hash,
    bench_batch_hash,
    bench_batch_hash_parallel,
    bench_blake3_large_file,
    bench_sha3_256_large_file,
    bench_all_algorithms_small,
);
criterion_main!(benches);
