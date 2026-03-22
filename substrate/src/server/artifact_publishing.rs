use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use dashmap::DashMap;
use md5::Digest as _;
use tonic::{Request, Response, Status};

use crate::proto::{
    artifact_publishing_service_server::ArtifactPublishingService, ArtifactDescriptor,
    ArtifactPublishStatus, GetArtifactChecksumsRequest, GetArtifactChecksumsResponse,
    GetPublishingStatusRequest, GetPublishingStatusResponse, RecordUploadResultRequest,
    RecordUploadResultResponse, RegisterArtifactRequest, RegisterArtifactResponse,
};

/// Tracked artifact being published.
struct TrackedArtifact {
    descriptor: ArtifactDescriptor,
    status: String,
    upload_duration_ms: i64,
    error_message: String,
}

/// Repository credentials.
struct RepoCredentials {
    #[allow(dead_code)]
    username: String,
    #[allow(dead_code)]
    password: String,
}

/// Rust-native artifact publishing service.
/// Manages artifact upload to Maven/Ivy repositories with checksums.
/// Supports real HTTP PUT uploads with authentication.
pub struct ArtifactPublishingServiceImpl {
    artifacts: DashMap<String, TrackedArtifact>,
    build_artifacts: DashMap<String, Vec<String>>, // build_id -> [artifact_id]
    artifacts_registered: AtomicI64,
    uploads_completed: AtomicI64,
    repos: DashMap<String, RepoCredentials>,
    #[allow(dead_code)]
    http_client: reqwest::Client,
}

impl Default for ArtifactPublishingServiceImpl {
    fn default() -> Self {
        Self::new()
    }
}

impl ArtifactPublishingServiceImpl {
    pub fn new() -> Self {
        Self {
            artifacts: DashMap::new(),
            build_artifacts: DashMap::new(),
            artifacts_registered: AtomicI64::new(0),
            uploads_completed: AtomicI64::new(0),
            repos: DashMap::new(),
            http_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(300))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    /// Register repository credentials for authenticated uploads.
    pub fn register_repo(&self, repo_id: String, username: String, password: String) {
        self.repos.insert(repo_id, RepoCredentials { username, password });
    }

    /// Build the Maven repository URL for an artifact.
    #[allow(dead_code)]
    fn artifact_url(&self, descriptor: &ArtifactDescriptor) -> String {
        let group_path = descriptor.group.replace('.', "/");
        let classifier = if descriptor.classifier.is_empty() {
            String::new()
        } else {
            format!("-{}", descriptor.classifier)
        };
        format!(
            "{}/{}/{}/{}/{}-{}{}.{}",
            descriptor.repository_id,
            group_path,
            descriptor.name,
            descriptor.version,
            descriptor.name,
            descriptor.version,
            classifier,
            descriptor.extension
        )
    }

    /// Perform an actual HTTP PUT upload of an artifact to a Maven repository.
    #[allow(dead_code)]
    async fn perform_upload(
        &self,
        descriptor: &ArtifactDescriptor,
    ) -> Result<i64, String> {
        let file_path = &descriptor.file_path;
        if file_path.is_empty() || !std::path::Path::new(file_path).exists() {
            return Err("Artifact file does not exist".to_string());
        }

        let data = std::fs::read(file_path)
            .map_err(|e| format!("Failed to read artifact file: {}", e))?;

        let base_url = self.artifact_url(descriptor);
        let start = std::time::Instant::now();

        // Build the request with optional auth
        let mut request = self.http_client
            .put(&base_url)
            .header("Content-Type", "application/octet-stream")
            .body(data.clone());

        if let Some(creds) = self.repos.get(&descriptor.repository_id) {
            use std::io::Write;
            let mut buf = Vec::new();
            write!(buf, "{}:{}", creds.username, creds.password).unwrap();
            let auth = base64_encode(&buf);
            request = request.header("Authorization", format!("Basic {}", auth));
        }

        let response = request.send().await
            .map_err(|e| format!("Upload request failed: {}", e))?;

        let status = response.status().as_u16();
        if !(200..=299).contains(&status) {
            return Err(format!("Upload returned HTTP {}", status));
        }

        // Upload checksum files
        let checksum_uploads = [
            (format!("{}.md5", base_url), format!("{:x}", md5::Md5::digest(&data))),
            (format!("{}.sha1", base_url), format!("{:x}", sha1::Sha1::digest(&data))),
            (format!("{}.sha256", base_url), format!("{:x}", sha2::Sha256::digest(&data))),
        ];

        for (url, checksum) in &checksum_uploads {
            let mut req = self.http_client
                .put(url)
                .body(checksum.clone());
            if let Some(creds) = self.repos.get(&descriptor.repository_id) {
                use std::io::Write;
                let mut buf = Vec::new();
                write!(buf, "{}:{}", creds.username, creds.password).unwrap();
                let auth = base64_encode(&buf);
                req = req.header("Authorization", format!("Basic {}", auth));
            }
            if let Err(e) = req.send().await {
                tracing::warn!(url = %url, error = %e, "Failed to upload checksum file");
            }
        }

        let duration = start.elapsed().as_millis() as i64;
        Ok(duration)
    }
}

#[allow(dead_code)]
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

#[tonic::async_trait]
impl ArtifactPublishingService for ArtifactPublishingServiceImpl {
    async fn register_artifact(
        &self,
        request: Request<RegisterArtifactRequest>,
    ) -> Result<Response<RegisterArtifactResponse>, Status> {
        let req = request.into_inner();

        let descriptor = req
            .artifact
            .ok_or_else(|| Status::invalid_argument("ArtifactDescriptor is required"))?;

        let artifact_id = descriptor.artifact_id.clone();
        let build_id = req.build_id.clone();

        self.artifacts.insert(
            artifact_id.clone(),
            TrackedArtifact {
                descriptor,
                status: "pending".to_string(),
                upload_duration_ms: 0,
                error_message: String::new(),
            },
        );

        self.build_artifacts
            .entry(build_id)
            .or_default()
            .push(artifact_id);

        self.artifacts_registered.fetch_add(1, Ordering::Relaxed);

        Ok(Response::new(RegisterArtifactResponse { accepted: true }))
    }

    async fn record_upload_result(
        &self,
        request: Request<RecordUploadResultRequest>,
    ) -> Result<Response<RecordUploadResultResponse>, Status> {
        let req = request.into_inner();

        if let Some(mut artifact) = self.artifacts.get_mut(&req.artifact_id) {
            artifact.status = if req.success {
                "uploaded".to_string()
            } else {
                "failed".to_string()
            };
            artifact.upload_duration_ms = req.upload_duration_ms;
            artifact.error_message = req.error_message;

            self.uploads_completed.fetch_add(1, Ordering::Relaxed);

            tracing::debug!(
                artifact_id = %req.artifact_id,
                success = req.success,
                duration_ms = req.upload_duration_ms,
                "Upload result recorded"
            );
        }

        Ok(Response::new(RecordUploadResultResponse { accepted: true }))
    }

    async fn get_publishing_status(
        &self,
        request: Request<GetPublishingStatusRequest>,
    ) -> Result<Response<GetPublishingStatusResponse>, Status> {
        let req = request.into_inner();

        let artifact_ids = self
            .build_artifacts
            .get(&req.build_id)
            .map(|a| a.clone())
            .unwrap_or_default();

        let mut artifacts = Vec::new();
        let mut uploaded = 0i32;
        let mut failed = 0i32;
        let mut pending = 0i32;

        for artifact_id in &artifact_ids {
            if let Some(artifact) = self.artifacts.get(artifact_id) {
                match artifact.status.as_str() {
                    "uploaded" => uploaded += 1,
                    "failed" => failed += 1,
                    _ => pending += 1,
                }

                artifacts.push(ArtifactPublishStatus {
                    artifact: Some(artifact.descriptor.clone()),
                    status: artifact.status.clone(),
                    upload_duration_ms: artifact.upload_duration_ms,
                    error_message: artifact.error_message.clone(),
                });
            }
        }

        let total = artifacts.len() as i32;

        Ok(Response::new(GetPublishingStatusResponse {
            artifacts,
            total,
            uploaded,
            failed,
            pending,
        }))
    }

    async fn get_artifact_checksums(
        &self,
        request: Request<GetArtifactChecksumsRequest>,
    ) -> Result<Response<GetArtifactChecksumsResponse>, Status> {
        let req = request.into_inner();

        if let Some(artifact) = self.artifacts.get(&req.artifact_id) {
            let file_path = &artifact.descriptor.file_path;

            // Compute checksums from the file
            let (md5, sha1, sha256) = if !file_path.is_empty() && std::path::Path::new(file_path).exists() {
                let content = std::fs::read(file_path).unwrap_or_default();
                let md5_hash = format!("{:x}", md5::Md5::digest(&content));
                let sha1_hash = format!("{:x}", sha1::Sha1::digest(&content));
                let sha256_hash = format!("{:x}", sha2::Sha256::digest(&content));
                (md5_hash, sha1_hash, sha256_hash)
            } else {
                (String::new(), String::new(), String::new())
            };

            Ok(Response::new(GetArtifactChecksumsResponse {
                md5,
                sha1,
                sha256,
            }))
        } else {
            Ok(Response::new(GetArtifactChecksumsResponse {
                md5: String::new(),
                sha1: String::new(),
                sha256: String::new(),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_artifact(id: &str, name: &str) -> ArtifactDescriptor {
        ArtifactDescriptor {
            artifact_id: id.to_string(),
            group: "com.example".to_string(),
            name: name.to_string(),
            version: "1.0.0".to_string(),
            classifier: String::new(),
            extension: "jar".to_string(),
            file_path: String::new(),
            file_size_bytes: 1024,
            repository_id: "maven-central".to_string(),
        }
    }

    #[tokio::test]
    async fn test_register_and_upload() {
        let svc = ArtifactPublishingServiceImpl::new();

        svc.register_artifact(Request::new(RegisterArtifactRequest {
            build_id: "build-1".to_string(),
            artifact: Some(make_artifact("a1", "my-lib")),
        }))
        .await
        .unwrap();

        svc.record_upload_result(Request::new(RecordUploadResultRequest {
            artifact_id: "a1".to_string(),
            success: true,
            error_message: String::new(),
            upload_duration_ms: 500,
            bytes_transferred: 1024,
        }))
        .await
        .unwrap();

        let status = svc
            .get_publishing_status(Request::new(GetPublishingStatusRequest {
                build_id: "build-1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(status.total, 1);
        assert_eq!(status.uploaded, 1);
        assert_eq!(status.failed, 0);
    }

    #[tokio::test]
    async fn test_failed_upload() {
        let svc = ArtifactPublishingServiceImpl::new();

        svc.register_artifact(Request::new(RegisterArtifactRequest {
            build_id: "build-2".to_string(),
            artifact: Some(make_artifact("a2", "bad-lib")),
        }))
        .await
        .unwrap();

        svc.record_upload_result(Request::new(RecordUploadResultRequest {
            artifact_id: "a2".to_string(),
            success: false,
            error_message: "Connection refused".to_string(),
            upload_duration_ms: 5000,
            bytes_transferred: 0,
        }))
        .await
        .unwrap();

        let status = svc
            .get_publishing_status(Request::new(GetPublishingStatusRequest {
                build_id: "build-2".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(status.failed, 1);
        assert_eq!(status.artifacts[0].error_message, "Connection refused");
    }

    #[tokio::test]
    async fn test_multiple_artifacts() {
        let svc = ArtifactPublishingServiceImpl::new();

        svc.register_artifact(Request::new(RegisterArtifactRequest {
            build_id: "build-3".to_string(),
            artifact: Some(make_artifact("a3", "lib")),
        }))
        .await
        .unwrap();

        svc.register_artifact(Request::new(RegisterArtifactRequest {
            build_id: "build-3".to_string(),
            artifact: Some(make_artifact("a4", "sources")),
        }))
        .await
        .unwrap();

        svc.record_upload_result(Request::new(RecordUploadResultRequest {
            artifact_id: "a3".to_string(),
            success: true,
            error_message: String::new(),
            upload_duration_ms: 200,
            bytes_transferred: 1024,
        }))
        .await
        .unwrap();

        let status = svc
            .get_publishing_status(Request::new(GetPublishingStatusRequest {
                build_id: "build-3".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(status.total, 2);
        assert_eq!(status.uploaded, 1);
        assert_eq!(status.pending, 1);
    }

    #[tokio::test]
    async fn test_checksums_missing_file() {
        let svc = ArtifactPublishingServiceImpl::new();

        svc.register_artifact(Request::new(RegisterArtifactRequest {
            build_id: "build-4".to_string(),
            artifact: Some(make_artifact("a5", "no-file")),
        }))
        .await
        .unwrap();

        let checksums = svc
            .get_artifact_checksums(Request::new(GetArtifactChecksumsRequest {
                artifact_id: "a5".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(checksums.md5.is_empty());
    }

    #[tokio::test]
    async fn test_checksums_real_file() {
        let svc = ArtifactPublishingServiceImpl::new();

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.jar");
        std::fs::write(&file_path, b"hello world").unwrap();

        let mut artifact = make_artifact("a6", "real-lib");
        artifact.file_path = file_path.to_string_lossy().to_string();

        svc.register_artifact(Request::new(RegisterArtifactRequest {
            build_id: "build-5".to_string(),
            artifact: Some(artifact),
        }))
        .await
        .unwrap();

        let checksums = svc
            .get_artifact_checksums(Request::new(GetArtifactChecksumsRequest {
                artifact_id: "a6".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!checksums.md5.is_empty());
        assert!(!checksums.sha1.is_empty());
        assert!(!checksums.sha256.is_empty());
    }

    #[test]
    fn test_artifact_url() {
        let svc = ArtifactPublishingServiceImpl::new();
        let desc = ArtifactDescriptor {
            artifact_id: "test".to_string(),
            group: "com.example".to_string(),
            name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            classifier: String::new(),
            extension: "jar".to_string(),
            file_path: String::new(),
            file_size_bytes: 0,
            repository_id: "https://repo.example.com/maven2".to_string(),
        };
        let url = svc.artifact_url(&desc);
        assert_eq!(url, "https://repo.example.com/maven2/com/example/my-lib/1.0.0/my-lib-1.0.0.jar");
    }

    #[test]
    fn test_artifact_url_with_classifier() {
        let svc = ArtifactPublishingServiceImpl::new();
        let desc = ArtifactDescriptor {
            artifact_id: "test".to_string(),
            group: "com.example".to_string(),
            name: "my-lib".to_string(),
            version: "1.0.0".to_string(),
            classifier: "sources".to_string(),
            extension: "jar".to_string(),
            file_path: String::new(),
            file_size_bytes: 0,
            repository_id: "https://repo.example.com/maven2".to_string(),
        };
        let url = svc.artifact_url(&desc);
        assert_eq!(url, "https://repo.example.com/maven2/com/example/my-lib/1.0.0/my-lib-1.0.0-sources.jar");
    }

    #[test]
    fn test_repo_credentials() {
        let svc = ArtifactPublishingServiceImpl::new();
        svc.register_repo("my-repo".to_string(), "user".to_string(), "pass".to_string());
        assert!(svc.repos.contains_key("my-repo"));
    }

    #[tokio::test]
    async fn test_publishing_status_empty_build() {
        let svc = ArtifactPublishingServiceImpl::new();

        let status = svc
            .get_publishing_status(Request::new(GetPublishingStatusRequest {
                build_id: "nonexistent".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(status.total, 0);
        assert_eq!(status.uploaded, 0);
        assert_eq!(status.failed, 0);
        assert_eq!(status.pending, 0);
        assert!(status.artifacts.is_empty());
    }

    #[tokio::test]
    async fn test_checksums_nonexistent_artifact() {
        let svc = ArtifactPublishingServiceImpl::new();

        let checksums = svc
            .get_artifact_checksums(Request::new(GetArtifactChecksumsRequest {
                artifact_id: "nonexistent".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(checksums.md5.is_empty());
        assert!(checksums.sha1.is_empty());
        assert!(checksums.sha256.is_empty());
    }

    #[tokio::test]
    async fn test_record_upload_nonexistent_artifact() {
        let svc = ArtifactPublishingServiceImpl::new();

        // Recording upload for nonexistent artifact should succeed
        let resp = svc
            .record_upload_result(Request::new(RecordUploadResultRequest {
                artifact_id: "nonexistent".to_string(),
                success: true,
                error_message: String::new(),
                upload_duration_ms: 100,
                bytes_transferred: 0,
            }))
            .await
        .unwrap()
        .into_inner();

        assert!(resp.accepted);
    }

    #[tokio::test]
    async fn test_multiple_builds_isolated() {
        let svc = ArtifactPublishingServiceImpl::new();

        svc.register_artifact(Request::new(RegisterArtifactRequest {
            build_id: "build-A".to_string(),
            artifact: Some(make_artifact("a-a1", "lib-a")),
        }))
        .await
        .unwrap();

        svc.register_artifact(Request::new(RegisterArtifactRequest {
            build_id: "build-B".to_string(),
            artifact: Some(make_artifact("a-b1", "lib-b")),
        }))
        .await
        .unwrap();

        svc.record_upload_result(Request::new(RecordUploadResultRequest {
            artifact_id: "a-a1".to_string(),
            success: true,
            error_message: String::new(),
            upload_duration_ms: 100,
            bytes_transferred: 1024,
        }))
        .await
        .unwrap();

        let status_a = svc
            .get_publishing_status(Request::new(GetPublishingStatusRequest {
                build_id: "build-A".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        let status_b = svc
            .get_publishing_status(Request::new(GetPublishingStatusRequest {
                build_id: "build-B".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(status_a.total, 1);
        assert_eq!(status_a.uploaded, 1);
        assert_eq!(status_b.total, 1);
        assert_eq!(status_b.pending, 1);
    }
}
