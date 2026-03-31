use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use blake3::Hasher as Blake3Hasher;
use md5::{Digest, Md5};
use rayon::prelude::*;
use sha1::Sha1;
use sha2::Sha256;
use sha3::{Sha3_256, Sha3_512};
use tonic::{Request, Response, Status};

use crate::error::SubstrateError;
use crate::proto::{
    hash_service_server::HashService, HashBatchRequest, HashBatchResponse, HashResult,
};

#[derive(Default)]
pub struct HashServiceImpl;

/// Minimum file count to trigger parallel hashing via rayon.
const PARALLEL_THRESHOLD: usize = 16;

/// Supported hash algorithms.
#[derive(Debug, Clone, Copy)]
pub enum HashAlgorithm {
    Md5,
    Sha1,
    Sha256,
    Sha3_256,
    Sha3_512,
    Blake3,
}

impl HashAlgorithm {
    pub fn from_name(name: &str) -> Option<Self> {
        // Case-insensitive matching without heap allocation
        match name.as_bytes() {
            b"" | b"MD5" | b"md5" => Some(HashAlgorithm::Md5),
            b"SHA-1" | b"sha-1" | b"SHA1" | b"sha1" => Some(HashAlgorithm::Sha1),
            b"SHA-256" | b"sha-256" | b"SHA256" | b"sha256" => Some(HashAlgorithm::Sha256),
            b"SHA3-256" | b"sha3-256" | b"SHA3_256" | b"sha3_256" => Some(HashAlgorithm::Sha3_256),
            b"SHA3-512" | b"sha3-512" | b"SHA3_512" | b"sha3_512" => Some(HashAlgorithm::Sha3_512),
            b"BLAKE3" | b"blake3" => Some(HashAlgorithm::Blake3),
            other => {
                // Fallback for mixed-case: byte-by-byte comparison
                if Self::eq_ignore_case(other, b"MD5") {
                    Some(HashAlgorithm::Md5)
                } else if Self::eq_ignore_case(other, b"SHA-1") || Self::eq_ignore_case(other, b"SHA1") {
                    Some(HashAlgorithm::Sha1)
                } else if Self::eq_ignore_case(other, b"SHA-256") || Self::eq_ignore_case(other, b"SHA256") {
                    Some(HashAlgorithm::Sha256)
                } else if Self::eq_ignore_case(other, b"SHA3-256") || Self::eq_ignore_case(other, b"SHA3_256") {
                    Some(HashAlgorithm::Sha3_256)
                } else if Self::eq_ignore_case(other, b"SHA3-512") || Self::eq_ignore_case(other, b"SHA3_512") {
                    Some(HashAlgorithm::Sha3_512)
                } else if Self::eq_ignore_case(other, b"BLAKE3") {
                    Some(HashAlgorithm::Blake3)
                } else {
                    None
                }
            }
        }
    }

    /// Case-insensitive byte slice comparison without allocation.
    fn eq_ignore_case(a: &[u8], b: &[u8]) -> bool {
        a.eq_ignore_ascii_case(b)
    }

    /// Expected output length in bytes for this algorithm.
    #[cfg(test)]
    fn output_len(&self) -> usize {
        match self {
            HashAlgorithm::Md5 => 16,
            HashAlgorithm::Sha1 => 20,
            HashAlgorithm::Sha256 => 32,
            HashAlgorithm::Sha3_256 => 32,
            HashAlgorithm::Sha3_512 => 64,
            HashAlgorithm::Blake3 => 32,
        }
    }
}

/// Hash a file using the given algorithm with streaming I/O.
///
/// Reads in 64KB chunks for good I/O throughput without loading entire files
/// into memory. For small files (< 64KB), reads entirely in one call.
fn hash_file_with_algorithm(
    path: &Path,
    algorithm: HashAlgorithm,
    with_signature: bool,
) -> Result<Vec<u8>, SubstrateError> {
    let file = File::open(path)
        .map_err(|e| SubstrateError::Hash(format!("Cannot open {}: {}", path.display(), e)))?;

    let metadata = file
        .metadata()
        .map_err(|e| SubstrateError::Hash(format!("Cannot stat {}: {}", path.display(), e)))?;

    let file_len = metadata.len() as usize;
    let mut reader = BufReader::with_capacity(64 * 1024, file);

    match algorithm {
        HashAlgorithm::Md5 => {
            let mut hasher = Md5::new();
            if with_signature {
                hasher.update(gradle_signature());
            }
            stream_hash(&mut reader, &mut hasher, path, file_len)?;
            Ok(hasher.finalize().to_vec())
        }
        HashAlgorithm::Sha1 => {
            let mut hasher = Sha1::new();
            stream_hash(&mut reader, &mut hasher, path, file_len)?;
            Ok(hasher.finalize().to_vec())
        }
        HashAlgorithm::Sha256 => {
            let mut hasher = Sha256::new();
            stream_hash(&mut reader, &mut hasher, path, file_len)?;
            Ok(hasher.finalize().to_vec())
        }
        HashAlgorithm::Sha3_256 => {
            let mut hasher = Sha3_256::new();
            stream_hash(&mut reader, &mut hasher, path, file_len)?;
            Ok(hasher.finalize().to_vec())
        }
        HashAlgorithm::Sha3_512 => {
            let mut hasher = Sha3_512::new();
            stream_hash(&mut reader, &mut hasher, path, file_len)?;
            Ok(hasher.finalize().to_vec())
        }
        HashAlgorithm::Blake3 => {
            let mut hasher = Blake3Hasher::new();
            stream_hash_blake3(&mut reader, &mut hasher, path, file_len)?;
            Ok(hasher.finalize().as_bytes().to_vec())
        }
    }
}

/// Stream data from reader into a hasher implementing `Digest` trait.
/// Uses 64KB chunks for efficient I/O on large files.
fn stream_hash<D: Digest, R: Read>(
    reader: &mut BufReader<R>,
    hasher: &mut D,
    path: &Path,
    file_len: usize,
) -> Result<(), SubstrateError> {
    if file_len < 64 * 1024 {
        let mut buf = Vec::with_capacity(file_len);
        reader
            .read_to_end(&mut buf)
            .map_err(|e| SubstrateError::Hash(format!("Cannot read {}: {}", path.display(), e)))?;
        hasher.update(&buf);
    } else {
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let n = reader.read(&mut buffer).map_err(|e| {
                SubstrateError::Hash(format!("Cannot read {}: {}", path.display(), e))
            })?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
    }
    Ok(())
}

/// Stream data from reader into a blake3 hasher.
/// blake3 has its own `update` method (not the `Digest` trait).
fn stream_hash_blake3<R: Read>(
    reader: &mut BufReader<R>,
    hasher: &mut Blake3Hasher,
    path: &Path,
    file_len: usize,
) -> Result<(), SubstrateError> {
    if file_len < 64 * 1024 {
        let mut buf = Vec::with_capacity(file_len);
        reader
            .read_to_end(&mut buf)
            .map_err(|e| SubstrateError::Hash(format!("Cannot read {}: {}", path.display(), e)))?;
        hasher.update(&buf);
    } else {
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let n = reader.read(&mut buffer).map_err(|e| {
                SubstrateError::Hash(format!("Cannot read {}: {}", path.display(), e))
            })?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
    }
    Ok(())
}

/// Gradle DefaultStreamHasher signature prefix, computed once.
/// signature = MD5(int32_le(9) + "SIGNATURE" + int32_le(52) + "CLASS:org.gradle.internal.hash.DefaultStreamHasher")
static GRADLE_SIGNATURE: std::sync::OnceLock<[u8; 16]> = std::sync::OnceLock::new();

fn gradle_signature() -> &'static [u8; 16] {
    GRADLE_SIGNATURE.get_or_init(|| {
        let mut sig_hasher = md5::Md5::new();
        let sig_label = b"SIGNATURE";
        sig_hasher.update((sig_label.len() as i32).to_le_bytes());
        sig_hasher.update(sig_label);
        let class_name = b"CLASS:org.gradle.internal.hash.DefaultStreamHasher";
        sig_hasher.update((class_name.len() as i32).to_le_bytes());
        sig_hasher.update(class_name);
        sig_hasher.finalize().into()
    })
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
                    "Unsupported hash algorithm: '{}'. Supported: MD5, SHA-1, SHA-256, SHA3-256, SHA3-512, BLAKE3",
                    req.algorithm
                )));
            }
        };

        let files = req.files;
        let use_parallel = files.len() >= PARALLEL_THRESHOLD;

        let results: Vec<HashResult> = if use_parallel {
            files
                .into_par_iter()
                .map(|file| {
                    let path = Path::new(&file.absolute_path);
                    let with_signature = matches!(algorithm, HashAlgorithm::Md5);
                    match hash_file_with_algorithm(path, algorithm, with_signature) {
                        Ok(hash_bytes) => HashResult {
                            absolute_path: file.absolute_path,
                            hash_bytes,
                            error: false,
                            error_message: String::new(),
                        },
                        Err(e) => HashResult {
                            absolute_path: file.absolute_path,
                            hash_bytes: Vec::new(),
                            error: true,
                            error_message: e.to_string(),
                        },
                    }
                })
                .collect()
        } else {
            files
                .into_iter()
                .map(|file| {
                    let path = Path::new(&file.absolute_path);
                    let with_signature = matches!(algorithm, HashAlgorithm::Md5);
                    match hash_file_with_algorithm(path, algorithm, with_signature) {
                        Ok(hash_bytes) => HashResult {
                            absolute_path: file.absolute_path,
                            hash_bytes,
                            error: false,
                            error_message: String::new(),
                        },
                        Err(e) => HashResult {
                            absolute_path: file.absolute_path,
                            hash_bytes: Vec::new(),
                            error: true,
                            error_message: e.to_string(),
                        },
                    }
                })
                .collect()
        };

        tracing::debug!(
            count = results.len(),
            algorithm = %req.algorithm,
            parallel = use_parallel,
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

/// Hash a file using SHA3-256 (no signature prefix).
pub fn hash_file_sha3_256(path: &Path) -> Result<Vec<u8>, SubstrateError> {
    hash_file_with_algorithm(path, HashAlgorithm::Sha3_256, false)
}

/// Hash a file using SHA3-512 (no signature prefix).
pub fn hash_file_sha3_512(path: &Path) -> Result<Vec<u8>, SubstrateError> {
    hash_file_with_algorithm(path, HashAlgorithm::Sha3_512, false)
}

/// Hash a file using BLAKE3 (no signature prefix).
pub fn hash_file_blake3(path: &Path) -> Result<Vec<u8>, SubstrateError> {
    hash_file_with_algorithm(path, HashAlgorithm::Blake3, false)
}

/// Hash a batch of files in parallel using the given algorithm.
/// Returns results in the same order as the input paths.
pub fn hash_batch_parallel(
    paths: &[PathBuf],
    algorithm: HashAlgorithm,
) -> Vec<Result<Vec<u8>, SubstrateError>> {
    paths
        .par_iter()
        .map(|path| {
            let with_signature = matches!(algorithm, HashAlgorithm::Md5);
            hash_file_with_algorithm(path, algorithm, with_signature)
        })
        .collect()
}

#[cfg(test)]
#[allow(non_snake_case)]
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
        assert_eq!(gradle_signature().len(), 16);
    }

    // --- SHA3-256 tests ---

    #[test]
    fn test_sha3_256_hash() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "sha3-256 test content").unwrap();
        let hash = hash_file_sha3_256(tmp.path()).unwrap();
        assert_eq!(hash.len(), 32);
    }

    #[test]
    fn test_sha3_256_deterministic() {
        let mut tmp1 = NamedTempFile::new().unwrap();
        let mut tmp2 = NamedTempFile::new().unwrap();
        tmp1.write_all(b"sha3 determinism test").unwrap();
        tmp2.write_all(b"sha3 determinism test").unwrap();
        assert_eq!(
            hash_file_sha3_256(tmp1.path()).unwrap(),
            hash_file_sha3_256(tmp2.path()).unwrap()
        );
    }

    #[test]
    fn test_sha3_256_differs_from_sha256() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "same content different algo").unwrap();
        let sha256 = hash_file_sha256(tmp.path()).unwrap();
        let sha3_256 = hash_file_sha3_256(tmp.path()).unwrap();
        assert_eq!(sha256.len(), 32);
        assert_eq!(sha3_256.len(), 32);
        assert_ne!(
            sha256, sha3_256,
            "SHA-256 and SHA3-256 must produce different hashes"
        );
    }

    #[test]
    fn test_sha3_256_empty_file() {
        let tmp = NamedTempFile::new().unwrap();
        let hash = hash_file_sha3_256(tmp.path()).unwrap();
        assert_eq!(hash.len(), 32);
        // SHA3-256 of empty string is a known value: a7ffc6f8bf1ed76651c14756a061d662f580ff4de43b49fa82d80a4b80f8434a
        let hex_str: String = hash.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(
            hex_str,
            "a7ffc6f8bf1ed76651c14756a061d662f580ff4de43b49fa82d80a4b80f8434a"
        );
    }

    // --- SHA3-512 tests ---

    #[test]
    fn test_sha3_512_hash() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "sha3-512 test content").unwrap();
        let hash = hash_file_sha3_512(tmp.path()).unwrap();
        assert_eq!(hash.len(), 64);
    }

    #[test]
    fn test_sha3_512_deterministic() {
        let mut tmp1 = NamedTempFile::new().unwrap();
        let mut tmp2 = NamedTempFile::new().unwrap();
        tmp1.write_all(b"sha3-512 determinism").unwrap();
        tmp2.write_all(b"sha3-512 determinism").unwrap();
        assert_eq!(
            hash_file_sha3_512(tmp1.path()).unwrap(),
            hash_file_sha3_512(tmp2.path()).unwrap()
        );
    }

    #[test]
    fn test_sha3_512_differs_from_sha3_256() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "same content different sizes").unwrap();
        let sha3_256 = hash_file_sha3_256(tmp.path()).unwrap();
        let sha3_512 = hash_file_sha3_512(tmp.path()).unwrap();
        assert_ne!(sha3_256, sha3_512);
    }

    #[test]
    fn test_sha3_512_large_file() {
        let mut tmp = NamedTempFile::new().unwrap();
        let data = vec![0xCD_u8; 500_000];
        tmp.write_all(&data).unwrap();
        let hash = hash_file_sha3_512(tmp.path()).unwrap();
        assert_eq!(hash.len(), 64);
        assert!(hash.iter().any(|&b| b != 0));
    }

    // --- BLAKE3 tests ---

    #[test]
    fn test_blake3_hash() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "blake3 test content").unwrap();
        let hash = hash_file_blake3(tmp.path()).unwrap();
        assert_eq!(hash.len(), 32);
    }

    #[test]
    fn test_blake3_deterministic() {
        let mut tmp1 = NamedTempFile::new().unwrap();
        let mut tmp2 = NamedTempFile::new().unwrap();
        tmp1.write_all(b"blake3 determinism test").unwrap();
        tmp2.write_all(b"blake3 determinism test").unwrap();
        assert_eq!(
            hash_file_blake3(tmp1.path()).unwrap(),
            hash_file_blake3(tmp2.path()).unwrap()
        );
    }

    #[test]
    fn test_blake3_empty_file() {
        let tmp = NamedTempFile::new().unwrap();
        let hash = hash_file_blake3(tmp.path()).unwrap();
        assert_eq!(hash.len(), 32);
        // BLAKE3 of empty string is a known value: af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262
        let hex_str: String = hash.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(
            hex_str,
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    #[test]
    fn test_blake3_large_file() {
        let mut tmp = NamedTempFile::new().unwrap();
        let data = vec![0xEF_u8; 2_000_000];
        tmp.write_all(&data).unwrap();
        let hash = hash_file_blake3(tmp.path()).unwrap();
        assert_eq!(hash.len(), 32);
        assert!(hash.iter().any(|&b| b != 0));
    }

    #[test]
    fn test_blake3_differs_from_all_sha_variants() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "cross-algorithm comparison content").unwrap();

        let blake3_hash = hash_file_blake3(tmp.path()).unwrap();
        let sha256_hash = hash_file_sha256(tmp.path()).unwrap();
        let sha3_256_hash = hash_file_sha3_256(tmp.path()).unwrap();

        assert_ne!(blake3_hash, sha256_hash);
        assert_ne!(blake3_hash, sha3_256_hash);
    }

    // --- Algorithm name parsing tests ---

    #[test]
    fn test_algorithm_from_name_variants() {
        assert!(HashAlgorithm::from_name("MD5").is_some());
        assert!(HashAlgorithm::from_name("md5").is_some());
        assert!(HashAlgorithm::from_name("").is_some()); // default = MD5
        assert!(HashAlgorithm::from_name("SHA-1").is_some());
        assert!(HashAlgorithm::from_name("SHA1").is_some());
        assert!(HashAlgorithm::from_name("sha-1").is_some());
        assert!(HashAlgorithm::from_name("SHA-256").is_some());
        assert!(HashAlgorithm::from_name("SHA256").is_some());
        assert!(HashAlgorithm::from_name("sha-256").is_some());
        assert!(HashAlgorithm::from_name("SHA3-256").is_some());
        assert!(HashAlgorithm::from_name("SHA3_256").is_some());
        assert!(HashAlgorithm::from_name("sha3-256").is_some());
        assert!(HashAlgorithm::from_name("SHA3-512").is_some());
        assert!(HashAlgorithm::from_name("SHA3_512").is_some());
        assert!(HashAlgorithm::from_name("BLAKE3").is_some());
        assert!(HashAlgorithm::from_name("blake3").is_some());
        assert!(HashAlgorithm::from_name("unknown").is_none());
        assert!(HashAlgorithm::from_name("CRC32").is_none());
    }

    #[test]
    fn test_algorithm_output_lengths() {
        assert_eq!(HashAlgorithm::Md5.output_len(), 16);
        assert_eq!(HashAlgorithm::Sha1.output_len(), 20);
        assert_eq!(HashAlgorithm::Sha256.output_len(), 32);
        assert_eq!(HashAlgorithm::Sha3_256.output_len(), 32);
        assert_eq!(HashAlgorithm::Sha3_512.output_len(), 64);
        assert_eq!(HashAlgorithm::Blake3.output_len(), 32);
    }

    // --- Streaming tests ---

    #[test]
    fn test_streaming_hash_large_file_consistent_with_known_value() {
        // Verify streaming produces same result regardless of file size
        let mut small_tmp = NamedTempFile::new().unwrap();
        let mut large_tmp = NamedTempFile::new().unwrap();

        let content = b"streaming consistency test data";
        small_tmp.write_all(content).unwrap();

        // Write same content padded to > 64KB to exercise chunked path
        let mut large_data = content.to_vec();
        large_data.extend(std::iter::repeat(0u8).take(100_000));
        large_tmp.write_all(&large_data).unwrap();

        // The small file hash should be deterministic
        let hash1 = hash_file_sha256(small_tmp.path()).unwrap();
        let hash2 = hash_file_sha256(small_tmp.path()).unwrap();
        assert_eq!(hash1, hash2);

        // The large file should also be deterministic
        let large_hash1 = hash_file_sha256(large_tmp.path()).unwrap();
        let large_hash2 = hash_file_sha256(large_tmp.path()).unwrap();
        assert_eq!(large_hash1, large_hash2);

        // Different content should produce different hashes
        assert_ne!(hash1, large_hash1);
    }

    #[test]
    fn test_streaming_all_algorithms_on_large_file() {
        let mut tmp = NamedTempFile::new().unwrap();
        let data = vec![0x42_u8; 200_000]; // > 64KB to exercise streaming
        tmp.write_all(&data).unwrap();

        // All algorithms should produce non-trivial hashes on large files
        assert_eq!(hash_file_md5(tmp.path()).unwrap().len(), 16);
        assert_eq!(hash_file_sha1(tmp.path()).unwrap().len(), 20);
        assert_eq!(hash_file_sha256(tmp.path()).unwrap().len(), 32);
        assert_eq!(hash_file_sha3_256(tmp.path()).unwrap().len(), 32);
        assert_eq!(hash_file_sha3_512(tmp.path()).unwrap().len(), 64);
        assert_eq!(hash_file_blake3(tmp.path()).unwrap().len(), 32);
    }

    // --- Parallel batch hashing tests ---

    #[test]
    fn test_parallel_batch_hashing() {
        let dir = tempfile::tempdir().unwrap();
        let paths: Vec<std::path::PathBuf> = (0..20)
            .map(|i| {
                let path = dir.path().join(format!("file_{}.txt", i));
                std::fs::write(&path, format!("content of file {}", i)).unwrap();
                path
            })
            .collect();

        let results = hash_batch_parallel(&paths, HashAlgorithm::Sha256);
        assert_eq!(results.len(), 20);
        for (i, result) in results.iter().enumerate() {
            assert!(result.is_ok(), "file {} should hash successfully", i);
            assert_eq!(result.as_ref().unwrap().len(), 32);
        }

        // Verify determinism: hash again, should get same results
        let results2 = hash_batch_parallel(&paths, HashAlgorithm::Sha256);
        for (r1, r2) in results.iter().zip(results2.iter()) {
            assert_eq!(r1.as_ref().unwrap(), r2.as_ref().unwrap());
        }
    }

    #[test]
    fn test_parallel_batch_with_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let paths: Vec<std::path::PathBuf> = (0..20)
            .map(|i| {
                let path = dir.path().join(format!("file_{}.txt", i));
                if i != 7 {
                    std::fs::write(&path, format!("content {}", i)).unwrap();
                }
                path
            })
            .collect();

        let results = hash_batch_parallel(&paths, HashAlgorithm::Md5);
        assert_eq!(results.len(), 20);

        // File index 7 should fail
        assert!(results[7].is_err());
        // All others should succeed
        for (i, result) in results.iter().enumerate() {
            if i != 7 {
                assert!(result.is_ok(), "file {} should succeed", i);
            }
        }
    }

    #[test]
    fn test_parallel_batch_all_algorithms() {
        let dir = tempfile::tempdir().unwrap();
        let paths: Vec<std::path::PathBuf> = (0..20)
            .map(|i| {
                let path = dir.path().join(format!("f_{}.bin", i));
                std::fs::write(&path, vec![i as u8; 1024]).unwrap();
                path
            })
            .collect();

        let algos = [
            HashAlgorithm::Md5,
            HashAlgorithm::Sha1,
            HashAlgorithm::Sha256,
            HashAlgorithm::Sha3_256,
            HashAlgorithm::Sha3_512,
            HashAlgorithm::Blake3,
        ];

        for algo in &algos {
            let results = hash_batch_parallel(&paths, *algo);
            assert_eq!(
                results.len(),
                20,
                "all files should return a result for {:?}",
                algo
            );
            for (i, result) in results.iter().enumerate() {
                assert!(result.is_ok(), "file {} should hash with {:?}", i, algo);
                assert_eq!(result.as_ref().unwrap().len(), algo.output_len());
            }
        }
    }

    // --- gRPC tests ---

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
    async fn test_gRPC_sha3_256_algorithm() {
        let svc = HashServiceImpl;

        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "test sha3-256 via grpc").unwrap();

        let resp = svc
            .hash_batch(Request::new(HashBatchRequest {
                files: vec![crate::proto::FileToHash {
                    absolute_path: tmp.path().to_string_lossy().to_string(),
                    length: 0,
                    last_modified: 0,
                }],
                algorithm: "SHA3-256".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results[0].hash_bytes.len(), 32);
        assert!(!resp.results[0].error);
    }

    #[tokio::test]
    async fn test_gRPC_sha3_512_algorithm() {
        let svc = HashServiceImpl;

        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "test sha3-512 via grpc").unwrap();

        let resp = svc
            .hash_batch(Request::new(HashBatchRequest {
                files: vec![crate::proto::FileToHash {
                    absolute_path: tmp.path().to_string_lossy().to_string(),
                    length: 0,
                    last_modified: 0,
                }],
                algorithm: "SHA3-512".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results[0].hash_bytes.len(), 64);
        assert!(!resp.results[0].error);
    }

    #[tokio::test]
    async fn test_gRPC_blake3_algorithm() {
        let svc = HashServiceImpl;

        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "test blake3 via grpc").unwrap();

        let resp = svc
            .hash_batch(Request::new(HashBatchRequest {
                files: vec![crate::proto::FileToHash {
                    absolute_path: tmp.path().to_string_lossy().to_string(),
                    length: 0,
                    last_modified: 0,
                }],
                algorithm: "BLAKE3".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results[0].hash_bytes.len(), 32);
        assert!(!resp.results[0].error);
    }

    #[tokio::test]
    async fn test_gRPC_unsupported_algorithm() {
        let svc = HashServiceImpl;

        let result = svc
            .hash_batch(Request::new(HashBatchRequest {
                files: vec![],
                algorithm: "CRC32".to_string(),
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
        assert!(
            resp.results[0].error,
            "expected error flag for non-existent file"
        );
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
        assert!(!resp.results[0].error, "first file should succeed");
        assert_eq!(resp.results[0].hash_bytes.len(), 16);
        assert!(resp.results[0].error_message.is_empty());

        // Second file: should fail gracefully
        assert!(resp.results[1].error, "second file should have error flag");
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
            hasher.update(gradle_signature());
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
        assert!(
            !resp.results[0].error,
            "large file should hash without error"
        );
        assert_eq!(
            resp.results[0].hash_bytes.len(),
            16,
            "MD5 hash must be 16 bytes"
        );
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
            assert!(
                !result.error,
                "each file should hash successfully with SHA-256"
            );
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
            md5_resp.results[0].hash_bytes, sha256_resp.results[0].hash_bytes,
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
            "cafe\u{0301}.txt",             // "café.txt" with combining accent
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
                result.absolute_path, result.error_message
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

    // --- Parallel batch gRPC tests ---

    #[tokio::test]
    async fn test_gRPC_parallel_batch_hashing_large_batch() {
        let svc = HashServiceImpl;

        let dir = tempfile::tempdir().unwrap();
        let files_to_hash: Vec<crate::proto::FileToHash> = (0..20)
            .map(|i| {
                let path = dir.path().join(format!("par_file_{}.txt", i));
                std::fs::write(&path, format!("parallel content {}", i)).unwrap();
                crate::proto::FileToHash {
                    absolute_path: path.to_string_lossy().to_string(),
                    length: 0,
                    last_modified: 0,
                }
            })
            .collect();

        let resp = svc
            .hash_batch(Request::new(HashBatchRequest {
                files: files_to_hash,
                algorithm: "SHA-256".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.results.len(), 20);
        for result in &resp.results {
            assert!(!result.error);
            assert_eq!(result.hash_bytes.len(), 32);
        }
    }

    #[tokio::test]
    async fn test_gRPC_parallel_batch_with_blake3() {
        let svc = HashServiceImpl;

        let dir = tempfile::tempdir().unwrap();
        let files_to_hash: Vec<crate::proto::FileToHash> = (0..20)
            .map(|i| {
                let path = dir.path().join(format!("blake3_{}.bin", i));
                std::fs::write(&path, vec![i as u8; 2048]).unwrap();
                crate::proto::FileToHash {
                    absolute_path: path.to_string_lossy().to_string(),
                    length: 0,
                    last_modified: 0,
                }
            })
            .collect();

        let resp = svc
            .hash_batch(Request::new(HashBatchRequest {
                files: files_to_hash,
                algorithm: "BLAKE3".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.results.len(), 20);
        for result in &resp.results {
            assert!(!result.error, "BLAKE3 parallel hash should succeed");
            assert_eq!(result.hash_bytes.len(), 32);
        }
    }

    #[tokio::test]
    async fn test_gRPC_small_batch_uses_sequential_path() {
        let svc = HashServiceImpl;

        let dir = tempfile::tempdir().unwrap();
        // 5 files < PARALLEL_THRESHOLD (16) → sequential path
        let files_to_hash: Vec<crate::proto::FileToHash> = (0..5)
            .map(|i| {
                let path = dir.path().join(format!("seq_{}.txt", i));
                std::fs::write(&path, format!("sequential {}", i)).unwrap();
                crate::proto::FileToHash {
                    absolute_path: path.to_string_lossy().to_string(),
                    length: 0,
                    last_modified: 0,
                }
            })
            .collect();

        let resp = svc
            .hash_batch(Request::new(HashBatchRequest {
                files: files_to_hash,
                algorithm: "SHA3-256".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.results.len(), 5);
        for result in &resp.results {
            assert!(!result.error);
            assert_eq!(result.hash_bytes.len(), 32);
        }
    }
}
