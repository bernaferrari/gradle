use std::path::Path;

use md5::{Digest, Md5};
use tonic::{Request, Response, Status};

use crate::proto::{
    file_fingerprint_service_server::FileFingerprintService, FileFingerprintEntry,
    FingerprintFilesRequest, FingerprintFilesResponse, FingerprintType,
};

/// A single fingerprinted file entry: (relative_path, content_hash, size_bytes, modified_time_ms, is_directory).
type FingerprintEntry = (String, Vec<u8>, i64, i64, bool);

/// Rust-native file fingerprinting service.
/// Walks file trees and computes content hashes, replacing Java's FileCollectionFingerprinter.
#[derive(Default)]
pub struct FileFingerprintServiceImpl;

/// Normalization strategy for file paths in fingerprint computation.
#[derive(Debug, Clone, Copy, PartialEq)]
enum NormalizationStrategy {
    /// Use absolute paths (default).
    AbsolutePath,
    /// Use paths relative to the common root directory.
    RelativePath,
    /// Use only file names (ignore directory structure).
    NameOnly,
    /// Use only content hashes (ignore paths entirely).
    HashOnly,
}

impl NormalizationStrategy {
    fn from_str(s: &str) -> Self {
        match s {
            "RELATIVE_PATH" => Self::RelativePath,
            "NAME_ONLY" => Self::NameOnly,
            "HASH" => Self::HashOnly,
            _ => Self::AbsolutePath,
        }
    }

    /// Normalize a path according to the strategy.
    /// `base` is the root directory (used for RELATIVE_PATH).
    /// `relative` is the path relative to base.
    fn normalize<'a>(&self, _base: &Path, relative: &'a str, full_path: &Path) -> std::borrow::Cow<'a, str> {
        match self {
            Self::AbsolutePath => {
                // Use the full absolute path
                std::borrow::Cow::Owned(full_path.to_string_lossy().to_string())
            }
            Self::RelativePath => {
                // Already relative to base
                std::borrow::Cow::Borrowed(relative)
            }
            Self::NameOnly => {
                // Use only the file name
                std::borrow::Cow::Owned(
                    full_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(relative)
                        .to_string(),
                )
            }
            Self::HashOnly => {
                // Use "hash-" prefix + relative as a placeholder;
                // the actual entry path is replaced with the hash below
                std::borrow::Cow::Borrowed(relative)
            }
        }
    }

    /// Replace a path entry with a hash-based identifier for HashOnly strategy.
    fn hash_only_path(hash: &[u8]) -> String {
        format!("hash-{:x}", Md5::digest(hash))
    }
}

impl FileFingerprintServiceImpl {
    pub fn new() -> Self {
        Self
    }

    fn fingerprint_file(path: &Path) -> Result<(Vec<u8>, i64, i64), String> {
        let metadata = std::fs::metadata(path).map_err(|e| format!("{}: {}", path.display(), e))?;
        let size = metadata.len() as i64;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        // Compute MD5 hash of file content (matching Java's DefaultStreamHasher)
        let mut hasher = Md5::new();
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {}", path.display(), e))?;
        let mut reader = std::io::BufReader::new(file);
        let mut buffer = [0u8; 8192];
        loop {
            let n = std::io::Read::read(&mut reader, &mut buffer)
                .map_err(|e| format!("{}: {}", path.display(), e))?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        let hash = hasher.finalize().to_vec();

        Ok((hash, size, modified))
    }

    fn fingerprint_directory(
        dir: &Path,
        ignore_patterns: &[String],
        strategy: NormalizationStrategy,
    ) -> Result<(Vec<FingerprintEntry>, Vec<u8>), String> {
        let mut entries = Vec::new();
        let mut dir_hasher = Md5::new();

        Self::walk_dir(dir, dir, &mut entries, &mut dir_hasher, ignore_patterns, strategy)?;

        let collection_hash = dir_hasher.finalize().to_vec();
        Ok((entries, collection_hash))
    }

    fn should_ignore(path: &Path, ignore_patterns: &[String]) -> bool {
        let file_name = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        let path_str = path.to_string_lossy();

        for pattern in ignore_patterns {
            // Exact filename match
            if file_name == pattern {
                return true;
            }
            // *.ext pattern: match files ending with .ext
            if pattern.starts_with("*.") {
                let ext = &pattern[1..]; // e.g. ".class"
                if file_name.ends_with(ext) {
                    return true;
                }
            }
            // Directory/partial path match: if path contains the pattern as a path component
            if path_str.contains(&format!("/{}", pattern)) {
                return true;
            }
            // Endswith for directory patterns like "build"
            if path_str.ends_with(&format!("/{}", pattern)) {
                return true;
            }
        }
        false
    }

    fn walk_dir(
        base: &Path,
        current: &Path,
        entries: &mut Vec<FingerprintEntry>,
        hasher: &mut Md5,
        ignore_patterns: &[String],
        strategy: NormalizationStrategy,
    ) -> Result<(), String> {
        let dir_entries = std::fs::read_dir(current)
            .map_err(|e| format!("{}: {}", current.display(), e))?;

        let mut dir_entries: Vec<_> = dir_entries
            .filter_map(|e| e.ok())
            .collect();
        dir_entries.sort_by_key(|e| e.file_name());

        for entry in dir_entries {
            let path = entry.path();

            if Self::should_ignore(&path, ignore_patterns) {
                continue;
            }

            let relative = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();

            if path.is_dir() {
                Self::walk_dir(base, &path, entries, hasher, ignore_patterns, strategy)?;
            } else {
                if let Ok((hash, size, modified)) = Self::fingerprint_file(&path) {
                    let normalized = strategy.normalize(base, &relative, &path);

                    match strategy {
                        NormalizationStrategy::HashOnly => {
                            // Only hash content contributes; path is ignored
                            hasher.update(&hash);
                            hasher.update(b";");
                            entries.push((NormalizationStrategy::hash_only_path(&hash), hash, size, modified, false));
                        }
                        _ => {
                            hasher.update(normalized.as_bytes());
                            hasher.update(b"=");
                            hasher.update(&hash);
                            hasher.update(b";");
                            entries.push((normalized.into_owned(), hash, size, modified, false));
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

#[tonic::async_trait]
impl FileFingerprintService for FileFingerprintServiceImpl {
    async fn fingerprint_files(
        &self,
        request: Request<FingerprintFilesRequest>,
    ) -> Result<Response<FingerprintFilesResponse>, Status> {
        let req = request.into_inner();
        let mut all_entries = Vec::new();
        let mut collection_hasher = Md5::new();
        let strategy = NormalizationStrategy::from_str(&req.normalization_strategy);

        for file in &req.files {
            let path = Path::new(&file.absolute_path);

            if !path.exists() {
                continue;
            }

            let file_type = FingerprintType::try_from(file.r#type)
                .unwrap_or(FingerprintType::FingerprintFile);

            match file_type {
                FingerprintType::FingerprintDirectory | FingerprintType::FingerprintRoot => {
                    if path.is_dir() {
                        match Self::fingerprint_directory(path, &req.ignore_patterns, strategy) {
                            Ok((entries, dir_hash)) => {
                                for (entry_path, hash, _size, _modified, _is_dir) in &entries {
                                    match strategy {
                                        NormalizationStrategy::HashOnly => {
                                            collection_hasher.update(hash);
                                        }
                                        _ => {
                                            collection_hasher.update(entry_path.as_bytes());
                                            collection_hasher.update(hash);
                                        }
                                    }
                                }
                                for (entry_path, hash, size, modified, is_dir) in entries {
                                    all_entries.push(FileFingerprintEntry {
                                        path: entry_path,
                                        hash,
                                        size,
                                        last_modified: modified,
                                        is_directory: is_dir,
                                    });
                                }
                                collection_hasher.update(&dir_hash);
                            }
                            Err(e) => {
                                return Ok(Response::new(FingerprintFilesResponse {
                                    success: false,
                                    error_message: e,
                                    collection_hash: Vec::new(),
                                    entries: Vec::new(),
                                }));
                            }
                        }
                    }
                }
                FingerprintType::FingerprintFile => {
                    match Self::fingerprint_file(path) {
                        Ok((hash, size, modified)) => {
                            let display_path = match strategy {
                                NormalizationStrategy::RelativePath => file.absolute_path.clone(),
                                NormalizationStrategy::NameOnly => {
                                    path.file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or(&file.absolute_path)
                                        .to_string()
                                }
                                NormalizationStrategy::HashOnly => format!("hash-{:x}", Md5::digest(&hash)),
                                _ => file.absolute_path.clone(),
                            };
                            all_entries.push(FileFingerprintEntry {
                                path: display_path.clone(),
                                hash: hash.clone(),
                                size,
                                last_modified: modified,
                                is_directory: false,
                            });
                            match strategy {
                                NormalizationStrategy::HashOnly => {
                                    collection_hasher.update(&hash);
                                }
                                _ => {
                                    collection_hasher.update(display_path.as_bytes());
                                    collection_hasher.update(&hash);
                                }
                            }
                        }
                        Err(e) => {
                            return Ok(Response::new(FingerprintFilesResponse {
                                success: false,
                                error_message: e,
                                collection_hash: Vec::new(),
                                entries: Vec::new(),
                            }));
                        }
                    }
                }
            }
        }

        let collection_hash = collection_hasher.finalize().to_vec();

        tracing::debug!(
            files = all_entries.len(),
            strategy = ?strategy,
            "Fingerprinted files"
        );

        Ok(Response::new(FingerprintFilesResponse {
            success: true,
            error_message: String::new(),
            collection_hash,
            entries: all_entries,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_fingerprint_single_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello world").unwrap();

        let svc = FileFingerprintServiceImpl::new();
        let resp = svc
            .fingerprint_files(Request::new(FingerprintFilesRequest {
                files: vec![crate::proto::FileToFingerprint {
                    absolute_path: file_path.to_string_lossy().to_string(),
                    r#type: FingerprintType::FingerprintFile as i32,
                }],
                normalization_strategy: "ABSOLUTE_PATH".to_string(),
                ignore_patterns: Vec::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        assert_eq!(resp.entries.len(), 1);
        assert!(!resp.collection_hash.is_empty());
        assert_eq!(resp.entries[0].size, 11);
    }

    #[tokio::test]
    async fn test_fingerprint_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "aaa").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/b.txt"), "bbb").unwrap();

        let svc = FileFingerprintServiceImpl::new();
        let resp = svc
            .fingerprint_files(Request::new(FingerprintFilesRequest {
                files: vec![crate::proto::FileToFingerprint {
                    absolute_path: dir.path().to_string_lossy().to_string(),
                    r#type: FingerprintType::FingerprintDirectory as i32,
                }],
                normalization_strategy: "ABSOLUTE_PATH".to_string(),
                ignore_patterns: Vec::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        assert_eq!(resp.entries.len(), 2);
        assert!(!resp.collection_hash.is_empty());
    }

    #[tokio::test]
    async fn test_fingerprint_missing_file() {
        let svc = FileFingerprintServiceImpl::new();
        let resp = svc
            .fingerprint_files(Request::new(FingerprintFilesRequest {
                files: vec![crate::proto::FileToFingerprint {
                    absolute_path: "/nonexistent/path.txt".to_string(),
                    r#type: FingerprintType::FingerprintFile as i32,
                }],
                normalization_strategy: "ABSOLUTE_PATH".to_string(),
                ignore_patterns: Vec::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        assert_eq!(resp.entries.len(), 0);
    }

    #[test]
    fn test_hash_known_content() {
        // Verify that file hashing produces the standard MD5 of file content
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("known.txt");
        std::fs::write(&file_path, "test content").unwrap();

        let (hash, size, _) = FileFingerprintServiceImpl::fingerprint_file(&file_path).unwrap();

        // Standard MD5 of "test content" = 9473fdd0d880a43c21b7778d34872157
        let expected: [u8; 16] = Md5::digest(b"test content").into();
        assert_eq!(hash, expected.to_vec());
        assert_eq!(size, 12);
    }

    #[tokio::test]
    async fn test_fingerprint_with_ignore_patterns() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "aaa").unwrap();
        std::fs::write(dir.path().join("a.class"), "compiled").unwrap();
        std::fs::write(dir.path().join("b.txt"), "bbb").unwrap();
        std::fs::create_dir(dir.path().join("build")).unwrap();
        std::fs::write(dir.path().join("build/output.class"), "compiled").unwrap();

        let svc = FileFingerprintServiceImpl::new();
        let resp = svc
            .fingerprint_files(Request::new(FingerprintFilesRequest {
                files: vec![crate::proto::FileToFingerprint {
                    absolute_path: dir.path().to_string_lossy().to_string(),
                    r#type: FingerprintType::FingerprintDirectory as i32,
                }],
                normalization_strategy: "ABSOLUTE_PATH".to_string(),
                ignore_patterns: vec!["*.class".to_string(), "build".to_string()],
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        // Should only include a.txt and b.txt (class files and build/ ignored)
        assert_eq!(resp.entries.len(), 2);
    }

    #[test]
    fn test_should_ignore() {
        let path = Path::new("/some/path/build/output.class");
        assert!(FileFingerprintServiceImpl::should_ignore(
            path,
            &["*.class".to_string(), "build".to_string()],
        ));

        let path2 = Path::new("/some/path/src/Main.java");
        assert!(!FileFingerprintServiceImpl::should_ignore(
            path2,
            &["*.class".to_string(), "build".to_string()],
        ));
    }

    #[tokio::test]
    async fn test_fingerprint_name_only_strategy() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/a.txt"), "hello").unwrap();
        std::fs::write(dir.path().join("sub/b.txt"), "world").unwrap();

        let svc = FileFingerprintServiceImpl::new();
        let resp = svc
            .fingerprint_files(Request::new(FingerprintFilesRequest {
                files: vec![crate::proto::FileToFingerprint {
                    absolute_path: dir.path().to_string_lossy().to_string(),
                    r#type: FingerprintType::FingerprintDirectory as i32,
                }],
                normalization_strategy: "NAME_ONLY".to_string(),
                ignore_patterns: Vec::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        assert_eq!(resp.entries.len(), 2);
        // Paths should be just filenames
        for entry in &resp.entries {
            assert!(!entry.path.contains('/'), "Expected name-only, got: {}", entry.path);
            assert!(
                entry.path == "a.txt" || entry.path == "b.txt",
                "Expected a.txt or b.txt, got: {}",
                entry.path
            );
        }
    }

    #[tokio::test]
    async fn test_fingerprint_hash_only_strategy() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.txt"), "content").unwrap();

        let svc = FileFingerprintServiceImpl::new();
        let resp = svc
            .fingerprint_files(Request::new(FingerprintFilesRequest {
                files: vec![crate::proto::FileToFingerprint {
                    absolute_path: dir.path().to_string_lossy().to_string(),
                    r#type: FingerprintType::FingerprintDirectory as i32,
                }],
                normalization_strategy: "HASH".to_string(),
                ignore_patterns: Vec::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        assert_eq!(resp.entries.len(), 1);
        // Path should start with "hash-"
        assert!(resp.entries[0].path.starts_with("hash-"), "Expected hash- prefix, got: {}", resp.entries[0].path);
    }

    #[tokio::test]
    async fn test_fingerprint_relative_path_strategy() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.txt"), "data").unwrap();

        let svc = FileFingerprintServiceImpl::new();
        let resp = svc
            .fingerprint_files(Request::new(FingerprintFilesRequest {
                files: vec![crate::proto::FileToFingerprint {
                    absolute_path: dir.path().to_string_lossy().to_string(),
                    r#type: FingerprintType::FingerprintDirectory as i32,
                }],
                normalization_strategy: "RELATIVE_PATH".to_string(),
                ignore_patterns: Vec::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        assert_eq!(resp.entries.len(), 1);
        // Relative path should be just "test.txt" (no absolute path prefix)
        assert_eq!(resp.entries[0].path, "test.txt");
    }

    #[tokio::test]
    async fn test_same_content_different_paths_hash_only() {
        // Two directories with same file content but different paths should have same hash
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        std::fs::write(dir1.path().join("same.txt"), "identical content").unwrap();
        std::fs::write(dir2.path().join("same.txt"), "identical content").unwrap();

        let svc = FileFingerprintServiceImpl::new();

        let resp1 = svc
            .fingerprint_files(Request::new(FingerprintFilesRequest {
                files: vec![crate::proto::FileToFingerprint {
                    absolute_path: dir1.path().to_string_lossy().to_string(),
                    r#type: FingerprintType::FingerprintDirectory as i32,
                }],
                normalization_strategy: "HASH".to_string(),
                ignore_patterns: Vec::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        let resp2 = svc
            .fingerprint_files(Request::new(FingerprintFilesRequest {
                files: vec![crate::proto::FileToFingerprint {
                    absolute_path: dir2.path().to_string_lossy().to_string(),
                    r#type: FingerprintType::FingerprintDirectory as i32,
                }],
                normalization_strategy: "HASH".to_string(),
                ignore_patterns: Vec::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp1.collection_hash, resp2.collection_hash,
            "HASH strategy should produce same collection hash for same content regardless of path");
    }

    #[test]
    fn test_normalization_strategy_from_str() {
        assert_eq!(NormalizationStrategy::from_str("ABSOLUTE_PATH"), NormalizationStrategy::AbsolutePath);
        assert_eq!(NormalizationStrategy::from_str("RELATIVE_PATH"), NormalizationStrategy::RelativePath);
        assert_eq!(NormalizationStrategy::from_str("NAME_ONLY"), NormalizationStrategy::NameOnly);
        assert_eq!(NormalizationStrategy::from_str("HASH"), NormalizationStrategy::HashOnly);
        assert_eq!(NormalizationStrategy::from_str("unknown"), NormalizationStrategy::AbsolutePath);
        assert_eq!(NormalizationStrategy::from_str(""), NormalizationStrategy::AbsolutePath);
    }
}
