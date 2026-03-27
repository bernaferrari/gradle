use std::collections::HashSet;
use std::path::{Path, PathBuf};

use tonic::{Request, Response, Status};

use crate::proto::{
    file_tree_service_server::FileTreeService, FileTreeEntry, MatchPatternsRequest,
    MatchPatternsResponse, PatternMatchResult, TraverseFileTreeRequest, TraverseFileTreeResponse,
};

#[derive(Default)]
pub struct FileTreeServiceImpl;

impl FileTreeServiceImpl {
    pub const fn new() -> Self {
        Self
    }
}

/// Gradle/Ant default excludes (from Apache Ant DirectoryScanner).
const DEFAULT_EXCLUDES: &[&str] = &[
    "**/*~",
    "**/#*#",
    "**/.#*",
    "**/%*%",
    "**/._*",
    "**/CVS",
    "**/CVS/**",
    "**/.git",
    "**/.git/**",
    "**/.svn",
    "**/.svn/**",
    "**/.hg",
    "**/.hg/**",
    "**/.DS_Store",
    "**/Thumbs.db",
];

/// Check if an Ant-style pattern matches a path.
///
/// Supports:
/// - `*` — matches any number of characters (not path separator)
/// - `?` — matches exactly one character (not path separator)
/// - `**` — matches any number of directories or files
///
/// Check if an path matches an glob pattern. Supports `*`, `?`,, **` — matches any number of directories or files
pub(crate) fn ant_match(path: &str, pattern: &str) -> bool {
    match_segments(pattern, 0, path, 0)
}

/// Recursive segment matching with `**` support.
/// Works directly on string slices without allocation.
fn match_segments(pat: &str, pi: usize, path: &str, si: usize) -> bool {
    let pat_rest = &pat[pi..];
    let path_rest = &path[si..];

    if pat_rest.is_empty() {
        return path_rest.is_empty();
    }

    // Extract next pattern segment (up to '/' or end)
    let (pat_seg, next_pi) = match pat_rest.find('/') {
        Some(pos) => (&pat_rest[..pos], pi + pos + 1),
        None => (pat_rest, pat.len()),
    };

    if pat_seg == "**" {
        // `**` can match zero or more path segments
        if match_segments(pat, next_pi, path, si) {
            return true;
        }
        // Try consuming one path segment at a time
        let mut offset = 0;
        loop {
            match path_rest[offset..].find('/') {
                Some(pos) => {
                    offset += pos + 1;
                    if match_segments(pat, next_pi, path, si + offset) {
                        return true;
                    }
                }
                None => {
                    // Try matching remaining path as last segment
                    return match_segments(pat, next_pi, path, path.len());
                }
            }
        }
    }

    // Extract next path segment
    let (path_seg, next_si) = match path_rest.find('/') {
        Some(pos) => (&path_rest[..pos], si + pos + 1),
        None => {
            if path_rest.is_empty() {
                return false;
            }
            (path_rest, path.len())
        }
    };

    if glob_match(pat_seg, path_seg) {
        return match_segments(pat, next_pi, path, next_si);
    }

    false
}

/// Match a single path segment against a glob pattern.
/// `*` matches any chars except `/`, `?` matches exactly one char.
/// Non-allocating byte-level matching.
fn glob_match(pattern: &str, text: &str) -> bool {
    let p = pattern.as_bytes();
    let t = text.as_bytes();
    glob_match_impl(p, 0, t, 0)
}

#[inline]
fn glob_match_impl(p: &[u8], pi: usize, t: &[u8], ti: usize) -> bool {
    let mut pi = pi;
    let mut ti = ti;

    loop {
        if pi == p.len() {
            return ti == t.len();
        }

        match p[pi] {
            b'*' => {
                // Skip consecutive stars
                pi += 1;
                while pi < p.len() && p[pi] == b'*' {
                    pi += 1;
                }
                // `*` matches zero or more bytes
                for i in ti..=t.len() {
                    if glob_match_impl(p, pi, t, i) {
                        return true;
                    }
                }
                return false;
            }
            b'?' => {
                if ti >= t.len() {
                    return false;
                }
                pi += 1;
                ti += 1;
            }
            c => {
                if ti >= t.len() || t[ti] != c {
                    return false;
                }
                pi += 1;
                ti += 1;
            }
        }
    }
}

/// Check if a path matches any of the given patterns.
fn matches_any_pattern(path: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|p| ant_match(path, p))
}

/// Check if a path matches any default exclude pattern.
fn matches_default_exclude(path: &str) -> bool {
    DEFAULT_EXCLUDES.iter().any(|p| ant_match(path, p))
}

impl FileTreeServiceImpl {
    fn traverse_impl(req: &TraverseFileTreeRequest) -> (Vec<FileTreeEntry>, i64, Option<String>) {
        let root = Path::new(&req.root_dir);
        if !root.exists() {
            return (
                vec![],
                0,
                Some(format!("root directory does not exist: {}", req.root_dir)),
            );
        }
        if !root.is_dir() {
            return (
                vec![],
                0,
                Some(format!("path is not a directory: {}", req.root_dir)),
            );
        }

        let include_files = if req.include_files { req.include_files } else { true };
        let include_dirs = req.include_dirs;
        let follow_symlinks = req.follow_symlinks;
        let max_depth = if req.max_depth > 0 {
            req.max_depth as usize
        } else {
            usize::MAX
        };
        let include_metadata = req.include_metadata;
        let apply_default = req.apply_default_excludes;

        let exclude_patterns = &req.exclude_patterns;

        let include_patterns = &req.include_patterns;

        let mut entries = Vec::new();
        let mut total_size: i64 = 0;
        let mut visited = HashSet::new();

        Self::walk_dir(
            root,
            root,
            0,
            max_depth,
            include_files,
            include_dirs,
            follow_symlinks,
            include_metadata,
            apply_default,
            include_patterns,
            exclude_patterns,
            &mut entries,
            &mut total_size,
            &mut visited,
        );

        (entries, total_size, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn walk_dir(
        root: &Path,
        current: &Path,
        depth: usize,
        max_depth: usize,
        include_files: bool,
        include_dirs: bool,
        follow_symlinks: bool,
        include_metadata: bool,
        apply_default: bool,
        include_patterns: &[String],
        exclude_patterns: &[String],
        entries: &mut Vec<FileTreeEntry>,
        total_size: &mut i64,
        visited: &mut HashSet<PathBuf>,
    ) {
        if depth > max_depth {
            return;
        }

        let canonical = if follow_symlinks {
            current.canonicalize().unwrap_or_else(|_| current.to_path_buf())
        } else {
            current.to_path_buf()
        };

        if !visited.insert(canonical) {
            return; // cycle detection
        }

        let dir_entries = match std::fs::read_dir(current) {
            Ok(entries) => entries,
            Err(_) => return,
        };

        let mut dir_entries: Vec<_> = dir_entries.filter_map(|e| e.ok()).collect();
        dir_entries.sort_unstable_by_key(|e| e.file_name());

        for entry in dir_entries {
            let path = entry.path();
            let is_symlink = path.is_symlink();

            let metadata = if follow_symlinks {
                path.metadata().ok()
            } else {
                path.symlink_metadata().ok()
            };

            let metadata = match metadata {
                Some(m) => m,
                None => continue,
            };

            let is_dir = metadata.is_dir() && (!is_symlink || follow_symlinks);
            let is_file = metadata.is_file() || (!is_dir && is_symlink && !follow_symlinks);

            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();

            // Convert backslash to forward slash for pattern matching
            let relative_normalized = relative.replace('\\', "/");

            // Check exclude patterns
            if (apply_default && matches_default_exclude(&relative_normalized))
                || (!exclude_patterns.is_empty() && matches_any_pattern(&relative_normalized, exclude_patterns))
            {
                continue;
            }

            if is_dir {
                // Check include patterns for directories (optional)
                if include_dirs
                    && (include_patterns.is_empty()
                        || matches_any_pattern(&relative_normalized, include_patterns))
                {
                    entries.push(FileTreeEntry {
                        relative_path: relative_normalized,
                        absolute_path: path.to_string_lossy().to_string(),
                        is_directory: true,
                        size: if include_metadata {
                            metadata.len() as i64
                        } else {
                            0
                        },
                        last_modified_ms: if include_metadata {
                            metadata
                                .modified()
                                .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64)
                                .unwrap_or(0)
                        } else {
                            0
                        },
                    });
                }
                Self::walk_dir(
                    root,
                    &path,
                    depth + 1,
                    max_depth,
                    include_files,
                    include_dirs,
                    follow_symlinks,
                    include_metadata,
                    apply_default,
                    include_patterns,
                    exclude_patterns,
                    entries,
                    total_size,
                    visited,
                );
            } else if is_file && include_files {
                // Check include patterns for files
                if include_patterns.is_empty()
                    || matches_any_pattern(&relative_normalized, include_patterns)
                {
                    if include_metadata {
                        *total_size += metadata.len() as i64;
                    }
                    entries.push(FileTreeEntry {
                        relative_path: relative_normalized,
                        absolute_path: path.to_string_lossy().to_string(),
                        is_directory: false,
                        size: if include_metadata {
                            metadata.len() as i64
                        } else {
                            0
                        },
                        last_modified_ms: if include_metadata {
                            metadata
                                .modified()
                                .map(|t| {
                                    t.duration_since(std::time::UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_millis() as i64
                                })
                                .unwrap_or(0)
                        } else {
                            0
                        },
                    });
                }
            }
        }
    }

    fn match_patterns_impl(
        paths: &[String],
        include_patterns: &[String],
        exclude_patterns: &[String],
    ) -> Vec<PatternMatchResult> {
        paths
            .iter()
            .map(|path| {
                let normalized = path.replace('\\', "/");
                // Included if: passes include filter AND passes exclude filter
                let passes_include = include_patterns.is_empty()
                    || matches_any_pattern(&normalized, include_patterns);
                let passes_exclude =
                    exclude_patterns.is_empty() || !matches_any_pattern(&normalized, exclude_patterns);
                PatternMatchResult {
                    path: path.clone(),
                    included: passes_include && passes_exclude,
                }
            })
            .collect()
    }
}

#[tonic::async_trait]
impl FileTreeService for FileTreeServiceImpl {
    async fn traverse_file_tree(
        &self,
        request: Request<TraverseFileTreeRequest>,
    ) -> Result<Response<TraverseFileTreeResponse>, Status> {
        let req = request.into_inner();
        let (entries, total_size, error_message) = Self::traverse_impl(&req);
        let total_entries = entries.len() as i32;

        tracing::debug!(
            root = ?req.root_dir,
            total_entries,
            total_size,
            "File tree traversal complete"
        );

        Ok(Response::new(TraverseFileTreeResponse {
            entries,
            total_entries,
            total_size,
            error_message: error_message.unwrap_or_default(),
        }))
    }

    async fn match_patterns(
        &self,
        request: Request<MatchPatternsRequest>,
    ) -> Result<Response<MatchPatternsResponse>, Status> {
        let req = request.into_inner();
        let results = Self::match_patterns_impl(&req.paths, &req.include_patterns, &req.exclude_patterns);
        Ok(Response::new(MatchPatternsResponse { results }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn make_traverse_request(root: &str) -> TraverseFileTreeRequest {
        TraverseFileTreeRequest {
            root_dir: root.to_string(),
            include_patterns: vec![],
            exclude_patterns: vec![],
            include_files: true,
            include_dirs: false,
            follow_symlinks: false,
            max_depth: 0,
            include_metadata: true,
            apply_default_excludes: false,
        }
    }

    #[test]
    fn test_glob_match_star() {
        assert!(glob_match("*.java", "Foo.java"));
        assert!(glob_match("*.java", "Bar.java"));
        assert!(!glob_match("*.java", "Foo.kt"));
        assert!(glob_match("*Test.java", "FooTest.java"));
        assert!(!glob_match("*Test.java", "FooTest.kt"));
    }

    #[test]
    fn test_glob_match_question() {
        assert!(glob_match("?.java", "A.java"));
        assert!(!glob_match("?.java", "Ab.java"));
        assert!(glob_match("???.java", "Foo.java"));
    }

    #[test]
    fn test_glob_match_star_in_middle() {
        assert!(glob_match("Foo*Bar.java", "FooBar.java"));
        assert!(glob_match("Foo*Bar.java", "FooBazBar.java"));
    }

    #[test]
    fn test_ant_match_double_star() {
        assert!(ant_match("src/main/java/Foo.java", "**/*.java"));
        assert!(ant_match("a/b/c/d/Foo.java", "**/*.java"));
        // ** matches zero or more segments, so **/*.java matches Foo.java
        assert!(ant_match("Foo.java", "**/*.java"));
    }

    #[test]
    fn test_ant_match_exact() {
        assert!(ant_match("build.gradle", "build.gradle"));
        assert!(!ant_match("build.gradle.kts", "build.gradle"));
    }

    #[test]
    fn test_ant_match_double_star_deep() {
        assert!(ant_match("src/test/java/com/example/Test.java", "**/test/**/*.java"));
        assert!(ant_match("test/Foo.java", "**/test/**/*.java"));
        assert!(!ant_match("src/main/java/com/example/Test.java", "**/test/**/*.java"));
    }

    #[test]
    fn test_ant_match_double_star_prefix() {
        assert!(ant_match("com/example/Foo.java", "com/**"));
        assert!(ant_match("com/a/b/c/Foo.java", "com/**"));
        assert!(!ant_match("org/example/Foo.java", "com/**"));
    }

    #[test]
    fn test_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let req = make_traverse_request(tmp.path().to_str().unwrap());
        let (entries, total_size, err) = FileTreeServiceImpl::traverse_impl(&req);
        assert!(entries.is_empty());
        assert_eq!(total_size, 0);
        assert!(err.is_none());
    }

    #[test]
    fn test_traverse_with_java_pattern() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("src/main/java/com/example")).unwrap();
        fs::write(tmp.path().join("src/main/java/com/example/Foo.java"), "class Foo {}")
            .unwrap();
        fs::write(tmp.path().join("src/main/java/com/example/Bar.kt"), "class Bar")
            .unwrap();

        let mut req = make_traverse_request(tmp.path().to_str().unwrap());
        req.include_patterns = vec!["**/*.java".to_string()];
        let (entries, _, err) = FileTreeServiceImpl::traverse_impl(&req);
        assert!(err.is_none());
        assert_eq!(entries.len(), 1);
        assert!(entries[0].relative_path.contains("Foo.java"));
        assert!(!entries[0].is_directory);
    }

    #[test]
    fn test_traverse_with_exclude() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("src/test")).unwrap();
        fs::create_dir_all(tmp.path().join("src/main")).unwrap();
        fs::write(tmp.path().join("src/test/Foo.java"), "test").unwrap();
        fs::write(tmp.path().join("src/main/Bar.java"), "main").unwrap();

        let mut req = make_traverse_request(tmp.path().to_str().unwrap());
        req.include_patterns = vec!["**/*.java".to_string()];
        req.exclude_patterns = vec!["**/test/**".to_string()];
        let (entries, _, err) = FileTreeServiceImpl::traverse_impl(&req);
        assert!(err.is_none());
        assert_eq!(entries.len(), 1);
        assert!(entries[0].relative_path.contains("Bar.java"));
    }

    #[test]
    fn test_default_excludes() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".git/objects")).unwrap();
        fs::write(tmp.path().join(".git/HEAD"), "ref: main").unwrap();
        fs::write(tmp.path().join("Foo.java"), "class Foo {}").unwrap();

        let mut req = make_traverse_request(tmp.path().to_str().unwrap());
        req.apply_default_excludes = true;
        let (entries, _, err) = FileTreeServiceImpl::traverse_impl(&req);
        assert!(err.is_none());
        assert_eq!(entries.len(), 1);
        assert!(entries[0].relative_path.contains("Foo.java"));
    }

    #[test]
    fn test_max_depth() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("a/b/c")).unwrap();
        fs::write(tmp.path().join("a/f1.txt"), "1").unwrap();
        fs::write(tmp.path().join("a/b/f2.txt"), "2").unwrap();
        fs::write(tmp.path().join("a/b/c/f3.txt"), "3").unwrap();

        let mut req = make_traverse_request(tmp.path().to_str().unwrap());
        req.max_depth = 1; // only files in the root directory
        let (entries, _, err) = FileTreeServiceImpl::traverse_impl(&req);
        assert!(err.is_none());
        // f1.txt is at depth 1 (a/), f2.txt at depth 2 (a/b/), f3.txt at depth 3 (a/b/c/)
        // With max_depth=1, only f1.txt should be found
        assert_eq!(entries.len(), 1);
        assert!(entries[0].relative_path.contains("f1.txt"));
    }

    #[test]
    fn test_include_dirs() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("src/main")).unwrap();

        let mut req = make_traverse_request(tmp.path().to_str().unwrap());
        req.include_files = false;
        req.include_dirs = true;
        let (entries, _, err) = FileTreeServiceImpl::traverse_impl(&req);
        assert!(err.is_none());
        assert!(!entries.is_empty());
        assert!(entries.iter().any(|e| e.is_directory));
    }

    #[test]
    fn test_metadata_included() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("test.txt"), "hello world").unwrap();

        let req = make_traverse_request(tmp.path().to_str().unwrap());
        let (entries, total_size, err) = FileTreeServiceImpl::traverse_impl(&req);
        assert!(err.is_none());
        assert_eq!(entries.len(), 1);
        assert!(entries[0].size > 0);
        assert!(entries[0].last_modified_ms > 0);
        assert!(total_size > 0);
    }

    #[test]
    fn test_metadata_not_included() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("test.txt"), "hello world").unwrap();

        let mut req = make_traverse_request(tmp.path().to_str().unwrap());
        req.include_metadata = false;
        let (entries, total_size, err) = FileTreeServiceImpl::traverse_impl(&req);
        assert!(err.is_none());
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].size, 0);
        assert_eq!(entries[0].last_modified_ms, 0);
        assert_eq!(total_size, 0);
    }

    #[test]
    fn test_match_patterns_include_only() {
        let paths = vec![
            "src/main/java/Foo.java".to_string(),
            "src/main/java/Bar.kt".to_string(),
            "build.gradle".to_string(),
        ];
        let results = FileTreeServiceImpl::match_patterns_impl(
            &paths,
            &["**/*.java".to_string()],
            &[],
        );
        assert_eq!(results.len(), 3);
        assert!(results[0].included);
        assert!(!results[1].included);
        assert!(!results[2].included);
    }

    #[test]
    fn test_match_patterns_include_and_exclude() {
        let paths = vec![
            "src/main/java/Foo.java".to_string(),
            "src/test/java/FooTest.java".to_string(),
        ];
        let results = FileTreeServiceImpl::match_patterns_impl(
            &paths,
            &["**/*.java".to_string()],
            &["**/test/**".to_string()],
        );
        assert!(results[0].included);
        assert!(!results[1].included);
    }

    #[test]
    fn test_nonexistent_root() {
        let req = make_traverse_request("/nonexistent/path/12345");
        let (entries, _, err) = FileTreeServiceImpl::traverse_impl(&req);
        assert!(entries.is_empty());
        assert!(err.is_some());
    }

    #[test]
    fn test_sorted_deterministic_order() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("z.txt"), "z").unwrap();
        fs::write(tmp.path().join("a.txt"), "a").unwrap();
        fs::write(tmp.path().join("m.txt"), "m").unwrap();

        let req = make_traverse_request(tmp.path().to_str().unwrap());
        let (entries, _, err) = FileTreeServiceImpl::traverse_impl(&req);
        assert!(err.is_none());
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].relative_path, "a.txt");
        assert_eq!(entries[1].relative_path, "m.txt");
        assert_eq!(entries[2].relative_path, "z.txt");
    }
}
