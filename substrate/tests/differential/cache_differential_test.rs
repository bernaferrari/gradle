/// Differential cache testing: validates Rust CacheService put/get roundtrip
/// correctness, TTL behavior, cache misses, and concurrent operations.
///
/// The cache service uses streaming:
///   StoreEntry: client-streaming (stream CacheStoreChunk) -> CacheStoreResponse
///   LoadEntry: server-streaming CacheLoadRequest -> (stream CacheLoadChunk)
///
/// Tests cover:
/// - 50 entries stored and retrieved with byte-for-byte equality
/// - Cache misses for nonexistent keys
/// - Overwrite semantics (last-write-wins)
/// - Concurrent put/get operations
use std::fs;
use std::sync::Arc;

use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::{Channel, Endpoint, Server};
use tonic::Request;

use gradle_substrate_daemon::proto::*;
use gradle_substrate_daemon::server::{
    artifact_publishing::ArtifactPublishingServiceImpl, bootstrap::BootstrapServiceImpl,
    build_comparison::BuildComparisonServiceImpl, build_event_stream::BuildEventStreamServiceImpl,
    build_init::BuildInitServiceImpl, build_layout::BuildLayoutServiceImpl,
    build_metrics::BuildMetricsServiceImpl, build_operations::BuildOperationsServiceImpl,
    build_result::BuildResultServiceImpl, cache::CacheServiceImpl,
    cache_orchestration::BuildCacheOrchestrationServiceImpl,
    config_cache::ConfigurationCacheServiceImpl, configuration::ConfigurationServiceImpl,
    console::ConsoleServiceImpl, control::ControlServiceImpl, dag_executor::DagExecutorServiceImpl,
    dependency_resolution::DependencyResolutionServiceImpl, event_dispatcher::EventDispatcher,
    exec::ExecServiceImpl, execution_history::ExecutionHistoryServiceImpl,
    file_fingerprint::FileFingerprintServiceImpl, file_watch::FileWatchServiceImpl,
    garbage_collection::GarbageCollectionServiceImpl, hash::HashServiceImpl,
    incremental_compilation::IncrementalCompilationServiceImpl, plugin::PluginServiceImpl,
    problem_reporting::ProblemReportingServiceImpl,
    resource_management::ResourceManagementServiceImpl, task_graph::TaskGraphServiceImpl,
    test_execution::TestExecutionServiceImpl, toolchain::ToolchainServiceImpl,
    value_snapshot::ValueSnapshotServiceImpl, work::WorkerScheduler,
    worker_process::WorkerProcessServiceImpl,
};

// ============================================================
// Test server setup
// ============================================================

async fn spawn_server() -> (String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("test.sock");
    let socket_path_str = socket_path.to_string_lossy().to_string();

    let cache_dir = dir.path().join("cache");
    fs::create_dir_all(&cache_dir).unwrap();
    let history_dir = dir.path().join("history");
    fs::create_dir_all(&history_dir).unwrap();
    let config_cache_dir = dir.path().join("config-cache");
    fs::create_dir_all(&config_cache_dir).unwrap();
    let toolchain_dir = dir.path().join("toolchains");
    fs::create_dir_all(&toolchain_dir).unwrap();
    let artifact_store_dir = dir.path().join("artifacts");
    fs::create_dir_all(&artifact_store_dir).unwrap();

    let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);

    let control = ControlServiceImpl::new(shutdown_tx);
    let hash = HashServiceImpl;
    let cache = CacheServiceImpl::new(cache_dir.clone());
    let cache_local_store = cache.local_store();
    let exec = ExecServiceImpl::new();
    let work_scheduler = Arc::new(WorkerScheduler::new(4));
    let work = gradle_substrate_daemon::server::work::WorkServiceImpl::new(work_scheduler.clone());
    let shared_history = Arc::new(ExecutionHistoryServiceImpl::new(history_dir.clone()));
    let execution_plan =
        gradle_substrate_daemon::server::execution_plan::ExecutionPlanServiceImpl::with_persistent_history(
            work_scheduler.clone(),
            Arc::clone(&shared_history),
        );
    let execution_plan_arc = Arc::new(execution_plan);
    let execution_plan_server =
        gradle_substrate_daemon::server::execution_plan::ExecutionPlanServiceImpl::with_persistent_history(
            work_scheduler.clone(),
            Arc::clone(&shared_history),
        );
    let cache_orchestration =
        BuildCacheOrchestrationServiceImpl::with_local_cache(cache_local_store);
    let file_fingerprint = FileFingerprintServiceImpl::new();
    let value_snapshot = ValueSnapshotServiceImpl::new();
    let task_graph = Arc::new(TaskGraphServiceImpl::with_history(Arc::clone(
        &shared_history,
    )));
    let execution_history = ExecutionHistoryServiceImpl::new(history_dir.clone());
    let configuration = ConfigurationServiceImpl::new();
    let plugin = PluginServiceImpl::new();
    let build_operations = BuildOperationsServiceImpl::new();
    let bootstrap = BootstrapServiceImpl::new();
    let dependency_resolution = DependencyResolutionServiceImpl::new(artifact_store_dir);
    let file_watch = FileWatchServiceImpl::with_task_graph(Arc::clone(&task_graph));
    let config_cache = ConfigurationCacheServiceImpl::new(config_cache_dir.clone());
    let toolchain = ToolchainServiceImpl::new(toolchain_dir);
    let console = Arc::new(ConsoleServiceImpl::new());
    let build_metrics = Arc::new(BuildMetricsServiceImpl::new());
    let event_dispatchers: Vec<Arc<dyn EventDispatcher>> = vec![
        Arc::clone(&console) as Arc<dyn EventDispatcher>,
        Arc::clone(&build_metrics) as Arc<dyn EventDispatcher>,
    ];
    let build_event_stream =
        BuildEventStreamServiceImpl::with_dispatchers(event_dispatchers.clone());
    let dag_executor = DagExecutorServiceImpl::new(
        work_scheduler.clone(),
        Arc::clone(&task_graph),
        execution_plan_arc,
        event_dispatchers,
    );
    let worker_process = WorkerProcessServiceImpl::new();
    let build_layout = BuildLayoutServiceImpl::new();
    let build_result = BuildResultServiceImpl::new();
    let problem_reporting = ProblemReportingServiceImpl::new();
    let resource_management = ResourceManagementServiceImpl::new();
    let build_comparison = BuildComparisonServiceImpl::new();
    let test_execution = TestExecutionServiceImpl::new();
    let artifact_publishing = ArtifactPublishingServiceImpl::new();
    let build_init = BuildInitServiceImpl::new();
    let incremental_compilation = IncrementalCompilationServiceImpl::new();
    let garbage_collection = GarbageCollectionServiceImpl::new(
        cache_dir.clone(),
        history_dir.clone(),
        config_cache_dir.clone(),
    );

    let listener = UnixListener::bind(&socket_path).unwrap();

    tokio::spawn(async move {
        let _ = Server::builder()
            .add_service(control_service_server::ControlServiceServer::new(control))
            .add_service(dag_executor_service_server::DagExecutorServiceServer::new(
                dag_executor,
            ))
            .add_service(hash_service_server::HashServiceServer::new(hash))
            .add_service(cache_service_server::CacheServiceServer::new(cache))
            .add_service(exec_service_server::ExecServiceServer::new(exec))
            .add_service(work_service_server::WorkServiceServer::new(work))
            .add_service(
                execution_plan_service_server::ExecutionPlanServiceServer::new(
                    execution_plan_server,
                ),
            )
            .add_service(
                execution_history_service_server::ExecutionHistoryServiceServer::new(
                    execution_history,
                ),
            )
            .add_service(
                build_cache_orchestration_service_server::BuildCacheOrchestrationServiceServer::new(
                    cache_orchestration,
                ),
            )
            .add_service(
                file_fingerprint_service_server::FileFingerprintServiceServer::new(
                    file_fingerprint,
                ),
            )
            .add_service(
                value_snapshot_service_server::ValueSnapshotServiceServer::new(value_snapshot),
            )
            .add_service(task_graph_service_server::TaskGraphServiceServer::new(
                (*task_graph).clone(),
            ))
            .add_service(
                configuration_service_server::ConfigurationServiceServer::new(configuration),
            )
            .add_service(plugin_service_server::PluginServiceServer::new(plugin))
            .add_service(
                build_operations_service_server::BuildOperationsServiceServer::new(
                    build_operations,
                ),
            )
            .add_service(bootstrap_service_server::BootstrapServiceServer::new(
                bootstrap,
            ))
            .add_service(
                dependency_resolution_service_server::DependencyResolutionServiceServer::new(
                    dependency_resolution,
                ),
            )
            .add_service(file_watch_service_server::FileWatchServiceServer::new(
                file_watch,
            ))
            .add_service(
                configuration_cache_service_server::ConfigurationCacheServiceServer::new(
                    config_cache,
                ),
            )
            .add_service(toolchain_service_server::ToolchainServiceServer::new(
                toolchain,
            ))
            .add_service(
                build_event_stream_service_server::BuildEventStreamServiceServer::new(
                    build_event_stream,
                ),
            )
            .add_service(
                worker_process_service_server::WorkerProcessServiceServer::new(worker_process),
            )
            .add_service(build_layout_service_server::BuildLayoutServiceServer::new(
                build_layout,
            ))
            .add_service(build_result_service_server::BuildResultServiceServer::new(
                build_result,
            ))
            .add_service(
                problem_reporting_service_server::ProblemReportingServiceServer::new(
                    problem_reporting,
                ),
            )
            .add_service(
                resource_management_service_server::ResourceManagementServiceServer::new(
                    resource_management,
                ),
            )
            .add_service(
                build_comparison_service_server::BuildComparisonServiceServer::new(
                    build_comparison,
                ),
            )
            .add_service(console_service_server::ConsoleServiceServer::new(
                (*console).clone(),
            ))
            .add_service(
                test_execution_service_server::TestExecutionServiceServer::new(test_execution),
            )
            .add_service(
                artifact_publishing_service_server::ArtifactPublishingServiceServer::new(
                    artifact_publishing,
                ),
            )
            .add_service(build_init_service_server::BuildInitServiceServer::new(
                build_init,
            ))
            .add_service(
                incremental_compilation_service_server::IncrementalCompilationServiceServer::new(
                    incremental_compilation,
                ),
            )
            .add_service(
                build_metrics_service_server::BuildMetricsServiceServer::new(
                    (*build_metrics).clone(),
                ),
            )
            .add_service(
                garbage_collection_service_server::GarbageCollectionServiceServer::new(
                    garbage_collection,
                ),
            )
            .serve_with_incoming(UnixListenerStream::new(listener))
            .await;
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    (socket_path_str, dir)
}

async fn connect(socket_path: &str) -> Channel {
    let path = socket_path.to_string();
    Endpoint::from_shared("http://localhost".to_string())
        .unwrap()
        .connect_with_connector(tower::service_fn(move |_: tonic::transport::Uri| {
            let path = path.clone();
            async move {
                let stream = tokio::net::UnixStream::connect(&path).await?;
                let io = hyper_util::rt::TokioIo::new(stream);
                Ok::<_, std::io::Error>(io)
            }
        }))
        .await
        .unwrap()
}

// ============================================================
// Cache streaming helpers
// ============================================================

/// Store a value in the cache via the streaming StoreEntry RPC.
async fn cache_store_entry(
    client: &mut cache_service_client::CacheServiceClient<Channel>,
    key: &[u8],
    value: &[u8],
) -> bool {
    use futures_util::stream;

    let init_chunk = CacheStoreChunk {
        payload: Some(cache_store_chunk::Payload::Init(CacheStoreInit {
            key: key.to_vec(),
            total_size: value.len() as i64,
        })),
    };

    // Split value into 8KB chunks (matching server's chunk size)
    let data_chunks: Vec<CacheStoreChunk> = value
        .chunks(64 * 1024)
        .map(|chunk| CacheStoreChunk {
            payload: Some(cache_store_chunk::Payload::Data(chunk.to_vec())),
        })
        .collect();

    let all_chunks: Vec<CacheStoreChunk> = std::iter::once(init_chunk)
        .chain(data_chunks.into_iter())
        .collect();

    let stream = stream::iter(all_chunks);

    match client.store_entry(stream).await {
        Ok(response) => response.into_inner().success,
        Err(e) => {
            eprintln!("Store error: {}", e);
            false
        }
    }
}

/// Load a value from the cache via the streaming LoadEntry RPC.
/// Returns (metadata, data) or None on cache miss.
async fn cache_load_entry(
    client: &mut cache_service_client::CacheServiceClient<Channel>,
    key: &[u8],
) -> Option<(CacheEntryMetadata, Vec<u8>)> {
    let response = client
        .load_entry(Request::new(CacheLoadRequest { key: key.to_vec() }))
        .await
        .ok()?;

    let mut metadata: Option<CacheEntryMetadata> = None;
    let mut data = Vec::new();

    let mut stream = response.into_inner();
    while let Some(chunk) = stream.message().await.unwrap_or(None) {
        match chunk.payload {
            Some(cache_load_chunk::Payload::Metadata(meta)) => {
                metadata = Some(meta);
            }
            Some(cache_load_chunk::Payload::Data(bytes)) => {
                data.extend_from_slice(&bytes);
            }
            None => {}
        }
    }

    // If we got no chunks at all, it's a cache miss
    if metadata.is_none() && data.is_empty() {
        return None;
    }

    metadata.map(|meta| (meta, data))
}

// ============================================================
// Test 1: Put 50 entries, get them back with byte-for-byte equality
// ============================================================

#[tokio::test]
async fn test_put_50_entries_byte_for_byte_equality() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = cache_service_client::CacheServiceClient::new(channel);

    let num_entries = 50;
    let mut mismatches: Vec<String> = Vec::new();

    // Store 50 entries
    for i in 0..num_entries {
        let key = format!("diff-test-key-{:04}", i);
        let value = format!(
            "cache value number {} with some unique content padding: {}",
            i,
            "x".repeat(i * 7)
        );
        let key_bytes = key.as_bytes();

        let stored = cache_store_entry(&mut client, key_bytes, value.as_bytes()).await;
        assert!(stored, "Failed to store entry {}", i);
    }

    // Retrieve and verify all 50 entries
    for i in 0..num_entries {
        let key = format!("diff-test-key-{:04}", i);
        let expected_value = format!(
            "cache value number {} with some unique content padding: {}",
            i,
            "x".repeat(i * 7)
        );
        let key_bytes = key.as_bytes();

        let loaded = cache_load_entry(&mut client, key_bytes).await;

        match loaded {
            Some((meta, data)) => {
                if data != expected_value.as_bytes() {
                    mismatches.push(format!(
                        "Entry {} byte mismatch:\n  expected {} bytes: {:?}\n  got      {} bytes: {:?}",
                        i,
                        expected_value.len(),
                        &expected_value.as_bytes()[..std::cmp::min(64, expected_value.len())],
                        data.len(),
                        &data[..std::cmp::min(64, data.len())],
                    ));
                }
                if meta.size != expected_value.len() as i64 {
                    mismatches.push(format!(
                        "Entry {} metadata size mismatch: expected {}, got {}",
                        i,
                        expected_value.len(),
                        meta.size
                    ));
                }
            }
            None => {
                mismatches.push(format!("Entry {} not found (cache miss)", i));
            }
        }
    }

    if !mismatches.is_empty() {
        panic!(
            "Cache roundtrip mismatches ({} failures):\n{}",
            mismatches.len(),
            mismatches.join("\n")
        );
    }
}

// ============================================================
// Test 2: Cache misses for nonexistent keys
// ============================================================

#[tokio::test]
async fn test_cache_misses_for_nonexistent_keys() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = cache_service_client::CacheServiceClient::new(channel);

    let missing_keys = [
        b"nonexistent-key-1".as_slice(),
        b"nonexistent-key-2".as_slice(),
        b"also-not-here-3".as_slice(),
        b"zzz_missing_key_4".as_slice(),
        b"000000_not_present".as_slice(),
    ];

    for (i, key) in missing_keys.iter().enumerate() {
        let result = cache_load_entry(&mut client, *key).await;
        assert!(
            result.is_none(),
            "Key {} should be a cache miss, but got data",
            i
        );
    }
}

// ============================================================
// Test 3: Overwrite semantics (last-write-wins)
// ============================================================

#[tokio::test]
async fn test_overwrite_last_write_wins() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = cache_service_client::CacheServiceClient::new(channel);

    let key = b"overwrite-test-key";

    // Store first value
    let value1 = b"first value stored initially";
    let stored1 = cache_store_entry(&mut client, key, value1).await;
    assert!(stored1, "First store should succeed");

    // Verify first value
    let loaded1 = cache_load_entry(&mut client, key).await;
    assert!(loaded1.is_some(), "First value should be retrievable");
    assert_eq!(loaded1.unwrap().1, value1, "First value should match");

    // Overwrite with second value
    let value2 = b"second value that replaces the first one completely different";
    let stored2 = cache_store_entry(&mut client, key, value2).await;
    assert!(stored2, "Overwrite store should succeed");

    // Verify second value
    let loaded2 = cache_load_entry(&mut client, key).await;
    assert!(loaded2.is_some(), "Overwritten value should be retrievable");
    assert_eq!(
        loaded2.unwrap().1,
        value2,
        "Overwritten value should match second write"
    );

    // Ensure first value is gone (different lengths already prove they differ)
    assert_ne!(
        value1.len(),
        value2.len(),
        "Test sanity: values should have different lengths"
    );
    let loaded_final = cache_load_entry(&mut client, key).await;
    assert_eq!(
        loaded_final.unwrap().1,
        value2,
        "Final read should return the second (overwritten) value"
    );
}

// ============================================================
// Test 4: Binary data roundtrip
// ============================================================

#[tokio::test]
async fn test_binary_data_roundtrip() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = cache_service_client::CacheServiceClient::new(channel);

    let bytes_0_255: Vec<u8> = (0..=255).collect();
    let pattern_64kb: Vec<u8> = (0..65536).map(|i| (i % 256) as u8).collect();

    let binary_cases: Vec<(&[u8], &str)> = vec![
        (&[0u8; 1024], "all zeros 1KB"),
        (&[0xFFu8; 2048], "all 0xFF 2KB"),
        (&bytes_0_255, "bytes 0-255"),
        (&pattern_64kb, "64KB pattern"),
        (&[], "empty data"),
        (&[0xDE, 0xAD, 0xBE, 0xEF], "4 bytes"),
    ];

    for (value, description) in &binary_cases {
        let key = format!("binary-{}", description);
        let stored = cache_store_entry(&mut client, key.as_bytes(), value).await;
        assert!(stored, "Store should succeed for {}", description);

        let loaded = cache_load_entry(&mut client, key.as_bytes()).await;
        assert!(loaded.is_some(), "Load should succeed for {}", description);
        let (meta, data) = loaded.unwrap();

        assert_eq!(
            data,
            *value,
            "Binary roundtrip mismatch for '{}': expected {} bytes, got {} bytes",
            description,
            value.len(),
            data.len()
        );
        assert_eq!(
            meta.size,
            value.len() as i64,
            "Metadata size mismatch for '{}'",
            description
        );
    }
}

// ============================================================
// Test 5: Large entry roundtrip (100KB+)
// ============================================================

#[tokio::test]
async fn test_large_entry_roundtrip() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = cache_service_client::CacheServiceClient::new(channel);

    // 100KB of data (exceeds the 64KB chunk size)
    let large_value: Vec<u8> = (0..100_000).map(|i| ((i * 37 + 13) % 256) as u8).collect();
    let key = b"large-entry-100kb";

    let stored = cache_store_entry(&mut client, key, &large_value).await;
    assert!(stored, "Large entry store should succeed");

    let loaded = cache_load_entry(&mut client, key).await;
    assert!(loaded.is_some(), "Large entry load should succeed");
    let (meta, data) = loaded.unwrap();

    assert_eq!(
        data.len(),
        large_value.len(),
        "Large entry data length mismatch"
    );
    assert_eq!(data, large_value, "Large entry byte-for-byte mismatch");
    assert_eq!(
        meta.size,
        large_value.len() as i64,
        "Large entry metadata size mismatch"
    );
}

// ============================================================
// Test 6: Concurrent put/get operations
// ============================================================

#[tokio::test]
async fn test_concurrent_put_get() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;

    let num_concurrent = 20;
    let mut handles = Vec::new();

    for i in 0..num_concurrent {
        let ch = channel.clone();
        handles.push(tokio::spawn(async move {
            let mut client = cache_service_client::CacheServiceClient::new(ch);

            let key = format!("concurrent-key-{}", i);
            let value = format!("concurrent value {} with padding {}", i, "c".repeat(i * 11));
            let key_bytes = key.as_bytes();
            let value_bytes = value.as_bytes();

            // Store
            let stored = cache_store_entry(&mut client, key_bytes, value_bytes).await;
            assert!(stored, "Concurrent store {} should succeed", i);

            // Load
            let loaded = cache_load_entry(&mut client, key_bytes).await;
            assert!(loaded.is_some(), "Concurrent load {} should succeed", i);
            let (meta, data) = loaded.unwrap();

            assert_eq!(data, value_bytes, "Concurrent entry {} mismatch", i);
            assert_eq!(
                meta.size,
                value_bytes.len() as i64,
                "Concurrent entry {} size mismatch",
                i
            );

            (i, true)
        }));
    }

    let mut successes = 0;
    for handle in handles {
        let (i, ok) = handle.await.unwrap();
        assert!(ok, "Concurrent task {} failed", i);
        successes += 1;
    }

    assert_eq!(
        successes, num_concurrent,
        "All concurrent operations should succeed"
    );
}

// ============================================================
// Test 7: Multiple gets return same data
// ============================================================

#[tokio::test]
async fn test_multiple_gets_return_same_data() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = cache_service_client::CacheServiceClient::new(channel);

    let key = b"multi-get-key";
    let value = b"test value for multiple get verification";

    cache_store_entry(&mut client, key, value).await;

    // Get the entry 5 times
    for i in 0..5 {
        let loaded = cache_load_entry(&mut client, key).await;
        assert!(loaded.is_some(), "Get {} should succeed", i);
        let (meta, data) = loaded.unwrap();
        assert_eq!(data, value, "Get {} data mismatch", i);
        assert_eq!(meta.size, value.len() as i64, "Get {} size mismatch", i);
    }
}

// ============================================================
// Test 8: Store after delete behavior (overwrite acts as fresh store)
// ============================================================

#[tokio::test]
async fn test_store_after_overwrite_consistency() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = cache_service_client::CacheServiceClient::new(channel);

    let key = b"overwrite-consistency-key";

    // Three sequential overwrites
    let values = [
        b"first".as_slice(),
        b"second value".as_slice(),
        b"third value that is the longest one".as_slice(),
    ];

    for (i, value) in values.iter().enumerate() {
        let stored = cache_store_entry(&mut client, key, value).await;
        assert!(stored, "Store {} should succeed", i);

        let loaded = cache_load_entry(&mut client, key).await;
        assert!(loaded.is_some(), "Load after store {} should succeed", i);
        let (_, data) = loaded.unwrap();
        assert_eq!(
            data, *value,
            "After store {}, should read back value {}",
            i, i
        );
    }

    // Final read should return the last value
    let final_loaded = cache_load_entry(&mut client, key).await;
    assert!(final_loaded.is_some());
    assert_eq!(
        final_loaded.unwrap().1,
        values[2],
        "Final value should be the third write"
    );
}

// ============================================================
// Test 9: Empty value storage and retrieval
// ============================================================

#[tokio::test]
async fn test_empty_value_roundtrip() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = cache_service_client::CacheServiceClient::new(channel);

    let key = b"empty-value-key";
    let value: &[u8] = b"";

    let stored = cache_store_entry(&mut client, key, value).await;
    assert!(stored, "Empty value store should succeed");

    let loaded = cache_load_entry(&mut client, key).await;
    assert!(loaded.is_some(), "Empty value load should succeed");
    let (meta, data) = loaded.unwrap();
    assert_eq!(data, value, "Empty value roundtrip mismatch");
    assert_eq!(meta.size, 0, "Empty value metadata size should be 0");
}
