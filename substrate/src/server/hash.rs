use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use md5::{Digest, Md5};
use sha1::Sha1;
use sha2::Sha256;
use tonic::{Request, Response, Status};

use crate::error::SubstrateError;
use crate::proto::{
    hash_service_server::HashService, HashBatchRequest, HashBatchResponse, HashResult,
};

#[derive(Default)]
pub struct HashServiceImpl;

/// Supported hash algorithms.
#[derive(Debug, Clone, Copy)]
enum HashAlgorithm {
    Md5,
    Sha1,
    Sha256,
}

impl HashAlgorithm {
    fn from_name(name: &str) -> Option<Self> {
        match name.to_uppercase().as_str() {
            "" | "MD5" => Some(HashAlgorithm::Md5),
            "SHA-1" | "SHA1" => Some(HashAlgorithm::Sha1),
            "SHA-256" | "SHA256" => Some(HashAlgorithm::Sha256),
            _ => None,
        }
    }
}

/// Hash a file with the given algorithm, optionally with the Gradle signature prefix.
fn hash_file_with_algorithm(path: &Path, algorithm: HashAlgorithm, with_signature: bool) -> Result<Vec<u8>, SubstrateError> {
    let file = File::open(path).map_err(|e| SubstrateError::Hash(format!(
        "Cannot open {}: {}",
        path.display(),
        e
    )))?;

    let metadata = file.metadata().map_err(|e| SubstrateError::Hash(format!(
        "Cannot stat {}: {}",
        path.display(),
        e
    )))?;

    let file_len = metadata.len() as usize;

    // Prepend Gradle signature if using MD5 with signature mode
    let prefix = if with_signature && matches!(algorithm, HashAlgorithm::Md5) {
        Some(compute_gradle_signature())
    } else {
        None
    };

    let mut reader = BufReader::with_capacity(8192, file);

    // For small files (< 8KB), read entirely and hash in one call
    if file_len < 8192 {
        let mut buf = Vec::with_capacity(file_len);
        reader.read_to_end(&mut buf).map_err(|e| SubstrateError::Hash(format!(
            "Cannot read {}: {}",
            path.display(),
            e
        )))?;

        match algorithm {
            HashAlgorithm::Md5 => {
                let mut hasher = Md5::new();
                if let Some(sig) = prefix {
                    hasher.update(sig);
                }
                hasher.update(&buf);
                Ok(hasher.finalize().to_vec())
            }
            HashAlgorithm::Sha1 => {
                let mut hasher = Sha1::new();
                hasher.update(&buf);
                Ok(hasher.finalize().to_vec())
            }
            HashAlgorithm::Sha256 => {
                let mut hasher = Sha256::new();
                hasher.update(&buf);
                Ok(hasher.finalize().to_vec())
            }
        }
    } else {
        // For larger files, read in chunks
        let mut buffer = [0u8; 8192];

        match algorithm {
            HashAlgorithm::Md5 => {
                let mut hasher = Md5::new();
                if let Some(sig) = prefix {
                    hasher.update(sig);
                }
                loop {
                    let n = reader.read(&mut buffer).map_err(|e| SubstrateError::Hash(format!(
                        "Cannot read {}: {}",
                        path.display(),
                        e
                    )))?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buffer[..n]);
                }
                Ok(hasher.finalize().to_vec())
            }
            HashAlgorithm::Sha1 => {
                let mut hasher = Sha1::new();
                loop {
                    let n = reader.read(&mut buffer).map_err(|e| SubstrateError::Hash(format!(
                        "Cannot read {}: {}",
                        path.display(),
                        e
                    )))?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buffer[..n]);
                }
                Ok(hasher.finalize().to_vec())
            }
            HashAlgorithm::Sha256 => {
                let mut hasher = Sha256::new();
                loop {
                    let n = reader.read(&mut buffer).map_err(|e| SubstrateError::Hash(format!(
                        "Cannot read {}: {}",
                        path.display(),
                        e
                    )))?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buffer[..n]);
                }
                Ok(hasher.finalize().to_vec())
            }
        }
    }
}

/// Compute the Gradle DefaultStreamHasher signature prefix.
/// signature = MD5(int32_le(9) + "SIGNATURE" + int32_le(52) + "CLASS:org.gradle.internal.hash.DefaultStreamHasher")
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

#[tonic::async_trait]
impl HashService for HashServiceImpl {
    async fn hash_batch(
        &self,
        request: Request<HashBatchRequest>,
    ) -> Result<Response<HashBatchResponse>, Status> {
        let req = request.into_inner();

        let algorithm = match HashAlgorithm::from_name(&req.algorithm) {
            Some(algo) => algo,
            None => {
                return Err(Status::invalid_argument(format!(
                    "Unsupported hash algorithm: '{}'. Supported: MD5, SHA-1, SHA-256",
                    req.algorithm
                )));
            }
        };

        let mut results = Vec::with_capacity(req.files.len());

        for file in req.files {
            let path = Path::new(&file.absolute_path);

            // Use Gradle signature prefix for MD5 (default mode)
            let with_signature = matches!(algorithm, HashAlgorithm::Md5);

            match hash_file_with_algorithm(path, algorithm, with_signature) {
                Ok(hash_bytes) => results.push(HashResult {
                    absolute_path: file.absolute_path,
                    hash_bytes,
                    error: false,
                    error_message: String::new(),
                }),
                Err(e) => results.push(HashResult {
                    absolute_path: file.absolute_path,
                    hash_bytes: Vec::new(),
                    error: true,
                    error_message: e.to_string(),
                }),
            }
        }

        tracing::debug!(
            count = results.len(),
            algorithm = %req.algorithm,
            "Hashed files"
        );

        Ok(Response::new(HashBatchResponse { results }))
    }
}

/// Hash a file using MD5 with Java's DefaultStreamHasher-compatible signature prefix.
///
/// Java's DefaultStreamHasher prepends a signature before hashing file content.
/// The final hash = MD5(signature_16_bytes || file_content_bytes)
pub fn hash_file_md5(path: &Path) -> Result<Vec<u8>, SubstrateError> {
    hash_file_with_algorithm(path, HashAlgorithm::Md5, true)
}

/// Hash a file using SHA-256 (no signature prefix).
pub fn hash_file_sha256(path: &Path) -> Result<Vec<u8>, SubstrateError> {
    hash_file_with_algorithm(path, HashAlgorithm::Sha256, false)
}

/// Hash a file using SHA-1 (no signature prefix).
pub fn hash_file_sha1(path: &Path) -> Result<Vec<u8>, SubstrateError> {
    hash_file_with_algorithm(path, HashAlgorithm::Sha1, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_hash_empty_file() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "").unwrap();
        let hash = hash_file_md5(tmp.path()).unwrap();
        assert_eq!(hash.len(), 16);
    }

    #[test]
    fn test_hash_known_content() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "hello world").unwrap();
        let hash = hash_file_md5(tmp.path()).unwrap();
        assert_eq!(hash.len(), 16);
        // Deterministic: same content always produces same hash
        let hash2 = hash_file_md5(tmp.path()).unwrap();
        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_hash_missing_file() {
        let result = hash_file_md5(Path::new("/nonexistent/file"));
        assert!(result.is_err());
    }

    #[test]
    fn test_hash_large_file() {
        let mut tmp = NamedTempFile::new().unwrap();
        let data = vec![0xAB_u8; 100_000];
        tmp.write_all(&data).unwrap();
        let hash = hash_file_md5(tmp.path()).unwrap();
        assert_eq!(hash.len(), 16);
    }

    #[test]
    fn test_sha256_hash() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "sha256 test content").unwrap();
        let hash = hash_file_sha256(tmp.path()).unwrap();
        assert_eq!(hash.len(), 32);

        // SHA-256 should produce different output than MD5
        let md5_hash = hash_file_md5(tmp.path()).unwrap();
        assert_eq!(md5_hash.len(), 16);
        assert_ne!(hash, md5_hash);
    }

    #[test]
    fn test_sha1_hash() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "sha1 test content").unwrap();
        let hash = hash_file_sha1(tmp.path()).unwrap();
        assert_eq!(hash.len(), 20);
    }

    #[test]
    fn test_sha256_deterministic() {
        let mut tmp1 = NamedTempFile::new().unwrap();
        let mut tmp2 = NamedTempFile::new().unwrap();
        tmp1.write_all(b"deterministic content for sha256").unwrap();
        tmp2.write_all(b"deterministic content for sha256").unwrap();
        let hash1 = hash_file_sha256(tmp1.path()).unwrap();
        let hash2 = hash_file_sha256(tmp2.path()).unwrap();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_gradle_signature_deterministic() {
        let sig1 = compute_gradle_signature();
        let sig2 = compute_gradle_signature();
        assert_eq!(sig1, sig2);
        assert_eq!(sig1.len(), 16);
    }

    #[tokio::test]
    async fn test_gRPC_sha256_algorithm() {
        let svc = HashServiceImpl;

        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "test sha256 via grpc").unwrap();

        let resp = svc
            .hash_batch(Request::new(HashBatchRequest {
                files: vec![crate::proto::FileToHash {
                    absolute_path: tmp.path().to_string_lossy().to_string(),
                    length: 0,
                    last_modified: 0,
                }],
                algorithm: "SHA-256".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results[0].hash_bytes.len(), 32);
        assert!(!resp.results[0].error);
    }

    #[tokio::test]
    async fn test_gRPC_sha1_algorithm() {
        let svc = HashServiceImpl;

        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "test sha1 via grpc").unwrap();

        let resp = svc
            .hash_batch(Request::new(HashBatchRequest {
                files: vec![crate::proto::FileToHash {
                    absolute_path: tmp.path().to_string_lossy().to_string(),
                    length: 0,
                    last_modified: 0,
                }],
                algorithm: "SHA-1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results[0].hash_bytes.len(), 20);
        assert!(!resp.results[0].error);
    }

    #[tokio::test]
    async fn test_gRPC_unsupported_algorithm() {
        let svc = HashServiceImpl;

        let result = svc
            .hash_batch(Request::new(HashBatchRequest {
                files: vec![],
                algorithm: "BLAKE3".to_string(),
            }))
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_gRPC_nonexistent_file_returns_error_result() {
        let svc = HashServiceImpl;

        let resp = svc
            .hash_batch(Request::new(HashBatchRequest {
                files: vec![crate::proto::FileToHash {
                    absolute_path: "/tmp/this_file_definitely_does_not_exist_12345".to_string(),
                    length: 0,
                    last_modified: 0,
                }],
                algorithm: "MD5".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.results.len(), 1);
        assert!(resp.results[0].error, "expected error flag for non-existent file");
        assert!(
            !resp.results[0].error_message.is_empty(),
            "expected a non-empty error message"
        );
        assert!(
            resp.results[0].hash_bytes.is_empty(),
            "expected empty hash bytes for non-existent file"
        );
    }

    #[tokio::test]
    async fn test_gRPC_batch_mix_of_existing_and_nonexistent() {
        let svc = HashServiceImpl;

        let mut existing = NamedTempFile::new().unwrap();
        write!(existing, "I exist").unwrap();

        let resp = svc
            .hash_batch(Request::new(HashBatchRequest {
                files: vec![
                    crate::proto::FileToHash {
                        absolute_path: existing.path().to_string_lossy().to_string(),
                        length: 0,
                        last_modified: 0,
                    },
                    crate::proto::FileToHash {
                        absolute_path: "/tmp/no_such_file_xyz_987".to_string(),
                        length: 0,
                        last_modified: 0,
                    },
                ],
                algorithm: "MD5".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.results.len(), 2);

        // First file: should succeed
        assert!(
            !resp.results[0].error,
            "first file should succeed"
        );
        assert_eq!(resp.results[0].hash_bytes.len(), 16);
        assert!(resp.results[0].error_message.is_empty());

        // Second file: should fail gracefully
        assert!(
            resp.results[1].error,
            "second file should have error flag"
        );
        assert!(resp.results[1].hash_bytes.is_empty());
        assert!(
            !resp.results[1].error_message.is_empty(),
            "second file should have an error message"
        );
    }

    #[tokio::test]
    async fn test_gRPC_hash_empty_file_via_service() {
        let svc = HashServiceImpl;

        let tmp = NamedTempFile::new().unwrap();

        let resp = svc
            .hash_batch(Request::new(HashBatchRequest {
                files: vec![crate::proto::FileToHash {
                    absolute_path: tmp.path().to_string_lossy().to_string(),
                    length: 0,
                    last_modified: 0,
                }],
                algorithm: "MD5".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.results.len(), 1);
        assert!(!resp.results[0].error);
        // An empty file still produces a 16-byte MD5 hash (the Gradle signature prefix alone)
        assert_eq!(resp.results[0].hash_bytes.len(), 16);
        // The hash of an empty file with Gradle signature should equal MD5(signature || empty)
        let expected = {
            let mut hasher = Md5::new();
            hasher.update(compute_gradle_signature());
            hasher.finalize().to_vec()
        };
        assert_eq!(resp.results[0].hash_bytes, expected);
    }

    #[tokio::test]
    async fn test_gRPC_hash_same_file_twice_is_deterministic() {
        let svc = HashServiceImpl;

        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "deterministic content for gRPC test").unwrap();

        let path_str = tmp.path().to_string_lossy().to_string();

        let request = HashBatchRequest {
            files: vec![crate::proto::FileToHash {
                absolute_path: path_str.clone(),
                length: 0,
                last_modified: 0,
            }],
            algorithm: "SHA-256".to_string(),
        };

        let resp1 = svc
            .hash_batch(Request::new(request.clone()))
            .await
            .unwrap()
            .into_inner();

        let resp2 = svc
            .hash_batch(Request::new(request))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp1.results.len(), 1);
        assert_eq!(resp2.results.len(), 1);
        assert!(!resp1.results[0].error);
        assert!(!resp2.results[0].error);
        assert_eq!(
            resp1.results[0].hash_bytes, resp2.results[0].hash_bytes,
            "hashing the same file twice must produce identical results"
        );
    }

    #[tokio::test]
    async fn test_gRPC_hash_large_file_1mb_produces_non_empty_hash() {
        let svc = HashServiceImpl;

        let mut tmp = NamedTempFile::new().unwrap();
        // Write 1 MB of pseudo-random-looking data
        let data: Vec<u8> = (0..1_048_576).map(|i| (i % 256) as u8).collect();
        tmp.write_all(&data).unwrap();

        let resp = svc
            .hash_batch(Request::new(HashBatchRequest {
                files: vec![crate::proto::FileToHash {
                    absolute_path: tmp.path().to_string_lossy().to_string(),
                    length: 0,
                    last_modified: 0,
                }],
                algorithm: "MD5".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.results.len(), 1);
        assert!(!resp.results[0].error, "large file should hash without error");
        assert_eq!(resp.results[0].hash_bytes.len(), 16, "MD5 hash must be 16 bytes");
        // Verify at least one byte is non-zero (the hash of 1MB of data should not be all zeros)
        assert!(
            resp.results[0].hash_bytes.iter().any(|&b| b != 0),
            "hash of 1 MB file should not be all zeros"
        );
    }

    #[tokio::test]
    async fn test_gRPC_hash_batch_duplicate_paths_returns_consistent_results() {
        let svc = HashServiceImpl;

        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "duplicate path content").unwrap();

        let path_str = tmp.path().to_string_lossy().to_string();

        let resp = svc
            .hash_batch(Request::new(HashBatchRequest {
                files: vec![
                    crate::proto::FileToHash {
                        absolute_path: path_str.clone(),
                        length: 0,
                        last_modified: 0,
                    },
                    crate::proto::FileToHash {
                        absolute_path: path_str.clone(),
                        length: 0,
                        last_modified: 0,
                    },
                    crate::proto::FileToHash {
                        absolute_path: path_str,
                        length: 0,
                        last_modified: 0,
                    },
                ],
                algorithm: "SHA-256".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.results.len(), 3);
        // All three results should succeed and produce identical hashes
        for result in &resp.results {
            assert!(!result.error, "duplicate path entry should succeed");
            assert_eq!(result.hash_bytes.len(), 32, "SHA-256 hash must be 32 bytes");
        }
        assert_eq!(
            resp.results[0].hash_bytes, resp.results[1].hash_bytes,
            "first and second duplicate must match"
        );
        assert_eq!(
            resp.results[0].hash_bytes, resp.results[2].hash_bytes,
            "first and third duplicate must match"
        );
    }

    #[tokio::test]
    async fn test_gRPC_hash_batch_sha256_produces_32_byte_hash() {
        let svc = HashServiceImpl;

        // Create three files with different content
        let mut tmp1 = NamedTempFile::new().unwrap();
        write!(tmp1, "alpha").unwrap();
        let mut tmp2 = NamedTempFile::new().unwrap();
        write!(tmp2, "beta").unwrap();
        let mut tmp3 = NamedTempFile::new().unwrap();
        write!(tmp3, "gamma").unwrap();

        let resp = svc
            .hash_batch(Request::new(HashBatchRequest {
                files: vec![
                    crate::proto::FileToHash {
                        absolute_path: tmp1.path().to_string_lossy().to_string(),
                        length: 0,
                        last_modified: 0,
                    },
                    crate::proto::FileToHash {
                        absolute_path: tmp2.path().to_string_lossy().to_string(),
                        length: 0,
                        last_modified: 0,
                    },
                    crate::proto::FileToHash {
                        absolute_path: tmp3.path().to_string_lossy().to_string(),
                        length: 0,
                        last_modified: 0,
                    },
                ],
                algorithm: "SHA-256".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.results.len(), 3);
        for result in &resp.results {
            assert!(!result.error, "each file should hash successfully with SHA-256");
            assert_eq!(
                result.hash_bytes.len(),
                32,
                "SHA-256 must always produce a 32-byte hash"
            );
        }
        // Different content must produce different hashes
        assert_ne!(resp.results[0].hash_bytes, resp.results[1].hash_bytes);
        assert_ne!(resp.results[1].hash_bytes, resp.results[2].hash_bytes);
        assert_ne!(resp.results[0].hash_bytes, resp.results[2].hash_bytes);
    }

    #[tokio::test]
    async fn test_gRPC_hash_same_file_with_md5_and_sha256_produces_different_results() {
        let svc = HashServiceImpl;

        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "multi-algorithm test data").unwrap();

        let path_str = tmp.path().to_string_lossy().to_string();
        let file_entry = crate::proto::FileToHash {
            absolute_path: path_str,
            length: 0,
            last_modified: 0,
        };

        // Hash with MD5
        let md5_resp = svc
            .hash_batch(Request::new(HashBatchRequest {
                files: vec![file_entry.clone()],
                algorithm: "MD5".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        // Hash with SHA-256
        let sha256_resp = svc
            .hash_batch(Request::new(HashBatchRequest {
                files: vec![file_entry],
                algorithm: "SHA-256".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!md5_resp.results[0].error);
        assert!(!sha256_resp.results[0].error);

        // MD5 is 16 bytes, SHA-256 is 32 bytes -- different lengths
        assert_eq!(md5_resp.results[0].hash_bytes.len(), 16);
        assert_eq!(sha256_resp.results[0].hash_bytes.len(), 32);
        assert_ne!(
            md5_resp.results[0].hash_bytes,
            sha256_resp.results[0].hash_bytes,
            "MD5 and SHA-256 hashes must differ"
        );

        // Also hash with SHA-1 for good measure
        let sha1_resp = svc
            .hash_batch(Request::new(HashBatchRequest {
                files: vec![crate::proto::FileToHash {
                    absolute_path: tmp.path().to_string_lossy().to_string(),
                    length: 0,
                    last_modified: 0,
                }],
                algorithm: "SHA-1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!sha1_resp.results[0].error);
        assert_eq!(sha1_resp.results[0].hash_bytes.len(), 20);
    }

    #[tokio::test]
    async fn test_gRPC_hash_file_with_special_characters_in_path() {
        let svc = HashServiceImpl;

        let temp_dir = tempfile::tempdir().unwrap();

        // Create files with spaces, unicode, and mixed characters in their names
        let special_names = [
            "file with spaces.txt",
            "\u{00e9}ntrepr\u{00ee}se.txt", // "entreprise" with accented chars
            "\u{4f60}\u{597d}\u{4e16}\u{754c}.txt", // "你好世界.txt" (Chinese)
            "cafe\u{0301}.txt",             // "cafe\u{0301}.txt" with combining accent
            "file with (parens) & ampersand.txt",
        ];

        let mut files_to_hash = Vec::new();
        for name in &special_names {
            let file_path = temp_dir.path().join(name);
            let mut f = File::create(&file_path).unwrap();
            write!(f, "content for {name}").unwrap();
            files_to_hash.push(crate::proto::FileToHash {
                absolute_path: file_path.to_string_lossy().to_string(),
                length: 0,
                last_modified: 0,
            });
        }

        let resp = svc
            .hash_batch(Request::new(HashBatchRequest {
                files: files_to_hash,
                algorithm: "MD5".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(
            resp.results.len(),
            special_names.len(),
            "should have one result per file"
        );
        for result in &resp.results {
            assert!(
                !result.error,
                "file at '{}' should hash without error: {}",
                result.absolute_path,
                result.error_message
            );
            assert_eq!(
                result.hash_bytes.len(),
                16,
                "MD5 hash for '{}' must be 16 bytes",
                result.absolute_path
            );
            // Each unique content must produce a non-zero hash
            assert!(
                result.hash_bytes.iter().any(|&b| b != 0),
                "hash for '{}' should not be all zeros",
                result.absolute_path
            );
        }
    }
}
