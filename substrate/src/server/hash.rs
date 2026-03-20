use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use md5::{Digest, Md5};
use tonic::{Request, Response, Status};

use crate::error::SubstrateError;
use crate::proto::{
    hash_service_server::HashService, HashBatchRequest, HashBatchResponse, HashResult,
};

pub struct HashServiceImpl;

#[tonic::async_trait]
impl HashService for HashServiceImpl {
    async fn hash_batch(
        &self,
        request: Request<HashBatchRequest>,
    ) -> Result<Response<HashBatchResponse>, Status> {
        let req = request.into_inner();
        let algorithm = req.algorithm.to_uppercase();

        if algorithm != "MD5" && !algorithm.is_empty() {
            return Err(Status::unimplemented(format!(
                "Unsupported hash algorithm: {}",
                algorithm
            )));
        }

        let mut results = Vec::with_capacity(req.files.len());

        for file in req.files {
            let path = Path::new(&file.absolute_path);
            match hash_file_md5(path) {
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

        tracing::debug!(count = results.len(), "Hashed files");
        Ok(Response::new(HashBatchResponse { results }))
    }
}

/// Hash a file using MD5 with Java's DefaultStreamHasher-compatible signature prefix.
///
/// Java's DefaultStreamHasher prepends a signature before hashing file content.
/// The signature is computed by:
///   Hashing.signature(DefaultStreamHasher.class)
/// which calls:
///   signature("CLASS:" + "org.gradle.internal.hash.DefaultStreamHasher")
///
/// The signature computation uses DefaultHasher (not PrimitiveHasher directly):
///   DefaultHasher.putString(str) = PrimitiveHasher.putInt(str.length()) + PrimitiveHasher.putString(str)
///
/// Where PrimitiveHasher.putInt writes 4 bytes little-endian, and
/// PrimitiveHasher.putString writes raw UTF-8 bytes.
///
/// So signature = MD5(int32_le(9) + "SIGNATURE" + int32_le(52) + "CLASS:org.gradle.internal.hash.DefaultStreamHasher")
///
/// The final hash = MD5(signature_16_bytes || file_content_bytes)
pub fn hash_file_md5(path: &Path) -> Result<Vec<u8>, SubstrateError> {
    // Step 1: Compute the signature prefix
    // DefaultHasher.putString writes: PrimitiveHasher.putInt(length) + PrimitiveHasher.putString(utf8_bytes)
    // PrimitiveHasher.putInt writes 4 bytes little-endian
    // PrimitiveHasher.putString writes raw UTF-8 bytes (no length prefix at PrimitiveHasher level)
    let mut sig_hasher = Md5::new();

    // putString("SIGNATURE")
    let sig_label = b"SIGNATURE";
    let sig_label_len = sig_label.len() as i32;
    sig_hasher.update(sig_label_len.to_le_bytes());
    sig_hasher.update(sig_label);

    // putString("CLASS:org.gradle.internal.hash.DefaultStreamHasher")
    let class_name = b"CLASS:org.gradle.internal.hash.DefaultStreamHasher";
    let class_name_len = class_name.len() as i32;
    sig_hasher.update(class_name_len.to_le_bytes());
    sig_hasher.update(class_name);

    let signature_bytes = sig_hasher.finalize();

    // Step 2: Hash signature || file content
    let mut hasher = Md5::new();
    hasher.update(signature_bytes);

    let file = File::open(path).map_err(|e| SubstrateError::Hash(format!(
        "Cannot open {}: {}",
        path.display(),
        e
    )))?;
    let mut reader = BufReader::with_capacity(8192, file);
    let mut buffer = [0u8; 8192];
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
}
