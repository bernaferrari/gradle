use std::sync::atomic::{AtomicI64, Ordering};

use dashmap::DashMap;
use md5::Digest as _;
use sha1::Digest as _;
use sha2::Digest as _;
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

/// Rust-native artifact publishing service.
/// Manages artifact upload to Maven/Ivy repositories with checksums.
pub struct ArtifactPublishingServiceImpl {
    artifacts: DashMap<String, TrackedArtifact>,
    build_artifacts: DashMap<String, Vec<String>>, // build_id -> [artifact_id]
    artifacts_registered: AtomicI64,
    uploads_completed: AtomicI64,
}

impl ArtifactPublishingServiceImpl {
    pub fn new() -> Self {
        Self {
            artifacts: DashMap::new(),
            build_artifacts: DashMap::new(),
            artifacts_registered: AtomicI64::new(0),
            uploads_completed: AtomicI64::new(0),
        }
    }
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
            .or_insert_with(Vec::new)
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

        // No file path set, so checksums should be empty
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
}
