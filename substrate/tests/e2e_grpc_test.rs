//! End-to-end gRPC integration test for the substrate daemon.
//!
//! Prerequisites:
//!   1. Build: cargo build --release --bin gradle-substrate-daemon
//!   2. Run:   cargo test --test e2e_grpc_test -- --nocapture
//!
//! Or set SUBSTRATE_DAEMON_BIN to override the binary path.

use gradle_substrate_daemon::proto::{
    control_service_client::ControlServiceClient, hash_service_client::HashServiceClient,
    parser_service_client::ParserServiceClient, FileToHash, HandshakeRequest, HashBatchRequest,
    ParseBuildScriptDependenciesRequest, ParseBuildScriptRepositoriesRequest,
    ParseBuildScriptRequest,
};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use tonic::transport::{Channel, Endpoint, Uri};

async fn connect_result(socket_path: &Path) -> Result<Channel, tonic::transport::Error> {
    let endpoint = Endpoint::from_static("http://[::]:0")
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10));

    #[cfg(unix)]
    {
        let path = socket_path.to_path_buf();
        endpoint
            .connect_with_connector(tower::service_fn(move |_: Uri| {
                let path = path.clone();
                async move {
                    let stream = tokio::net::UnixStream::connect(path).await?;
                    Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
                }
            }))
            .await
    }
    #[cfg(not(unix))]
    {
        compile_error!("E2E tests require Unix domain sockets");
    }
}

struct TestDaemon {
    socket_path: PathBuf,
    _temp_dir: tempfile::TempDir,
    child: Child,
}

impl TestDaemon {
    async fn start() -> Self {
        let temp_dir = tempfile::tempdir().expect("Failed to create e2e daemon temp dir");
        let socket_path = temp_dir.path().join("substrate.sock");
        eprintln!("[e2e] Starting daemon at {}...", socket_path.display());

        let daemon_bin = std::env::var("SUBSTRATE_DAEMON_BIN").unwrap_or_else(|_| {
            if let Ok(path) = std::env::var("CARGO_BIN_EXE_gradle-substrate-daemon") {
                return path;
            }

            // CARGO_MANIFEST_DIR points to substrate/, workspace root is one level up.
            let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
            let workspace = Path::new(&manifest).parent().unwrap_or(Path::new("."));
            let debug_bin = workspace
                .join("target")
                .join("debug")
                .join("gradle-substrate-daemon");
            if debug_bin.exists() {
                return debug_bin.to_string_lossy().to_string();
            }

            workspace
                .join("target")
                .join("release")
                .join("gradle-substrate-daemon")
                .to_string_lossy()
                .to_string()
        });

        let child = Command::new(&daemon_bin)
            .arg("--socket-path")
            .arg(&socket_path)
            .arg("--log-level")
            .arg("error")
            .arg("--cache-dir")
            .arg(temp_dir.path().join("cache"))
            .arg("--history-dir")
            .arg(temp_dir.path().join("history"))
            .arg("--config-cache-dir")
            .arg(temp_dir.path().join("config-cache"))
            .arg("--toolchain-dir")
            .arg(temp_dir.path().join("toolchains"))
            .arg("--artifact-store-dir")
            .arg(temp_dir.path().join("artifacts"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("Failed to start daemon");

        let mut daemon = Self {
            socket_path,
            _temp_dir: temp_dir,
            child,
        };

        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if connect_result(&daemon.socket_path).await.is_ok() {
                return daemon;
            }
            if let Ok(Some(status)) = daemon.child.try_wait() {
                panic!("Daemon exited before accepting connections: {status}");
            }
        }
        panic!("Daemon failed to start within 5 seconds");
    }

    async fn connect(&self) -> Channel {
        connect_result(&self.socket_path)
            .await
            .expect("Failed to connect to substrate daemon via UDS")
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

async fn test_channel() -> (TestDaemon, Channel) {
    let daemon = TestDaemon::start().await;
    let channel = daemon.connect().await;
    (daemon, channel)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn temp_file(name: &str, content: &[u8]) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("substrate-e2e");
    std::fs::create_dir_all(&dir).ok();
    let path = dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

// ─── Control ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_control_handshake() {
    let (_daemon, channel) = test_channel().await;
    let mut client = ControlServiceClient::new(channel);

    let resp = client
        .handshake(HandshakeRequest {
            client_version: "e2e-test-1.0".to_string(),
            protocol_version: "1.0.0".to_string(),
            client_pid: std::process::id() as i32,
            jvm_host_socket_path: String::new(),
        })
        .await
        .expect("Handshake failed")
        .into_inner();

    assert_eq!(resp.protocol_version, "1.0.0");
    assert!(resp.accepted);
    assert!(!resp.server_version.is_empty());
    println!(
        "[e2e] Handshake OK: server={}, protocol={}",
        resp.server_version, resp.protocol_version
    );
}

// ─── Hash ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_hash_batch_md5() {
    let (_daemon, channel) = test_channel().await;
    let mut client = HashServiceClient::new(channel);

    let file = temp_file("e2e_md5.txt", b"Hello, Gradle Rust Substrate!");
    let meta = std::fs::metadata(&file).unwrap();

    let resp = client
        .hash_batch(HashBatchRequest {
            algorithm: "MD5".to_string(),
            files: vec![FileToHash {
                absolute_path: file.to_string_lossy().to_string(),
                length: meta.len() as i64,
                last_modified: meta
                    .modified()
                    .unwrap()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64,
            }],
        })
        .await
        .expect("hash_batch failed")
        .into_inner();

    assert_eq!(resp.results.len(), 1);
    let r = &resp.results[0];
    assert!(!r.error, "Hash should succeed: {}", r.error_message);
    assert_eq!(r.hash_bytes.len(), 16, "MD5 = 16 bytes");

    let expected = gradle_substrate_daemon::server::hash::hash_file_md5(&file).unwrap();
    assert_eq!(
        r.hash_bytes, expected,
        "gRPC hash must match direct computation"
    );
    println!("[e2e] MD5 OK: {}", hex(&r.hash_bytes));
}

#[tokio::test]
async fn test_hash_batch_sha256() {
    let (_daemon, channel) = test_channel().await;
    let mut client = HashServiceClient::new(channel);

    let file = temp_file("e2e_sha256.txt", b"SHA-256 verification content\n");
    let meta = std::fs::metadata(&file).unwrap();

    let resp = client
        .hash_batch(HashBatchRequest {
            algorithm: "SHA256".to_string(),
            files: vec![FileToHash {
                absolute_path: file.to_string_lossy().to_string(),
                length: meta.len() as i64,
                last_modified: meta
                    .modified()
                    .unwrap()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs() as i64,
            }],
        })
        .await
        .expect("hash_batch SHA256 failed")
        .into_inner();

    assert_eq!(resp.results.len(), 1);
    assert!(!resp.results[0].error);
    assert_eq!(resp.results[0].hash_bytes.len(), 32);
    println!("[e2e] SHA256 OK: {}", hex(&resp.results[0].hash_bytes));
}

#[tokio::test]
async fn test_hash_batch_multiple_files_distinct_hashes() {
    let (_daemon, channel) = test_channel().await;
    let mut client = HashServiceClient::new(channel);

    let f1 = temp_file("e2e_multi_a.txt", b"content A");
    let f2 = temp_file("e2e_multi_b.txt", b"content B");
    let m1 = std::fs::metadata(&f1).unwrap();
    let m2 = std::fs::metadata(&f2).unwrap();

    let resp = client
        .hash_batch(HashBatchRequest {
            algorithm: "SHA256".to_string(),
            files: vec![
                FileToHash {
                    absolute_path: f1.to_string_lossy().to_string(),
                    length: m1.len() as i64,
                    last_modified: m1
                        .modified()
                        .unwrap()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs() as i64,
                },
                FileToHash {
                    absolute_path: f2.to_string_lossy().to_string(),
                    length: m2.len() as i64,
                    last_modified: m2
                        .modified()
                        .unwrap()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs() as i64,
                },
            ],
        })
        .await
        .expect("hash_batch multi failed")
        .into_inner();

    assert_eq!(resp.results.len(), 2);
    assert!(!resp.results[0].error);
    assert!(!resp.results[1].error);
    assert_ne!(
        resp.results[0].hash_bytes, resp.results[1].hash_bytes,
        "Different files must have different hashes"
    );
    println!(
        "[e2e] Multi-file OK: {} vs {}",
        hex(&resp.results[0].hash_bytes),
        hex(&resp.results[1].hash_bytes)
    );
}

#[tokio::test]
async fn test_hash_batch_empty() {
    let (_daemon, channel) = test_channel().await;
    let mut client = HashServiceClient::new(channel);

    let resp = client
        .hash_batch(HashBatchRequest {
            algorithm: "SHA256".to_string(),
            files: vec![],
        })
        .await
        .expect("empty batch should succeed")
        .into_inner();

    assert_eq!(resp.results.len(), 0);
    println!("[e2e] Empty batch OK");
}

#[tokio::test]
async fn test_hash_batch_nonexistent_file_returns_error() {
    let (_daemon, channel) = test_channel().await;
    let mut client = HashServiceClient::new(channel);

    let resp = client
        .hash_batch(HashBatchRequest {
            algorithm: "MD5".to_string(),
            files: vec![FileToHash {
                absolute_path: "/tmp/substrate_e2e_nonexistent_99999.txt".to_string(),
                length: 0,
                last_modified: 0,
            }],
        })
        .await
        .expect("nonexistent file should still return response")
        .into_inner();

    assert_eq!(resp.results.len(), 1);
    assert!(
        resp.results[0].error,
        "Nonexistent file should report error"
    );
    println!(
        "[e2e] Nonexistent file error OK: {}",
        resp.results[0].error_message
    );
}

// ─── Parser ──────────────────────────────────────────────────────

#[tokio::test]
async fn test_parser_build_script_elements() {
    let (_daemon, channel) = test_channel().await;
    let mut client = ParserServiceClient::new(channel);

    let resp = client
        .parse_build_script(ParseBuildScriptRequest {
            script_content:
                "plugins { id 'java' }\ndependencies { implementation 'com.example:lib:1.0' }\n"
                    .to_string(),
            file_path: "build.gradle".to_string(),
        })
        .await
        .expect("parse_build_script failed")
        .into_inner();

    assert_eq!(resp.error_count, 0, "Should parse without errors");
    assert!(!resp.elements.is_empty(), "Should produce elements");
    println!(
        "[e2e] Parse build script OK: {} elements",
        resp.elements.len()
    );
}

#[tokio::test]
async fn test_parser_dependencies_extraction() {
    let (_daemon, channel) = test_channel().await;
    let mut client = ParserServiceClient::new(channel);

    let resp = client.parse_build_script_dependencies(ParseBuildScriptDependenciesRequest {
        script_content: "dependencies {\n    implementation 'com.example:lib:1.0'\n    testImplementation 'junit:junit:4.13'\n}\n".to_string(),
        configuration_name: String::new(),
    }).await.expect("parse_build_script_dependencies failed").into_inner();

    assert!(
        resp.dependencies.len() >= 2,
        "Should find at least 2 deps, got {}",
        resp.dependencies.len()
    );
    println!("[e2e] Dependencies OK: {} deps", resp.dependencies.len());
}

#[tokio::test]
async fn test_parser_plugins_extraction() {
    let (_daemon, channel) = test_channel().await;
    let mut client = ParserServiceClient::new(channel);

    // Use content known to parse correctly (same as unit test)
    let resp = client
        .parse_build_script(ParseBuildScriptRequest {
            script_content: r#"plugins {
    id 'java'
    id 'application' version '1.0' apply false
}
dependencies {
    implementation 'com.example:lib:1.0'
}
"#
            .to_string(),
            file_path: "build.gradle".to_string(),
        })
        .await
        .expect("parse_build_script failed")
        .into_inner();

    assert!(
        !resp.elements.is_empty(),
        "Should find elements, got 0. Service may not handle this content format"
    );
    println!("[e2e] Plugins OK: {} elements", resp.elements.len());
}

#[tokio::test]
async fn test_parser_repositories_extraction() {
    let (_daemon, channel) = test_channel().await;
    let mut client = ParserServiceClient::new(channel);

    let resp = client
        .parse_build_script_repositories(ParseBuildScriptRepositoriesRequest {
            script_content: "repositories {\n    mavenCentral()\n    google()\n}\n".to_string(),
        })
        .await
        .expect("parse_build_script_repositories failed")
        .into_inner();

    assert!(
        resp.repositories.len() >= 2,
        "Should find at least 2 repos, got {}",
        resp.repositories.len()
    );
    println!("[e2e] Repositories OK: {} repos", resp.repositories.len());
}

#[tokio::test]
async fn test_parser_empty_script() {
    let (_daemon, channel) = test_channel().await;
    let mut client = ParserServiceClient::new(channel);

    let resp = client
        .parse_build_script(ParseBuildScriptRequest {
            script_content: String::new(),
            file_path: "build.gradle".to_string(),
        })
        .await
        .expect("empty script should not fail")
        .into_inner();

    assert_eq!(resp.error_count, 0);
    assert!(resp.elements.is_empty());
    println!("[e2e] Empty script OK");
}
