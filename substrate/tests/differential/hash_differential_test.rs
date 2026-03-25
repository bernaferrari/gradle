/// Differential hash testing: validates Rust HashService outputs against
/// independently computed reference values using standard Rust crypto crates.
///
/// Tests cover:
/// - 100+ files hashed with MD5, SHA-1, SHA-256
/// - Empty files, binary files, unicode filenames, large files (1MB+), symlinks
/// - Byte-for-byte comparison against known-correct reference hashes
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use md5::{Digest, Md5};
use sha1::Sha1;
use sha2::Sha256;
use tokio::net::UnixListener;
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::{Channel, Endpoint, Server};
use tonic::Request;

use gradle_substrate_daemon::proto::*;
use gradle_substrate_daemon::server::{
    // Import all services needed for the server
    artifact_publishing::ArtifactPublishingServiceImpl,
    bootstrap::BootstrapServiceImpl,
    build_comparison::BuildComparisonServiceImpl,
    build_event_stream::BuildEventStreamServiceImpl,
    build_init::BuildInitServiceImpl,
    build_layout::BuildLayoutServiceImpl,
    build_metrics::BuildMetricsServiceImpl,
    build_operations::BuildOperationsServiceImpl,
    build_result::BuildResultServiceImpl,
    cache::CacheServiceImpl,
    cache_orchestration::BuildCacheOrchestrationServiceImpl,
    config_cache::ConfigurationCacheServiceImpl,
    configuration::ConfigurationServiceImpl,
    console::ConsoleServiceImpl,
    control::ControlServiceImpl,
    dag_executor::DagExecutorServiceImpl,
    dependency_resolution::DependencyResolutionServiceImpl,
    event_dispatcher::EventDispatcher,
    exec::ExecServiceImpl,
    execution_history::ExecutionHistoryServiceImpl,
    file_fingerprint::FileFingerprintServiceImpl,
    file_watch::FileWatchServiceImpl,
    garbage_collection::GarbageCollectionServiceImpl,
    hash::HashServiceImpl,
    incremental_compilation::IncrementalCompilationServiceImpl,
    plugin::PluginServiceImpl,
    problem_reporting::ProblemReportingServiceImpl,
    resource_management::ResourceManagementServiceImpl,
    task_graph::TaskGraphServiceImpl,
    test_execution::TestExecutionServiceImpl,
    toolchain::ToolchainServiceImpl,
    value_snapshot::ValueSnapshotServiceImpl,
    work::WorkerScheduler,
    worker_process::WorkerProcessServiceImpl,
};

// ============================================================
// Test server setup (mirrors integration_test.rs pattern)
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
                execution_plan_service_server::ExecutionPlanServiceServer::new(execution_plan),
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
// Reference hash computation (independent of the service)
// ============================================================

/// Compute the Gradle DefaultStreamHasher signature prefix (same logic as the server).
fn compute_gradle_signature() -> [u8; 16] {
    let mut sig_hasher = Md5::new();
    let sig_label = b"SIGNATURE";
    sig_hasher.update((sig_label.len() as i32).to_le_bytes());
    sig_hasher.update(sig_label);
    let class_name = b"CLASS:org.gradle.internal.hash.DefaultStreamHasher";
    sig_hasher.update((class_name.len() as i32).to_le_bytes());
    sig_hasher.update(class_name);
    sig_hasher.finalize().into()
}

/// Reference MD5 with Gradle signature prefix (matches server behavior for MD5).
fn reference_md5(data: &[u8]) -> Vec<u8> {
    let sig = compute_gradle_signature();
    let mut hasher = Md5::new();
    hasher.update(sig);
    hasher.update(data);
    hasher.finalize().to_vec()
}

/// Reference SHA-256 (no signature prefix).
fn reference_sha256(data: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

/// Reference SHA-1 (no signature prefix).
fn reference_sha1(data: &[u8]) -> Vec<u8> {
    let mut hasher = Sha1::new();
    hasher.update(data);
    hasher.finalize().to_vec()
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Helper: write a file with given content, return its path as a String.
fn write_test_file(dir: &Path, name: &str, content: &[u8]) -> String {
    let path = dir.join(name);
    let mut f = fs::File::create(&path).unwrap();
    f.write_all(content).unwrap();
    path.to_string_lossy().to_string()
}

/// Helper: read file content as bytes.
fn read_file(path: &str) -> Vec<u8> {
    fs::read(path).unwrap()
}

// ============================================================
// Test 1: 100+ files with all three algorithms
// ============================================================

#[tokio::test]
async fn test_hash_100_files_against_reference_md5() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = hash_service_client::HashServiceClient::new(channel);

    let file_dir = tempfile::tempdir().unwrap();

    // Generate 105 files with varied content
    let mut files_to_hash = Vec::new();
    let mut mismatches: Vec<String> = Vec::new();

    for i in 0..105 {
        let content = format!(
            "file content number {} with some padding data to vary length: {}",
            i,
            "x".repeat(i % 256)
        );
        let file_path = write_test_file(
            file_dir.path(),
            &format!("file_{:04}.txt", i),
            content.as_bytes(),
        );
        files_to_hash.push(FileToHash {
            absolute_path: file_path.clone(),
            length: 0,
            last_modified: 0,
        });
    }

    // Hash all files via the service
    let response = client
        .hash_batch(Request::new(HashBatchRequest {
            files: files_to_hash,
            algorithm: "MD5".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(response.results.len(), 105, "Expected 105 hash results");

    for result in &response.results {
        if result.error {
            mismatches.push(format!(
                "FILE ERROR: {} - {}",
                result.absolute_path, result.error_message
            ));
            continue;
        }

        let file_data = read_file(&result.absolute_path);
        let expected = reference_md5(&file_data);

        if result.hash_bytes != expected {
            mismatches.push(format!(
                "MD5 MISMATCH: {}\n  expected: {}\n  got:      {}",
                result.absolute_path,
                bytes_to_hex(&expected),
                bytes_to_hex(&result.hash_bytes)
            ));
        }
    }

    if !mismatches.is_empty() {
        panic!(
            "Hash mismatches detected ({} failures):\n{}",
            mismatches.len(),
            mismatches.join("\n")
        );
    }
}

#[tokio::test]
async fn test_hash_100_files_against_reference_sha256() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = hash_service_client::HashServiceClient::new(channel);

    let file_dir = tempfile::tempdir().unwrap();

    let mut files_to_hash = Vec::new();
    let mut mismatches: Vec<String> = Vec::new();

    for i in 0..105 {
        let content = format!("sha256 content {} padding {}", i, "y".repeat(i % 512));
        let file_path = write_test_file(
            file_dir.path(),
            &format!("sha256_file_{:04}.dat", i),
            content.as_bytes(),
        );
        files_to_hash.push(FileToHash {
            absolute_path: file_path.clone(),
            length: 0,
            last_modified: 0,
        });
    }

    let response = client
        .hash_batch(Request::new(HashBatchRequest {
            files: files_to_hash,
            algorithm: "SHA-256".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(response.results.len(), 105);

    for result in &response.results {
        if result.error {
            mismatches.push(format!(
                "FILE ERROR: {} - {}",
                result.absolute_path, result.error_message
            ));
            continue;
        }

        assert_eq!(
            result.hash_bytes.len(),
            32,
            "SHA-256 must be 32 bytes for {}",
            result.absolute_path
        );

        let file_data = read_file(&result.absolute_path);
        let expected = reference_sha256(&file_data);

        if result.hash_bytes != expected {
            mismatches.push(format!(
                "SHA-256 MISMATCH: {}\n  expected: {}\n  got:      {}",
                result.absolute_path,
                bytes_to_hex(&expected),
                bytes_to_hex(&result.hash_bytes)
            ));
        }
    }

    if !mismatches.is_empty() {
        panic!(
            "Hash mismatches detected ({} failures):\n{}",
            mismatches.len(),
            mismatches.join("\n")
        );
    }
}

#[tokio::test]
async fn test_hash_100_files_against_reference_sha1() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = hash_service_client::HashServiceClient::new(channel);

    let file_dir = tempfile::tempdir().unwrap();

    let mut files_to_hash = Vec::new();
    let mut mismatches: Vec<String> = Vec::new();

    for i in 0..105 {
        let content = format!("sha1 test data #{} {}", i, "z".repeat(i % 384));
        let file_path = write_test_file(
            file_dir.path(),
            &format!("sha1_file_{:04}.bin", i),
            content.as_bytes(),
        );
        files_to_hash.push(FileToHash {
            absolute_path: file_path.clone(),
            length: 0,
            last_modified: 0,
        });
    }

    let response = client
        .hash_batch(Request::new(HashBatchRequest {
            files: files_to_hash,
            algorithm: "SHA-1".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(response.results.len(), 105);

    for result in &response.results {
        if result.error {
            mismatches.push(format!(
                "FILE ERROR: {} - {}",
                result.absolute_path, result.error_message
            ));
            continue;
        }

        assert_eq!(
            result.hash_bytes.len(),
            20,
            "SHA-1 must be 20 bytes for {}",
            result.absolute_path
        );

        let file_data = read_file(&result.absolute_path);
        let expected = reference_sha1(&file_data);

        if result.hash_bytes != expected {
            mismatches.push(format!(
                "SHA-1 MISMATCH: {}\n  expected: {}\n  got:      {}",
                result.absolute_path,
                bytes_to_hex(&expected),
                bytes_to_hex(&result.hash_bytes)
            ));
        }
    }

    if !mismatches.is_empty() {
        panic!(
            "Hash mismatches detected ({} failures):\n{}",
            mismatches.len(),
            mismatches.join("\n")
        );
    }
}

// ============================================================
// Test 2: Empty files
// ============================================================

#[tokio::test]
async fn test_hash_empty_files_against_reference() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = hash_service_client::HashServiceClient::new(channel);

    let file_dir = tempfile::tempdir().unwrap();
    let mut mismatches: Vec<String> = Vec::new();

    // Create multiple empty files
    let empty_paths: Vec<String> = (0..5)
        .map(|i| write_test_file(file_dir.path(), &format!("empty_{}.txt", i), b""))
        .collect();

    for algorithm in &["MD5", "SHA-1", "SHA-256"] {
        let files: Vec<FileToHash> = empty_paths
            .iter()
            .map(|p| FileToHash {
                absolute_path: p.clone(),
                length: 0,
                last_modified: 0,
            })
            .collect();

        let response = client
            .hash_batch(Request::new(HashBatchRequest {
                files,
                algorithm: algorithm.to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(response.results.len(), 5);

        for result in &response.results {
            assert!(
                !result.error,
                "Empty file should not error: {}",
                result.error_message
            );

            let expected = match *algorithm {
                "MD5" => reference_md5(b""),
                "SHA-1" => reference_sha1(b""),
                "SHA-256" => reference_sha256(b""),
                _ => panic!("Unknown algorithm"),
            };

            if result.hash_bytes != expected {
                mismatches.push(format!(
                    "{} MISMATCH for empty file {}:\n  expected: {}\n  got:      {}",
                    algorithm,
                    result.absolute_path,
                    bytes_to_hex(&expected),
                    bytes_to_hex(&result.hash_bytes)
                ));
            }
        }
    }

    if !mismatches.is_empty() {
        panic!(
            "Empty file hash mismatches ({}):\n{}",
            mismatches.len(),
            mismatches.join("\n")
        );
    }
}

// ============================================================
// Test 3: Binary files
// ============================================================

#[tokio::test]
async fn test_hash_binary_files_against_reference() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = hash_service_client::HashServiceClient::new(channel);

    let file_dir = tempfile::tempdir().unwrap();
    let mut mismatches: Vec<String> = Vec::new();

    // Create binary files with various patterns
    let binary_cases: Vec<(&str, Vec<u8>)> = vec![
        ("all_zeros.bin", vec![0u8; 1024]),
        ("all_ones.bin", vec![0xFFu8; 1024]),
        (
            "alternating.bin",
            (0..512)
                .flat_map(|i| vec![i as u8, 0xFF - i as u8])
                .collect(),
        ),
        (
            "random_like.bin",
            (0..2048).map(|i| ((i * 31 + 17) % 256) as u8).collect(),
        ),
        ("single_byte.bin", vec![0x42u8]),
        ("null_term.bin", { b"hello world\0with null".to_vec() }),
    ];

    for (name, content) in &binary_cases {
        let file_path = write_test_file(file_dir.path(), name, content);

        for algorithm in &["MD5", "SHA-1", "SHA-256"] {
            let response = client
                .hash_batch(Request::new(HashBatchRequest {
                    files: vec![FileToHash {
                        absolute_path: file_path.clone(),
                        length: 0,
                        last_modified: 0,
                    }],
                    algorithm: algorithm.to_string(),
                }))
                .await
                .unwrap()
                .into_inner();

            let result = &response.results[0];
            assert!(!result.error, "Binary file error: {}", result.error_message);

            let expected = match *algorithm {
                "MD5" => reference_md5(content),
                "SHA-1" => reference_sha1(content),
                "SHA-256" => reference_sha256(content),
                _ => panic!(),
            };

            if result.hash_bytes != expected {
                mismatches.push(format!(
                    "{} MISMATCH for binary file {}:\n  expected: {}\n  got:      {}",
                    algorithm,
                    name,
                    bytes_to_hex(&expected),
                    bytes_to_hex(&result.hash_bytes)
                ));
            }
        }
    }

    if !mismatches.is_empty() {
        panic!(
            "Binary file hash mismatches ({}):\n{}",
            mismatches.len(),
            mismatches.join("\n")
        );
    }
}

// ============================================================
// Test 4: Unicode filenames
// ============================================================

#[tokio::test]
async fn test_hash_unicode_filenames_against_reference() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = hash_service_client::HashServiceClient::new(channel);

    let file_dir = tempfile::tempdir().unwrap();
    let mut mismatches: Vec<String> = Vec::new();

    let unicode_names = [
        (
            "\u{00e9}ntrepr\u{00ee}se.txt",
            "enterprise content with accents",
        ),
        (
            "\u{4f60}\u{597d}\u{4e16}\u{754c}.txt",
            "chinese characters content",
        ),
        (
            "\u{043f}\u{0440}\u{0438}\u{0432}\u{0435}\u{0442}.txt",
            "russian privet",
        ),
        ("caf\u{00e9}\u{0301}.txt", "cafe with combining accent"),
        (
            "\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}.txt",
            "japanese konnichiwa",
        ),
        (
            "\u{00dc}\u{00f6}\u{00e4}\u{00df}.txt",
            "german umlauts and eszett",
        ),
    ];

    for (name, content) in &unicode_names {
        let file_path = write_test_file(file_dir.path(), name, content.as_bytes());

        let response = client
            .hash_batch(Request::new(HashBatchRequest {
                files: vec![FileToHash {
                    absolute_path: file_path.clone(),
                    length: 0,
                    last_modified: 0,
                }],
                algorithm: "SHA-256".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        let result = &response.results[0];
        assert!(
            !result.error,
            "Unicode filename {} should not error: {}",
            name, result.error_message
        );

        let expected = reference_sha256(content.as_bytes());
        if result.hash_bytes != expected {
            mismatches.push(format!(
                "SHA-256 MISMATCH for unicode file '{}':\n  expected: {}\n  got:      {}",
                name,
                bytes_to_hex(&expected),
                bytes_to_hex(&result.hash_bytes)
            ));
        }
    }

    if !mismatches.is_empty() {
        panic!(
            "Unicode filename hash mismatches ({}):\n{}",
            mismatches.len(),
            mismatches.join("\n")
        );
    }
}

// ============================================================
// Test 5: Large files (1MB+)
// ============================================================

#[tokio::test]
async fn test_hash_large_files_against_reference() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = hash_service_client::HashServiceClient::new(channel);

    let file_dir = tempfile::tempdir().unwrap();
    let mut mismatches: Vec<String> = Vec::new();

    // Generate large files that exceed the 8KB read threshold
    let large_cases: Vec<(&str, usize, u8)> = vec![
        ("1mb_sequential.bin", 1_048_576, 0x42), // 1 MB
        ("1mb_zero.bin", 1_048_576, 0x00),       // 1 MB of zeros
        ("2mb_pattern.bin", 2_097_152, 0xAB),    // 2 MB
        ("512kb.bin", 524_288, 0xFF),            // 512 KB (just under 1MB)
    ];

    for (name, size, fill_byte) in &large_cases {
        let content = vec![*fill_byte; *size];
        let file_path = write_test_file(file_dir.path(), name, &content);

        for algorithm in &["MD5", "SHA-256"] {
            let response = client
                .hash_batch(Request::new(HashBatchRequest {
                    files: vec![FileToHash {
                        absolute_path: file_path.clone(),
                        length: 0,
                        last_modified: 0,
                    }],
                    algorithm: algorithm.to_string(),
                }))
                .await
                .unwrap()
                .into_inner();

            let result = &response.results[0];
            assert!(
                !result.error,
                "Large file {} should not error: {}",
                name, result.error_message
            );

            let expected = match *algorithm {
                "MD5" => reference_md5(&content),
                "SHA-256" => reference_sha256(&content),
                _ => panic!(),
            };

            if result.hash_bytes != expected {
                mismatches.push(format!(
                    "{} MISMATCH for large file {} ({} bytes):\n  expected: {}\n  got:      {}",
                    algorithm,
                    name,
                    size,
                    bytes_to_hex(&expected),
                    bytes_to_hex(&result.hash_bytes)
                ));
            }
        }
    }

    if !mismatches.is_empty() {
        panic!(
            "Large file hash mismatches ({}):\n{}",
            mismatches.len(),
            mismatches.join("\n")
        );
    }
}

// ============================================================
// Test 6: Symlinks (followed by the service, so hash = target hash)
// ============================================================

#[tokio::test]
async fn test_hash_symlinks_against_reference() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = hash_service_client::HashServiceClient::new(channel);

    let file_dir = tempfile::tempdir().unwrap();
    let mut mismatches: Vec<String> = Vec::new();

    // Create a target file
    let target_path = file_dir.path().join("target_file.txt");
    let target_content = b"content that the symlink points to";
    fs::write(&target_path, target_content).unwrap();

    // Create symlinks pointing to the target
    let symlink_paths = [
        file_dir.path().join("symlink1.txt"),
        file_dir.path().join("nested").join("symlink2.txt"),
    ];

    fs::create_dir_all(file_dir.path().join("nested")).unwrap();
    for symlink in &symlink_paths {
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target_path, symlink).unwrap();
    }

    // The service follows symlinks, so hash(symlink) == hash(target)
    let expected_sha256 = reference_sha256(target_content);

    for symlink in &symlink_paths {
        let symlink_str = symlink.to_string_lossy().to_string();

        let response = client
            .hash_batch(Request::new(HashBatchRequest {
                files: vec![FileToHash {
                    absolute_path: symlink_str.clone(),
                    length: 0,
                    last_modified: 0,
                }],
                algorithm: "SHA-256".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        let result = &response.results[0];
        assert!(
            !result.error,
            "Symlink {} should not error: {}",
            symlink_str, result.error_message
        );

        if result.hash_bytes != expected_sha256 {
            mismatches.push(format!(
                "SHA-256 MISMATCH for symlink {}:\n  expected: {}\n  got:      {}",
                symlink_str,
                bytes_to_hex(&expected_sha256),
                bytes_to_hex(&result.hash_bytes)
            ));
        }
    }

    if !mismatches.is_empty() {
        panic!(
            "Symlink hash mismatches ({}):\n{}",
            mismatches.len(),
            mismatches.join("\n")
        );
    }
}

// ============================================================
// Test 7: Different algorithms produce different hashes for same content
// ============================================================

#[tokio::test]
async fn test_different_algorithms_produce_different_hashes() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = hash_service_client::HashServiceClient::new(channel);

    let file_dir = tempfile::tempdir().unwrap();
    let file_path = write_test_file(
        file_dir.path(),
        "multi_algo.txt",
        b"test content for multi-algo comparison",
    );

    let mut all_hashes: Vec<(String, Vec<u8>)> = Vec::new();

    for algorithm in &["MD5", "SHA-1", "SHA-256"] {
        let response = client
            .hash_batch(Request::new(HashBatchRequest {
                files: vec![FileToHash {
                    absolute_path: file_path.clone(),
                    length: 0,
                    last_modified: 0,
                }],
                algorithm: algorithm.to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        all_hashes.push((
            algorithm.to_string(),
            response.results[0].hash_bytes.clone(),
        ));
    }

    // All three should have different lengths
    assert_eq!(all_hashes[0].1.len(), 16, "MD5 should be 16 bytes");
    assert_eq!(all_hashes[1].1.len(), 20, "SHA-1 should be 20 bytes");
    assert_eq!(all_hashes[2].1.len(), 32, "SHA-256 should be 32 bytes");

    // All three should differ in content (different lengths already prove this,
    // but also verify first 16 bytes differ between any pair)
    assert_ne!(
        &all_hashes[0].1[..],
        &all_hashes[1].1[..16],
        "MD5 and SHA-1 first 16 bytes must differ"
    );
    assert_ne!(
        &all_hashes[0].1[..],
        &all_hashes[2].1[..16],
        "MD5 and SHA-256 first 16 bytes must differ"
    );
    assert_ne!(
        &all_hashes[1].1[..],
        &all_hashes[2].1[..16],
        "SHA-1 and SHA-256 first 16 bytes must differ"
    );
}

// ============================================================
// Test 8: Nonexistent files produce error results (not panics)
// ============================================================

#[tokio::test]
async fn test_nonexistent_files_report_errors_cleanly() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = hash_service_client::HashServiceClient::new(channel);

    let nonexistent = [
        "/tmp/nonexistent_differential_hash_test_12345.txt",
        "/tmp/also_missing_67890.dat",
        "/tmp/definitely_not_here_abcdef.bin",
    ];

    let files: Vec<FileToHash> = nonexistent
        .iter()
        .map(|p| FileToHash {
            absolute_path: p.to_string(),
            length: 0,
            last_modified: 0,
        })
        .collect();

    let response = client
        .hash_batch(Request::new(HashBatchRequest {
            files,
            algorithm: "SHA-256".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(response.results.len(), 3);
    for result in &response.results {
        assert!(
            result.error,
            "Nonexistent file {} should report error",
            result.absolute_path
        );
        assert!(
            !result.error_message.is_empty(),
            "Error message should not be empty for {}",
            result.absolute_path
        );
        assert!(
            result.hash_bytes.is_empty(),
            "Hash bytes should be empty for error result {}",
            result.absolute_path
        );
    }
}

// ============================================================
// Test 9: Mix of valid and invalid files in single batch
// ============================================================

#[tokio::test]
async fn test_mixed_valid_invalid_batch() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = hash_service_client::HashServiceClient::new(channel);

    let file_dir = tempfile::tempdir().unwrap();
    let valid_path = write_test_file(file_dir.path(), "valid.txt", b"valid content");
    let invalid_path = "/tmp/does_not_exist_xyz.bin".to_string();
    let valid_path2 = write_test_file(file_dir.path(), "valid2.txt", b"other valid content");

    let response = client
        .hash_batch(Request::new(HashBatchRequest {
            files: vec![
                FileToHash {
                    absolute_path: valid_path.clone(),
                    length: 0,
                    last_modified: 0,
                },
                FileToHash {
                    absolute_path: invalid_path,
                    length: 0,
                    last_modified: 0,
                },
                FileToHash {
                    absolute_path: valid_path2.clone(),
                    length: 0,
                    last_modified: 0,
                },
            ],
            algorithm: "SHA-256".to_string(),
        }))
        .await
        .unwrap()
        .into_inner();

    assert_eq!(response.results.len(), 3);

    // First: valid
    assert!(!response.results[0].error);
    let expected0 = reference_sha256(b"valid content");
    assert_eq!(
        response.results[0].hash_bytes, expected0,
        "First valid file hash mismatch"
    );

    // Second: invalid
    assert!(response.results[1].error);

    // Third: valid
    assert!(!response.results[2].error);
    let expected2 = reference_sha256(b"other valid content");
    assert_eq!(
        response.results[2].hash_bytes, expected2,
        "Third valid file hash mismatch"
    );
}

// ============================================================
// Test 10: Determinism -- same file always produces same hash
// ============================================================

#[tokio::test]
async fn test_hash_determinism_across_10_calls() {
    let (socket_path, _dir) = spawn_server().await;
    let channel = connect(&socket_path).await;
    let mut client = hash_service_client::HashServiceClient::new(channel);

    let file_dir = tempfile::tempdir().unwrap();
    let file_path = write_test_file(
        file_dir.path(),
        "determinism.txt",
        b"check determinism across 10 calls",
    );

    let file_entry = FileToHash {
        absolute_path: file_path.clone(),
        length: 0,
        last_modified: 0,
    };

    let mut hashes = Vec::new();
    for _ in 0..10 {
        let response = client
            .hash_batch(Request::new(HashBatchRequest {
                files: vec![file_entry.clone()],
                algorithm: "SHA-256".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        hashes.push(response.results.into_iter().next().unwrap().hash_bytes);
    }

    let first = &hashes[0];
    for (i, h) in hashes.iter().enumerate().skip(1) {
        assert_eq!(first, h, "Hash at call {} differs from first call", i + 1);
    }
}
