use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use blake3::Hasher as Blake3Hasher;
use md5::{Digest, Md5};
use rayon::prelude::*;
use sha1::Sha1;
use sha2::Sha256;
use sha3::{Sha3_256, Sha3_512};
use tonic::{Request, Response, Status};

use crate::proto::{
    classpath_service_server::ClasspathService, ClasspathDifference, ClasspathEntry,
    ClasspathEntryHash, CompareClasspathsRequest, CompareClasspathsResponse,
    HashClasspathRequest, HashClasspathResponse,
};

#[derive(Default)]
pub struct ClasspathServiceImpl;

/// Minimum entry count to trigger parallel hashing via rayon.
const PARALLEL_THRESHOLD: usize = 8;

/// Hash a single file's content using the specified algorithm.
fn hash_file_content(path: &Path, algorithm_name: &str) -> Option<Vec<u8>> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).ok()?;

    match algorithm_name.to_uppercase().as_str() {
        "" | "MD5" => {
            let mut hasher = Md5::new();
            hasher.update(&buf);
            Some(hasher.finalize().to_vec())
        }
        "SHA-1" | "SHA1" => {
            let mut hasher = Sha1::new();
            hasher.update(&buf);
            Some(hasher.finalize().to_vec())
        }
        "SHA-256" | "SHA256" => {
            let mut hasher = Sha256::new();
            hasher.update(&buf);
            Some(hasher.finalize().to_vec())
        }
        "SHA3-256" | "SHA3_256" => {
            let mut hasher = Sha3_256::new();
            hasher.update(&buf);
            Some(hasher.finalize().to_vec())
        }
        "SHA3-512" | "SHA3_512" => {
            let mut hasher = Sha3_512::new();
            hasher.update(&buf);
            Some(hasher.finalize().to_vec())
        }
        "BLAKE3" => {
            let mut hasher = Blake3Hasher::new();
            hasher.update(&buf);
            Some(hasher.finalize().as_bytes().to_vec())
        }
        _ => None,
    }
}

/// Hash metadata (path + size + mtime) using the specified algorithm.
fn hash_metadata(path: &str, length: i64, last_modified: i64, algorithm_name: &str) -> Vec<u8> {
    let mut data = Vec::new();
    data.extend_from_slice(path.as_bytes());
    data.push(0);
    data.extend_from_slice(&length.to_le_bytes());
    data.extend_from_slice(&last_modified.to_le_bytes());

    match algorithm_name.to_uppercase().as_str() {
        "" | "MD5" => {
            let mut hasher = Md5::new();
            hasher.update(&data);
            hasher.finalize().to_vec()
        }
        "SHA-1" | "SHA1" => {
            let mut hasher = Sha1::new();
            hasher.update(&data);
            hasher.finalize().to_vec()
        }
        "SHA-256" | "SHA256" => {
            let mut hasher = Sha256::new();
            hasher.update(&data);
            hasher.finalize().to_vec()
        }
        "BLAKE3" => {
            let mut hasher = Blake3Hasher::new();
            hasher.update(&data);
            hasher.finalize().as_bytes().to_vec()
        }
        _ => {
            let mut hasher = Md5::new();
            hasher.update(&data);
            hasher.finalize().to_vec()
        }
    }
}

/// Hash a composite of entry hashes (sorted by path) to produce the classpath hash.
fn composite_hash(entry_hashes: &[(String, Vec<u8>)], algorithm_name: &str) -> Vec<u8> {
    let algo = algorithm_name.to_uppercase();
    match algo.as_str() {
        "" | "MD5" => {
            let mut hasher = Md5::new();
            feed_entries(&mut hasher, entry_hashes);
            hasher.finalize().to_vec()
        }
        "SHA-1" | "SHA1" => {
            let mut hasher = Sha1::new();
            feed_entries(&mut hasher, entry_hashes);
            hasher.finalize().to_vec()
        }
        "SHA-256" | "SHA256" => {
            let mut hasher = Sha256::new();
            feed_entries(&mut hasher, entry_hashes);
            hasher.finalize().to_vec()
        }
        "BLAKE3" => {
            let mut hasher = Blake3Hasher::new();
            feed_blake3(&mut hasher, entry_hashes);
            hasher.finalize().as_bytes().to_vec()
        }
        _ => {
            let mut hasher = Md5::new();
            feed_entries(&mut hasher, entry_hashes);
            hasher.finalize().to_vec()
        }
    }
}

fn feed_entries<H: md5::Digest>(hasher: &mut H, entries: &[(String, Vec<u8>)]) {
    for (path, hash) in entries {
        hasher.update(path.as_bytes());
        hasher.update(&[0u8]);
        hasher.update(hash);
        hasher.update(&[0u8]);
    }
}

fn feed_blake3(hasher: &mut Blake3Hasher, entries: &[(String, Vec<u8>)]) {
    for (path, hash) in entries {
        hasher.update(path.as_bytes());
        hasher.update(&[0u8]);
        hasher.update(hash);
        hasher.update(&[0u8]);
    }
}

impl ClasspathServiceImpl {
    fn hash_classpath_impl(
        entries: &[ClasspathEntry],
        algorithm: &str,
        ignore_timestamps: bool,
        include_entry_hashes: bool,
    ) -> (Vec<u8>, Vec<ClasspathEntryHash>) {
        if entries.is_empty() {
            let hash = composite_hash(&[], algorithm);
            return (hash, vec![]);
        }

        let mut sorted: Vec<_> = entries.iter().collect();
        sorted.sort_by_key(|e| &e.absolute_path);

        let entry_hashes: Vec<(String, Vec<u8>)> = if sorted.len() >= PARALLEL_THRESHOLD {
            sorted
                .par_iter()
                .map(|entry| {
                    let hash = if ignore_timestamps {
                        hash_file_content(Path::new(&entry.absolute_path), algorithm)
                            .unwrap_or_else(|| {
                                hash_metadata(
                                    &entry.absolute_path,
                                    entry.length,
                                    entry.last_modified,
                                    algorithm,
                                )
                            })
                    } else {
                        hash_metadata(
                            &entry.absolute_path,
                            entry.length,
                            entry.last_modified,
                            algorithm,
                        )
                    };
                    (entry.absolute_path.clone(), hash)
                })
                .collect()
        } else {
            sorted
                .iter()
                .map(|entry| {
                    let hash = if ignore_timestamps {
                        hash_file_content(Path::new(&entry.absolute_path), algorithm)
                            .unwrap_or_else(|| {
                                hash_metadata(
                                    &entry.absolute_path,
                                    entry.length,
                                    entry.last_modified,
                                    algorithm,
                                )
                            })
                    } else {
                        hash_metadata(
                            &entry.absolute_path,
                            entry.length,
                            entry.last_modified,
                            algorithm,
                        )
                    };
                    (entry.absolute_path.clone(), hash)
                })
                .collect()
        };

        let classpath_hash = composite_hash(&entry_hashes, algorithm);

        let proto_entries = if include_entry_hashes {
            entry_hashes
                .into_iter()
                .map(|(path, hash)| ClasspathEntryHash {
                    absolute_path: path,
                    hash,
                    size: 0,
                })
                .collect()
        } else {
            vec![]
        };

        (classpath_hash, proto_entries)
    }

    fn compare_classpaths_impl(
        previous_hash: &[u8],
        current_entries: &[ClasspathEntry],
        algorithm: &str,
    ) -> (bool, Vec<u8>, Vec<ClasspathDifference>) {
        let (new_hash, _) = Self::hash_classpath_impl(current_entries, algorithm, false, false);

        if previous_hash == new_hash.as_slice() {
            return (false, new_hash, vec![]);
        }

        // Report all current entries as potential differences.
        // A full comparison would require the previous entries list;
        // this is a conservative "changed" detection.
        let mut differences = Vec::new();
        for entry in current_entries {
            differences.push(ClasspathDifference {
                change_type: "modified".to_string(),
                absolute_path: entry.absolute_path.clone(),
            });
        }

        (true, new_hash, differences)
    }
}

#[tonic::async_trait]
impl ClasspathService for ClasspathServiceImpl {
    async fn hash_classpath(
        &self,
        request: Request<HashClasspathRequest>,
    ) -> Result<Response<HashClasspathResponse>, Status> {
        let req = request.into_inner();
        let algorithm = if req.algorithm.is_empty() {
            "MD5"
        } else {
            &req.algorithm
        };

        let (classpath_hash, entries) = Self::hash_classpath_impl(
            &req.entries,
            algorithm,
            req.ignore_timestamps,
            req.include_entry_hashes,
        );

        tracing::debug!(
            entry_count = req.entries.len(),
            algorithm = algorithm,
            "Classpath hashed"
        );

        Ok(Response::new(HashClasspathResponse {
            classpath_hash,
            entries,
            algorithm_used: algorithm.to_string(),
        }))
    }

    async fn compare_classpaths(
        &self,
        request: Request<CompareClasspathsRequest>,
    ) -> Result<Response<CompareClasspathsResponse>, Status> {
        let req = request.into_inner();
        let algorithm = if req.algorithm.is_empty() {
            "MD5"
        } else {
            &req.algorithm
        };

        let (changed, new_hash, differences) =
            Self::compare_classpaths_impl(&req.previous_hash, &req.current_entries, algorithm);

        tracing::debug!(
            changed = changed,
            difference_count = differences.len(),
            "Classpath comparison complete"
        );

        Ok(Response::new(CompareClasspathsResponse {
            changed,
            new_hash,
            differences,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use tempfile::TempDir;

    fn make_entry(path: &str, entry_type: i32, length: i64, last_modified: i64) -> ClasspathEntry {
        ClasspathEntry {
            absolute_path: path.to_string(),
            entry_type,
            length,
            last_modified,
        }
    }

    #[test]
    fn test_empty_classpath_deterministic() {
        let (hash1, entries1) =
            ClasspathServiceImpl::hash_classpath_impl(&[], "MD5", false, false);
        let (hash2, entries2) =
            ClasspathServiceImpl::hash_classpath_impl(&[], "MD5", false, false);
        assert_eq!(hash1, hash2);
        assert!(entries1.is_empty());
        assert!(entries2.is_empty());
    }

    #[test]
    fn test_empty_classpath_sha256() {
        let (hash_md5, _) =
            ClasspathServiceImpl::hash_classpath_impl(&[], "MD5", false, false);
        let (hash_sha256, _) =
            ClasspathServiceImpl::hash_classpath_impl(&[], "SHA-256", false, false);
        assert_ne!(hash_md5, hash_sha256);
        assert_eq!(hash_md5.len(), 16); // MD5 = 16 bytes
        assert_eq!(hash_sha256.len(), 32); // SHA-256 = 32 bytes
    }

    #[test]
    fn test_single_entry_metadata_hash() {
        let entries = vec![make_entry("/tmp/test.jar", 0, 1024, 1000)];
        let (hash, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries, "MD5", false, false);
        assert_eq!(hash.len(), 16);

        // Same entry → same hash
        let entries2 = vec![make_entry("/tmp/test.jar", 0, 1024, 1000)];
        let (hash2, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries2, "MD5", false, false);
        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_multiple_entries_sorted() {
        // Order shouldn't matter — entries are sorted by path
        let entries_a = vec![
            make_entry("/tmp/b.jar", 0, 100, 1000),
            make_entry("/tmp/a.jar", 0, 200, 2000),
        ];
        let entries_b = vec![
            make_entry("/tmp/a.jar", 0, 200, 2000),
            make_entry("/tmp/b.jar", 0, 100, 1000),
        ];
        let (hash_a, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries_a, "MD5", false, false);
        let (hash_b, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries_b, "MD5", false, false);
        assert_eq!(hash_a, hash_b);
    }

    #[test]
    fn test_different_mtime_different_hash() {
        let entries_a = vec![make_entry("/tmp/test.jar", 0, 1024, 1000)];
        let entries_b = vec![make_entry("/tmp/test.jar", 0, 1024, 2000)];
        let (hash_a, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries_a, "MD5", false, false);
        let (hash_b, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries_b, "MD5", false, false);
        assert_ne!(hash_a, hash_b);
    }

    #[test]
    fn test_ignore_timestamps_content_based() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("test.jar");
        let mut f = fs::File::create(&file_path).unwrap();
        f.write_all(b"hello world").unwrap();

        let entries = vec![ClasspathEntry {
            absolute_path: file_path.to_string_lossy().to_string(),
            entry_type: 0,
            length: 11,
            last_modified: 1000,
        }];

        let (hash_a, _) = ClasspathServiceImpl::hash_classpath_impl(
            &entries,
            "MD5",
            true, // ignore_timestamps
            false,
        );

        // Change mtime in metadata, content-based hash should be the same
        let entries2 = vec![ClasspathEntry {
            absolute_path: file_path.to_string_lossy().to_string(),
            entry_type: 0,
            length: 11,
            last_modified: 9999,
        }];
        let (hash_b, _) = ClasspathServiceImpl::hash_classpath_impl(
            &entries2,
            "MD5",
            true,
            false,
        );
        assert_eq!(hash_a, hash_b);
    }

    #[test]
    fn test_include_entry_hashes() {
        let entries = vec![
            make_entry("/tmp/a.jar", 0, 100, 1000),
            make_entry("/tmp/b.jar", 0, 200, 2000),
        ];
        let (_, entry_hashes) =
            ClasspathServiceImpl::hash_classpath_impl(&entries, "MD5", false, true);
        assert_eq!(entry_hashes.len(), 2);
        assert_eq!(entry_hashes[0].absolute_path, "/tmp/a.jar");
        assert_eq!(entry_hashes[1].absolute_path, "/tmp/b.jar");
        assert!(!entry_hashes[0].hash.is_empty());
    }

    #[test]
    fn test_compare_identical_classpaths() {
        let entries = vec![
            make_entry("/tmp/a.jar", 0, 100, 1000),
            make_entry("/tmp/b.jar", 0, 200, 2000),
        ];
        let (hash, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries, "MD5", false, false);

        let (changed, _, differences) =
            ClasspathServiceImpl::compare_classpaths_impl(&hash, &entries, "MD5");
        assert!(!changed);
        assert!(differences.is_empty());
    }

    #[test]
    fn test_compare_different_classpaths() {
        let entries_a = vec![make_entry("/tmp/a.jar", 0, 100, 1000)];
        let (hash_a, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries_a, "MD5", false, false);

        let entries_b = vec![
            make_entry("/tmp/a.jar", 0, 100, 1000),
            make_entry("/tmp/b.jar", 0, 200, 2000),
        ];

        let (changed, _, differences) =
            ClasspathServiceImpl::compare_classpaths_impl(&hash_a, &entries_b, "MD5");
        assert!(changed);
        assert_eq!(differences.len(), 2);
    }

    #[test]
    fn test_blake3_algorithm() {
        let entries = vec![make_entry("/tmp/test.jar", 0, 1024, 1000)];
        let (hash, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries, "BLAKE3", false, false);
        assert_eq!(hash.len(), 32); // BLAKE3 = 32 bytes
    }

    #[test]
    fn test_sha1_algorithm() {
        let entries = vec![make_entry("/tmp/test.jar", 0, 1024, 1000)];
        let (hash, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries, "SHA-1", false, false);
        assert_eq!(hash.len(), 20); // SHA-1 = 20 bytes
    }

    #[test]
    fn test_default_algorithm_is_md5() {
        let entries = vec![make_entry("/tmp/test.jar", 0, 1024, 1000)];
        let (hash_empty, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries, "", false, false);
        let (hash_md5, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries, "MD5", false, false);
        assert_eq!(hash_empty, hash_md5);
    }

    #[test]
    fn test_parallel_hashing_consistent() {
        // 20 entries should trigger parallel path
        let entries: Vec<ClasspathEntry> = (0..20)
            .map(|i| make_entry(&format!("/tmp/lib{}.jar", i), 0, 100 + i, 1000 + i))
            .collect();

        let (hash1, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries, "MD5", false, false);
        let (hash2, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries, "MD5", false, false);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_content_hash_fallback_on_missing_file() {
        // File doesn't exist → falls back to metadata hash
        let entries = vec![make_entry("/nonexistent/path.jar", 0, 1024, 1000)];
        let (hash_ts, _) = ClasspathServiceImpl::hash_classpath_impl(
            &entries,
            "MD5",
            true, // ignore_timestamps → tries content, falls back
            false,
        );
        let (hash_meta, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries, "MD5", false, false);
        // Fallback should produce metadata-based hash
        assert_eq!(hash_ts, hash_meta);
    }
}
