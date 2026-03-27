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

impl ClasspathServiceImpl {
    pub const fn new() -> Self {
        Self
    }
}

/// Minimum entry count to trigger parallel hashing via rayon.
const PARALLEL_THRESHOLD: usize = 8;

/// Resolved hash algorithm — avoids repeated `.to_uppercase()` allocations
/// on every entry in the hot path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HashAlgo {
    Md5,
    Sha1,
    Sha256,
    Sha3_256,
    Sha3_512,
    Blake3,
}

impl HashAlgo {
    fn from_name(name: &str) -> Self {
        match name.to_uppercase().as_str() {
            "" | "MD5" => Self::Md5,
            "SHA-1" | "SHA1" => Self::Sha1,
            "SHA-256" | "SHA256" => Self::Sha256,
            "SHA3-256" | "SHA3_256" => Self::Sha3_256,
            "SHA3-512" | "SHA3_512" => Self::Sha3_512,
            "BLAKE3" => Self::Blake3,
            _ => Self::Md5, // default fallback
        }
    }

    fn display_name(&self) -> &'static str {
        match self {
            Self::Md5 => "MD5",
            Self::Sha1 => "SHA-1",
            Self::Sha256 => "SHA-256",
            Self::Sha3_256 => "SHA3-256",
            Self::Sha3_512 => "SHA3-512",
            Self::Blake3 => "BLAKE3",
        }
    }
}

/// Hash raw bytes using the resolved algorithm.
#[inline]
fn hash_bytes(algo: HashAlgo, data: &[u8]) -> Vec<u8> {
    match algo {
        HashAlgo::Md5 => Md5::digest(data).to_vec(),
        HashAlgo::Sha1 => Sha1::digest(data).to_vec(),
        HashAlgo::Sha256 => Sha256::digest(data).to_vec(),
        HashAlgo::Sha3_256 => Sha3_256::digest(data).to_vec(),
        HashAlgo::Sha3_512 => Sha3_512::digest(data).to_vec(),
        HashAlgo::Blake3 => {
            let mut h = Blake3Hasher::new();
            h.update(data);
            h.finalize().as_bytes().to_vec()
        }
    }
}

/// Hash a single file's content using the specified algorithm.
fn hash_file_content(path: &Path, algo: HashAlgo) -> Option<Vec<u8>> {
    let file = File::open(path).ok()?;
    let mut reader = BufReader::new(file);
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf).ok()?;
    Some(hash_bytes(algo, &buf))
}

/// Hash metadata (path + size + mtime) using the specified algorithm.
fn hash_metadata(path: &str, length: i64, last_modified: i64, algo: HashAlgo) -> Vec<u8> {
    let mut data = Vec::with_capacity(path.len() + 1 + 8 + 8);
    data.extend_from_slice(path.as_bytes());
    data.push(0);
    data.extend_from_slice(&length.to_le_bytes());
    data.extend_from_slice(&last_modified.to_le_bytes());
    hash_bytes(algo, &data)
}

const SEP: &[u8] = &[0];

/// Hash a composite of entry hashes (sorted by path) to produce the classpath hash.
fn composite_hash(entry_hashes: &[(String, Vec<u8>)], algo: HashAlgo) -> Vec<u8> {
    if entry_hashes.is_empty() {
        // Hash empty input to produce a deterministic empty-classpath hash.
        return hash_bytes(algo, b"");
    }

    // Pre-compute total size for a single allocation.
    let estimated = entry_hashes.iter().map(|(p, h)| p.len() + 1 + h.len() + 1).sum::<usize>();
    let mut buf = Vec::with_capacity(estimated);
    for (path, hash) in entry_hashes {
        buf.extend_from_slice(path.as_bytes());
        buf.extend_from_slice(SEP);
        buf.extend_from_slice(hash);
        buf.extend_from_slice(SEP);
    }
    hash_bytes(algo, &buf)
}

/// Hash a single classpath entry (metadata or content-based).
fn hash_entry(entry: &ClasspathEntry, algo: HashAlgo, ignore_timestamps: bool) -> Vec<u8> {
    if ignore_timestamps {
        hash_file_content(Path::new(&entry.absolute_path), algo)
            .unwrap_or_else(|| hash_metadata(&entry.absolute_path, entry.length, entry.last_modified, algo))
    } else {
        hash_metadata(&entry.absolute_path, entry.length, entry.last_modified, algo)
    }
}

impl ClasspathServiceImpl {
    fn hash_classpath_impl(
        entries: &[ClasspathEntry],
        algo: HashAlgo,
        ignore_timestamps: bool,
        include_entry_hashes: bool,
    ) -> (Vec<u8>, Vec<ClasspathEntryHash>) {
        if entries.is_empty() {
            let hash = composite_hash(&[], algo);
            return (hash, vec![]);
        }

        let mut sorted: Vec<_> = entries.iter().collect();
        sorted.sort_unstable_by_key(|e| &e.absolute_path);

        let entry_hashes: Vec<(String, Vec<u8>)> = if sorted.len() >= PARALLEL_THRESHOLD {
            sorted
                .par_iter()
                .map(|entry| {
                    let hash = hash_entry(entry, algo, ignore_timestamps);
                    (entry.absolute_path.clone(), hash)
                })
                .collect()
        } else {
            sorted
                .iter()
                .map(|entry| {
                    let hash = hash_entry(entry, algo, ignore_timestamps);
                    (entry.absolute_path.clone(), hash)
                })
                .collect()
        };

        let classpath_hash = composite_hash(&entry_hashes, algo);

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
        algo: HashAlgo,
    ) -> (bool, Vec<u8>, Vec<ClasspathDifference>) {
        let (new_hash, _) = Self::hash_classpath_impl(current_entries, algo, false, false);

        if previous_hash == new_hash.as_slice() {
            return (false, new_hash, vec![]);
        }

        let mut differences = Vec::with_capacity(current_entries.len());
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
        let algo = if req.algorithm.is_empty() {
            HashAlgo::Md5
        } else {
            HashAlgo::from_name(&req.algorithm)
        };

        let (classpath_hash, entries) = Self::hash_classpath_impl(
            &req.entries,
            algo,
            req.ignore_timestamps,
            req.include_entry_hashes,
        );

        tracing::debug!(
            entry_count = req.entries.len(),
            algorithm = ?algo,
            "Classpath hashed"
        );

        Ok(Response::new(HashClasspathResponse {
            classpath_hash,
            entries,
            algorithm_used: algo.display_name().to_string(),
        }))
    }

    async fn compare_classpaths(
        &self,
        request: Request<CompareClasspathsRequest>,
    ) -> Result<Response<CompareClasspathsResponse>, Status> {
        let req = request.into_inner();
        let algo = if req.algorithm.is_empty() {
            HashAlgo::Md5
        } else {
            HashAlgo::from_name(&req.algorithm)
        };

        let (changed, new_hash, differences) =
            Self::compare_classpaths_impl(&req.previous_hash, &req.current_entries, algo);

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
            ClasspathServiceImpl::hash_classpath_impl(&[], HashAlgo::Md5, false, false);
        let (hash2, entries2) =
            ClasspathServiceImpl::hash_classpath_impl(&[], HashAlgo::Md5, false, false);
        assert_eq!(hash1, hash2);
        assert!(entries1.is_empty());
        assert!(entries2.is_empty());
    }

    #[test]
    fn test_empty_classpath_sha256() {
        let (hash_md5, _) =
            ClasspathServiceImpl::hash_classpath_impl(&[], HashAlgo::Md5, false, false);
        let (hash_sha256, _) =
            ClasspathServiceImpl::hash_classpath_impl(&[], HashAlgo::Sha256, false, false);
        assert_ne!(hash_md5, hash_sha256);
        assert_eq!(hash_md5.len(), 16); // MD5 = 16 bytes
        assert_eq!(hash_sha256.len(), 32); // SHA-256 = 32 bytes
    }

    #[test]
    fn test_single_entry_metadata_hash() {
        let entries = vec![make_entry("/tmp/test.jar", 0, 1024, 1000)];
        let (hash, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries, HashAlgo::Md5, false, false);
        assert_eq!(hash.len(), 16);

        // Same entry → same hash
        let entries2 = vec![make_entry("/tmp/test.jar", 0, 1024, 1000)];
        let (hash2, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries2, HashAlgo::Md5, false, false);
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
            ClasspathServiceImpl::hash_classpath_impl(&entries_a, HashAlgo::Md5, false, false);
        let (hash_b, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries_b, HashAlgo::Md5, false, false);
        assert_eq!(hash_a, hash_b);
    }

    #[test]
    fn test_different_mtime_different_hash() {
        let entries_a = vec![make_entry("/tmp/test.jar", 0, 1024, 1000)];
        let entries_b = vec![make_entry("/tmp/test.jar", 0, 1024, 2000)];
        let (hash_a, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries_a, HashAlgo::Md5, false, false);
        let (hash_b, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries_b, HashAlgo::Md5, false, false);
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
            HashAlgo::Md5,
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
            HashAlgo::Md5,
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
            ClasspathServiceImpl::hash_classpath_impl(&entries, HashAlgo::Md5, false, true);
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
            ClasspathServiceImpl::hash_classpath_impl(&entries, HashAlgo::Md5, false, false);

        let (changed, _, differences) =
            ClasspathServiceImpl::compare_classpaths_impl(&hash, &entries, HashAlgo::Md5);
        assert!(!changed);
        assert!(differences.is_empty());
    }

    #[test]
    fn test_compare_different_classpaths() {
        let entries_a = vec![make_entry("/tmp/a.jar", 0, 100, 1000)];
        let (hash_a, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries_a, HashAlgo::Md5, false, false);

        let entries_b = vec![
            make_entry("/tmp/a.jar", 0, 100, 1000),
            make_entry("/tmp/b.jar", 0, 200, 2000),
        ];

        let (changed, _, differences) =
            ClasspathServiceImpl::compare_classpaths_impl(&hash_a, &entries_b, HashAlgo::Md5);
        assert!(changed);
        assert_eq!(differences.len(), 2);
    }

    #[test]
    fn test_blake3_algorithm() {
        let entries = vec![make_entry("/tmp/test.jar", 0, 1024, 1000)];
        let (hash, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries, HashAlgo::Blake3, false, false);
        assert_eq!(hash.len(), 32); // BLAKE3 = 32 bytes
    }

    #[test]
    fn test_sha1_algorithm() {
        let entries = vec![make_entry("/tmp/test.jar", 0, 1024, 1000)];
        let (hash, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries, HashAlgo::Sha1, false, false);
        assert_eq!(hash.len(), 20); // SHA-1 = 20 bytes
    }

    #[test]
    fn test_algo_from_name() {
        assert_eq!(HashAlgo::from_name(""), HashAlgo::Md5);
        assert_eq!(HashAlgo::from_name("md5"), HashAlgo::Md5);
        assert_eq!(HashAlgo::from_name("SHA-256"), HashAlgo::Sha256);
        assert_eq!(HashAlgo::from_name("sha256"), HashAlgo::Sha256);
        assert_eq!(HashAlgo::from_name("blake3"), HashAlgo::Blake3);
        assert_eq!(HashAlgo::from_name("unknown"), HashAlgo::Md5); // fallback
    }

    #[test]
    fn test_parallel_hashing_consistent() {
        // 20 entries should trigger parallel path
        let entries: Vec<ClasspathEntry> = (0..20)
            .map(|i| make_entry(&format!("/tmp/lib{}.jar", i), 0, 100 + i, 1000 + i))
            .collect();

        let (hash1, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries, HashAlgo::Md5, false, false);
        let (hash2, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries, HashAlgo::Md5, false, false);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_content_hash_fallback_on_missing_file() {
        // File doesn't exist → falls back to metadata hash
        let entries = vec![make_entry("/nonexistent/path.jar", 0, 1024, 1000)];
        let (hash_ts, _) = ClasspathServiceImpl::hash_classpath_impl(
            &entries,
            HashAlgo::Md5,
            true, // ignore_timestamps → tries content, falls back
            false,
        );
        let (hash_meta, _) =
            ClasspathServiceImpl::hash_classpath_impl(&entries, HashAlgo::Md5, false, false);
        // Fallback should produce metadata-based hash
        assert_eq!(hash_ts, hash_meta);
    }
}
