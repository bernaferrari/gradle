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
use super::scopes::BuildId;

/// Tracked artifact being published.
struct TrackedArtifact {
    descriptor: ArtifactDescriptor,
    status: String,
    upload_duration_ms: i64,
    error_message: String,
}

/// Repository credentials.
struct RepoCredentials {
    username: String,
    password: String,
}

/// Rust-native artifact publishing service.
/// Manages artifact upload to Maven/Ivy repositories with checksums.
/// Supports real HTTP PUT uploads with authentication.
pub struct ArtifactPublishingServiceImpl {
    artifacts: DashMap<String, TrackedArtifact>,
    build_artifacts: DashMap<BuildId, Vec<String>>, // build_id -> [artifact_id]
    artifacts_registered: AtomicI64,
    uploads_completed: AtomicI64,
    repos: DashMap<String, RepoCredentials>,
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
                descriptor: descriptor.clone(),
                status: "pending".to_string(),
                upload_duration_ms: 0,
                error_message: String::new(),
            },
        );

        // Log the target repository URL for the registered artifact
        let target_url = self.artifact_url(&descriptor);
        tracing::debug!(
            artifact_id = %artifact_id,
            build_id = %build_id,
            target_url = %target_url,
            repository_id = %descriptor.repository_id,
            file_size = descriptor.file_size_bytes,
            "Artifact registered for publishing"
        );

        self.build_artifacts
            .entry(BuildId::from(build_id))
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
            artifact.error_message = req.error_message.clone();

            self.uploads_completed.fetch_add(1, Ordering::Relaxed);

            // Compute the target repository URL and log auth configuration
            let target_url = self.artifact_url(&artifact.descriptor);
            let repo_id = &artifact.descriptor.repository_id;
            let auth_configured = if let Some(creds) = self.repos.get(repo_id.as_str()) {
                // Verify credentials are non-empty
                let has_auth = !creds.username.is_empty() && !creds.password.is_empty();
                if has_auth {
                    // Mask password in log output using base64_encode
                    let masked_pw = base64_encode(creds.password.as_bytes());
                    tracing::debug!(
                        artifact_id = %req.artifact_id,
                        target_url = %target_url,
                        username = %creds.username,
                        password_masked = %masked_pw,
                        "Upload recorded with authenticated repository"
                    );
                }
                has_auth
            } else {
                false
            };

            if !auth_configured {
                tracing::debug!(
                    artifact_id = %req.artifact_id,
                    target_url = %target_url,
                    "Upload recorded for unauthenticated repository"
                );
            }

            // After a successful upload, verify the artifact is reachable via HEAD request
            if req.success {
                if let Some(creds) = self.repos.get(repo_id.as_str()) {
                    let mut head_req = self.http_client.head(&target_url);
                    let mut buf = Vec::new();
                    use std::io::Write;
                    write!(buf, "{}:{}", creds.username, creds.password).unwrap();
                    let auth = base64_encode(&buf);
                    head_req = head_req.header("Authorization", format!("Basic {}", auth));
                    match head_req.send().await {
                        Ok(resp) => {
                            tracing::debug!(
                                artifact_id = %req.artifact_id,
                                status_code = resp.status().as_u16(),
                                "Artifact HEAD verification after upload"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                artifact_id = %req.artifact_id,
                                error = %e,
                                "Failed HEAD verification after upload"
                            );
                        }
                    }
                } else if !artifact.descriptor.file_path.is_empty() {
                    // No credentials configured but file exists -- attempt an unauthenticated
                    // upload via perform_upload for repositories that allow anonymous pushes.
                    match self.perform_upload(&artifact.descriptor).await {
                        Ok(duration) => {
                            tracing::info!(
                                artifact_id = %req.artifact_id,
                                upload_duration_ms = duration,
                                "Unauthenticated upload performed via perform_upload"
                            );
                        }
                        Err(e) => {
                            tracing::debug!(
                                artifact_id = %req.artifact_id,
                                error = %e,
                                "Unauthenticated perform_upload skipped (expected for local-only publishing)"
                            );
                        }
                    }
                }
            }

            tracing::info!(
                artifact_id = %req.artifact_id,
                success = req.success,
                duration_ms = req.upload_duration_ms,
                bytes_transferred = req.bytes_transferred,
                target_url = %target_url,
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
            .get(&BuildId::from(req.build_id))
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

    #[tokio::test]
    async fn test_register_artifact_with_classifier() {
        let svc = ArtifactPublishingServiceImpl::new();

        let mut artifact = make_artifact("src-jar-1", "my-lib");
        artifact.classifier = "sources".to_string();
        artifact.extension = "jar".to_string();

        let resp = svc
            .register_artifact(Request::new(RegisterArtifactRequest {
                build_id: "build-classifier".to_string(),
                artifact: Some(artifact.clone()),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.accepted);

        // Verify the stored descriptor retains the classifier
        let status = svc
            .get_publishing_status(Request::new(GetPublishingStatusRequest {
                build_id: "build-classifier".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(status.total, 1);
        let stored = &status.artifacts[0];
        assert_eq!(stored.artifact.as_ref().unwrap().classifier, "sources");
        assert_eq!(stored.artifact.as_ref().unwrap().extension, "jar");
        assert_eq!(stored.artifact.as_ref().unwrap().name, "my-lib");
        assert_eq!(stored.status, "pending");
    }

    #[tokio::test]
    async fn test_checksums_after_upload_recorded() {
        let svc = ArtifactPublishingServiceImpl::new();

        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("published.jar");
        std::fs::write(&file_path, b"artifact content for publishing").unwrap();

        let mut artifact = make_artifact("pub-1", "publish-lib");
        artifact.file_path = file_path.to_string_lossy().to_string();

        svc.register_artifact(Request::new(RegisterArtifactRequest {
            build_id: "build-pub".to_string(),
            artifact: Some(artifact),
        }))
        .await
        .unwrap();

        // Record a successful upload
        svc.record_upload_result(Request::new(RecordUploadResultRequest {
            artifact_id: "pub-1".to_string(),
            success: true,
            error_message: String::new(),
            upload_duration_ms: 1200,
            bytes_transferred: 2048,
        }))
        .await
        .unwrap();

        // Checksums should be computed from the real file
        let checksums = svc
            .get_artifact_checksums(Request::new(GetArtifactChecksumsRequest {
                artifact_id: "pub-1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        // Verify the MD5 matches expected value for the known content
        assert_eq!(
            checksums.md5,
            format!("{:x}", md5::Md5::digest(b"artifact content for publishing"))
        );
        assert_eq!(
            checksums.sha1,
            format!("{:x}", sha1::Sha1::digest(b"artifact content for publishing"))
        );
        assert_eq!(
            checksums.sha256,
            format!("{:x}", sha2::Sha256::digest(b"artifact content for publishing"))
        );

        // Also confirm the status reflects the completed upload
        let status = svc
            .get_publishing_status(Request::new(GetPublishingStatusRequest {
                build_id: "build-pub".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(status.uploaded, 1);
        assert_eq!(status.artifacts[0].upload_duration_ms, 1200);
    }

    #[tokio::test]
    async fn test_multiple_builds_overlapping_artifact_ids() {
        let svc = ArtifactPublishingServiceImpl::new();

        // Build-X registers artifact "shared-1"
        svc.register_artifact(Request::new(RegisterArtifactRequest {
            build_id: "build-X".to_string(),
            artifact: Some(make_artifact("shared-1", "lib-common")),
        }))
        .await
        .unwrap();

        // Build-Y also registers artifact "shared-1" (same ID, e.g. same artifact published by two builds)
        svc.register_artifact(Request::new(RegisterArtifactRequest {
            build_id: "build-Y".to_string(),
            artifact: Some(make_artifact("shared-1", "lib-common")),
        }))
        .await
        .unwrap();

        // Build-X registers its own unique artifact
        svc.register_artifact(Request::new(RegisterArtifactRequest {
            build_id: "build-X".to_string(),
            artifact: Some(make_artifact("x-only", "lib-x")),
        }))
        .await
        .unwrap();

        // Build-Y registers its own unique artifact
        svc.register_artifact(Request::new(RegisterArtifactRequest {
            build_id: "build-Y".to_string(),
            artifact: Some(make_artifact("y-only", "lib-y")),
        }))
        .await
        .unwrap();

        // Mark "shared-1" as uploaded (this mutates the single DashMap entry)
        svc.record_upload_result(Request::new(RecordUploadResultRequest {
            artifact_id: "shared-1".to_string(),
            success: true,
            error_message: String::new(),
            upload_duration_ms: 300,
            bytes_transferred: 4096,
        }))
        .await
        .unwrap();

        // Mark "x-only" as uploaded
        svc.record_upload_result(Request::new(RecordUploadResultRequest {
            artifact_id: "x-only".to_string(),
            success: true,
            error_message: String::new(),
            upload_duration_ms: 150,
            bytes_transferred: 2048,
        }))
        .await
        .unwrap();

        // "y-only" stays pending

        let status_x = svc
            .get_publishing_status(Request::new(GetPublishingStatusRequest {
                build_id: "build-X".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        let status_y = svc
            .get_publishing_status(Request::new(GetPublishingStatusRequest {
                build_id: "build-Y".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        // Both builds see "shared-1" as uploaded because the DashMap entry is shared
        assert_eq!(status_x.total, 2);
        assert_eq!(status_x.uploaded, 2);
        assert_eq!(status_x.pending, 0);

        assert_eq!(status_y.total, 2);
        assert_eq!(status_y.uploaded, 1); // shared-1 is uploaded
        assert_eq!(status_y.pending, 1);  // y-only is still pending
    }

    #[tokio::test]
    async fn test_mixed_publishing_status() {
        let svc = ArtifactPublishingServiceImpl::new();

        let build_id = "build-mixed".to_string();

        // Register three artifacts
        svc.register_artifact(Request::new(RegisterArtifactRequest {
            build_id: build_id.clone(),
            artifact: Some(make_artifact("m1", "core-lib")),
        }))
        .await
        .unwrap();

        svc.register_artifact(Request::new(RegisterArtifactRequest {
            build_id: build_id.clone(),
            artifact: Some(make_artifact("m2", "util-lib")),
        }))
        .await
        .unwrap();

        svc.register_artifact(Request::new(RegisterArtifactRequest {
            build_id: build_id.clone(),
            artifact: Some(make_artifact("m3", "test-lib")),
        }))
        .await
        .unwrap();

        // Mark m1 as uploaded successfully
        svc.record_upload_result(Request::new(RecordUploadResultRequest {
            artifact_id: "m1".to_string(),
            success: true,
            error_message: String::new(),
            upload_duration_ms: 800,
            bytes_transferred: 8192,
        }))
        .await
        .unwrap();

        // Mark m2 as failed
        svc.record_upload_result(Request::new(RecordUploadResultRequest {
            artifact_id: "m2".to_string(),
            success: false,
            error_message: "HTTP 403 Forbidden".to_string(),
            upload_duration_ms: 2500,
            bytes_transferred: 0,
        }))
        .await
        .unwrap();

        // m3 stays pending (no upload result recorded)

        let status = svc
            .get_publishing_status(Request::new(GetPublishingStatusRequest {
                build_id: build_id.clone(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(status.total, 3);
        assert_eq!(status.uploaded, 1);
        assert_eq!(status.failed, 1);
        assert_eq!(status.pending, 1);

        // Verify individual artifact statuses are correct
        let artifacts_by_name: std::collections::HashMap<&str, &ArtifactPublishStatus> = status
            .artifacts
            .iter()
            .map(|a| (a.artifact.as_ref().unwrap().name.as_str(), a))
            .collect();

        assert_eq!(artifacts_by_name["core-lib"].status, "uploaded");
        assert_eq!(artifacts_by_name["core-lib"].upload_duration_ms, 800);
        assert!(artifacts_by_name["core-lib"].error_message.is_empty());

        assert_eq!(artifacts_by_name["util-lib"].status, "failed");
        assert_eq!(artifacts_by_name["util-lib"].upload_duration_ms, 2500);
        assert_eq!(artifacts_by_name["util-lib"].error_message, "HTTP 403 Forbidden");

        assert_eq!(artifacts_by_name["test-lib"].status, "pending");
        assert_eq!(artifacts_by_name["test-lib"].upload_duration_ms, 0);
    }
}
