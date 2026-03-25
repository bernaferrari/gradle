use criterion::{black_box, criterion_group, criterion_main, Criterion};
use gradle_substrate_daemon::server::cache::LocalCacheStore;
use tokio::runtime::Runtime;

fn bench_cache_store(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = LocalCacheStore::new(dir.path().to_path_buf());
    // 1MB of data
    let data = vec![0x42u8; 1024 * 1024];
    let key = "bench-store-key-abc123";

    c.bench_function("cache_store_1mb", |b| {
        b.iter(|| {
            rt.block_on(async { store.store(black_box(key), black_box(&data)).await.unwrap() })
        })
    });
}

fn bench_cache_load(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = LocalCacheStore::new(dir.path().to_path_buf());
    let data = vec![0x42u8; 1024 * 1024];
    let key = "bench-load-key-abc123";
    rt.block_on(store.store(key, &data)).unwrap();

    c.bench_function("cache_load_1mb", |b| {
        b.iter(|| rt.block_on(async { store.load(black_box(key)).await.unwrap() }))
    });
}

fn bench_cache_store_and_load(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let store = LocalCacheStore::new(dir.path().to_path_buf());
    let data = vec![0x42u8; 1024 * 1024];

    let mut counter = 0u64;
    c.bench_function("cache_store_and_load_1mb", |b| {
        b.iter(|| {
            counter += 1;
            let key = format!("bench-cycle-key-{}", counter);
            rt.block_on(async {
                store.store(&key, black_box(&data)).await.unwrap();
                store.load(&key).await.unwrap()
            })
        })
    });
}

criterion_group!(
    benches,
    bench_cache_store,
    bench_cache_load,
    bench_cache_store_and_load
);
criterion_main!(benches);
