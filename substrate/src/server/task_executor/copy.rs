use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use crate::server::task_executor::{TaskExecutor, TaskInput, TaskResult};

/// Copies files from source paths to a target directory.
pub struct CopyTaskExecutor;

impl Default for CopyTaskExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl CopyTaskExecutor {
    pub fn new() -> Self {
        Self
    }

    fn list_files(
        dir: &Path,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<PathBuf>, String>> + Send + '_>> {
        Box::pin(async move {
            let mut files = Vec::new();
            let mut entries = tokio::fs::read_dir(dir)
                .await
                .map_err(|e| format!("Failed to read directory {}: {}", dir.display(), e))?;
            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|e| format!("Failed to read directory {}: {}", dir.display(), e))?
            {
                let path = entry.path();
                if path.is_dir() {
                    files.extend(Self::list_files(&path).await?);
                } else if path.is_file() {
                    files.push(path);
                }
            }
            files.sort_unstable();
            Ok(files)
        })
    }

    fn list_dirs(
        dir: &Path,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<PathBuf>, String>> + Send + '_>> {
        Box::pin(async move {
            let mut dirs = Vec::new();
            let mut entries = tokio::fs::read_dir(dir)
                .await
                .map_err(|e| format!("Failed to read directory {}: {}", dir.display(), e))?;
            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|e| format!("Failed to read directory {}: {}", dir.display(), e))?
            {
                let path = entry.path();
                if path.is_dir() {
                    dirs.push(path.clone());
                    dirs.extend(Self::list_dirs(&path).await?);
                }
            }
            dirs.sort_unstable();
            Ok(dirs)
        })
    }

    async fn copy_file(
        src: &Path,
        dest: &Path,
        result: &mut TaskResult,
        expand_properties: &[(String, String)],
        file_mode: Option<u32>,
        dir_mode: Option<u32>,
    ) -> Result<(), String> {
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create directory {}: {}", parent.display(), e))?;
            apply_unix_mode(parent, dir_mode)?;
        }
        let bytes = if expand_properties.is_empty() {
            tokio::fs::copy(src, dest)
                .await
                .map_err(|e| format!("Failed to copy {}: {}", src.display(), e))?
        } else {
            let data = tokio::fs::read(src)
                .await
                .map_err(|e| format!("Failed to read {}: {}", src.display(), e))?;
            let expanded = expand_bytes(data, expand_properties);
            tokio::fs::write(dest, &expanded)
                .await
                .map_err(|e| format!("Failed to write {}: {}", dest.display(), e))?;
            expanded.len() as u64
        };
        apply_unix_mode(dest, file_mode)?;
        result.files_processed += 1;
        result.bytes_processed += bytes;
        result.output_files.push(dest.to_path_buf());
        Ok(())
    }
}

pub(super) fn parse_expand_properties(value: Option<&String>) -> Vec<(String, String)> {
    value
        .map(|properties| {
            properties
                .split(',')
                .filter_map(|entry| entry.split_once('='))
                .map(|(key, value)| (key.trim().to_string(), value.trim().to_string()))
                .filter(|(key, _)| !key.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn expand_bytes(data: Vec<u8>, properties: &[(String, String)]) -> Vec<u8> {
    let mut text = match String::from_utf8(data) {
        Ok(text) => text,
        Err(err) => return err.into_bytes(),
    };
    for (key, value) in properties {
        text = text.replace(&format!("${{{}}}", key), value);
        text = text.replace(&format!("${}", key), value);
    }
    text.into_bytes()
}

pub(super) fn duplicate_strategy(input: &TaskInput) -> String {
    input
        .options
        .get("duplicates_strategy")
        .map(|strategy| strategy.to_ascii_uppercase())
        .unwrap_or_else(|| "INCLUDE".to_string())
}

pub(super) fn parse_patterns(value: Option<&String>) -> Vec<String> {
    value
        .map(|patterns| {
            patterns
                .split(',')
                .map(str::trim)
                .filter(|pattern| !pattern.is_empty())
                .map(|pattern| pattern.replace('\\', "/"))
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn case_sensitive(input: &TaskInput) -> bool {
    input
        .options
        .get("case_sensitive")
        .map(|value| value != "false")
        .unwrap_or(true)
}

pub(super) fn include_empty_dirs(input: &TaskInput) -> bool {
    input
        .options
        .get("include_empty_dirs")
        .map(|value| value != "false")
        .unwrap_or(true)
}

pub(super) fn file_permission_mode(input: &TaskInput) -> Option<u32> {
    parse_unix_mode(input.options.get("file_permissions"))
}

pub(super) fn dir_permission_mode(input: &TaskInput) -> Option<u32> {
    parse_unix_mode(input.options.get("dir_permissions"))
}

pub(super) fn parse_unix_mode(value: Option<&String>) -> Option<u32> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    let parsed = if value.len() > 1 && value.starts_with('0') {
        u32::from_str_radix(value, 8).ok()?
    } else {
        value.parse::<u32>().ok()?
    };
    (parsed <= 0o7777).then_some(parsed)
}

pub(super) fn apply_unix_mode(path: &Path, mode: Option<u32>) -> Result<(), String> {
    let Some(mode) = mode else {
        return Ok(());
    };
    apply_unix_mode_inner(path, mode)
}

#[cfg(unix)]
fn apply_unix_mode_inner(path: &Path, mode: u32) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let permissions = std::fs::Permissions::from_mode(mode);
    std::fs::set_permissions(path, permissions)
        .map_err(|e| format!("Failed to set permissions on {}: {}", path.display(), e))
}

#[cfg(not(unix))]
fn apply_unix_mode_inner(_path: &Path, _mode: u32) -> Result<(), String> {
    Ok(())
}

pub(super) fn path_included(
    relative_path: &Path,
    include_patterns: &[String],
    exclude_patterns: &[String],
    case_sensitive: bool,
) -> bool {
    let normalized = relative_path.to_string_lossy().replace('\\', "/");
    let included = include_patterns.is_empty()
        || include_patterns
            .iter()
            .any(|pattern| glob_matches(pattern, &normalized, case_sensitive));
    included
        && !exclude_patterns
            .iter()
            .any(|pattern| glob_matches(pattern, &normalized, case_sensitive))
}

fn glob_matches(pattern: &str, path: &str, case_sensitive: bool) -> bool {
    let pattern = if case_sensitive {
        pattern.to_string()
    } else {
        pattern.to_ascii_lowercase()
    };
    let path = if case_sensitive {
        path.to_string()
    } else {
        path.to_ascii_lowercase()
    };

    if !pattern.contains('/') {
        let basename = path.rsplit('/').next().unwrap_or(&path);
        return segment_matches(&pattern, basename);
    }

    let pattern_parts: Vec<&str> = pattern.split('/').filter(|part| !part.is_empty()).collect();
    let path_parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    path_parts_match(&pattern_parts, &path_parts)
}

fn path_parts_match(pattern_parts: &[&str], path_parts: &[&str]) -> bool {
    match (pattern_parts.split_first(), path_parts.split_first()) {
        (None, None) => true,
        (None, Some(_)) => false,
        (Some((pattern, rest)), _) if *pattern == "**" => {
            path_parts_match(rest, path_parts)
                || path_parts
                    .split_first()
                    .map(|(_, remaining)| path_parts_match(pattern_parts, remaining))
                    .unwrap_or(false)
        }
        (Some((pattern, rest_patterns)), Some((path, rest_paths))) => {
            segment_matches(pattern, path) && path_parts_match(rest_patterns, rest_paths)
        }
        (Some(_), None) => false,
    }
}

fn segment_matches(pattern: &str, text: &str) -> bool {
    let pattern = pattern.as_bytes();
    let text = text.as_bytes();
    let (mut p, mut t) = (0usize, 0usize);
    let mut star = None;
    let mut match_after_star = 0usize;

    while t < text.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            match_after_star = t;
            p += 1;
        } else if let Some(star_pos) = star {
            p = star_pos + 1;
            match_after_star += 1;
            t = match_after_star;
        } else {
            return false;
        }
    }

    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}

#[tonic::async_trait]
impl TaskExecutor for CopyTaskExecutor {
    fn task_type(&self) -> &str {
        "Copy"
    }

    async fn execute(&self, input: &TaskInput) -> TaskResult {
        let start = std::time::Instant::now();
        let mut result = TaskResult::default();
        let file_mode = file_permission_mode(input);
        let dir_mode = dir_permission_mode(input);

        if !input.target_dir.exists() {
            if let Err(e) = tokio::fs::create_dir_all(&input.target_dir).await {
                result.success = false;
                result.error_message = format!("Failed to create target directory: {}", e);
                return result;
            }
        }
        if let Err(e) = apply_unix_mode(&input.target_dir, dir_mode) {
            result.success = false;
            result.error_message = e;
            return result;
        }

        let expand_properties = parse_expand_properties(input.options.get("expand_properties"));
        let duplicate_strategy = duplicate_strategy(input);
        let include_patterns = parse_patterns(input.options.get("include_patterns"));
        let exclude_patterns = parse_patterns(input.options.get("exclude_patterns"));
        let case_sensitive = case_sensitive(input);
        let include_empty_dirs = include_empty_dirs(input);
        let mut seen_destinations = HashSet::new();

        for source in &input.source_files {
            if !source.exists() {
                result.success = false;
                result.error_message = format!("Source file not found: {}", source.display());
                return result;
            }

            if source.is_dir() {
                if include_empty_dirs {
                    let dirs = match Self::list_dirs(source).await {
                        Ok(dirs) => dirs,
                        Err(e) => {
                            result.success = false;
                            result.error_message = e;
                            return result;
                        }
                    };
                    for dir in dirs {
                        let relative = dir.strip_prefix(source).unwrap_or(&dir);
                        if path_included(
                            relative,
                            &include_patterns,
                            &exclude_patterns,
                            case_sensitive,
                        ) {
                            let dest = input.target_dir.join(relative);
                            if let Err(e) = tokio::fs::create_dir_all(&dest).await {
                                result.success = false;
                                result.error_message =
                                    format!("Failed to create directory {}: {}", dest.display(), e);
                                return result;
                            }
                            if let Err(e) = apply_unix_mode(&dest, dir_mode) {
                                result.success = false;
                                result.error_message = e;
                                return result;
                            }
                        }
                    }
                }
                let files = match Self::list_files(source).await {
                    Ok(files) => files,
                    Err(e) => {
                        result.success = false;
                        result.error_message = e;
                        return result;
                    }
                };
                for file in files {
                    let relative = file.strip_prefix(source).unwrap_or(&file);
                    if !path_included(
                        relative,
                        &include_patterns,
                        &exclude_patterns,
                        case_sensitive,
                    ) {
                        continue;
                    }
                    let dest = input.target_dir.join(relative);
                    if !seen_destinations.insert(dest.clone()) {
                        match duplicate_strategy.as_str() {
                            "EXCLUDE" => continue,
                            "FAIL" => {
                                result.success = false;
                                result.error_message =
                                    format!("Duplicate copy destination: {}", dest.display());
                                return result;
                            }
                            _ => {}
                        }
                    }
                    if let Err(e) = Self::copy_file(
                        &file,
                        &dest,
                        &mut result,
                        &expand_properties,
                        file_mode,
                        dir_mode,
                    )
                    .await
                    {
                        result.success = false;
                        result.error_message = e;
                        return result;
                    }
                }
            } else {
                let relative = Path::new(source.file_name().unwrap_or_default());
                if !path_included(
                    relative,
                    &include_patterns,
                    &exclude_patterns,
                    case_sensitive,
                ) {
                    continue;
                }
                let dest = input.target_dir.join(relative);
                if !seen_destinations.insert(dest.clone()) {
                    match duplicate_strategy.as_str() {
                        "EXCLUDE" => continue,
                        "FAIL" => {
                            result.success = false;
                            result.error_message =
                                format!("Duplicate copy destination: {}", dest.display());
                            return result;
                        }
                        _ => {}
                    }
                }
                if let Err(e) = Self::copy_file(
                    source,
                    &dest,
                    &mut result,
                    &expand_properties,
                    file_mode,
                    dir_mode,
                )
                .await
                {
                    result.success = false;
                    result.error_message = e;
                    return result;
                }
            }
        }

        result.duration_ms = start.elapsed().as_millis() as u64;
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn test_copy_single_file() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        let dest_dir = tmp.path().join("dest");
        tokio::fs::create_dir_all(&src_dir).await.unwrap();

        let src_file = src_dir.join("test.txt");
        tokio::fs::write(&src_file, b"hello world").await.unwrap();

        let executor = CopyTaskExecutor::new();
        let mut input = TaskInput::new("Copy");
        input.source_files.push(src_file.clone());
        input.target_dir = dest_dir.clone();

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert_eq!(result.files_processed, 1);
        assert_eq!(result.output_files.len(), 1);

        let content = tokio::fs::read(&dest_dir.join("test.txt")).await.unwrap();
        assert_eq!(content, b"hello world");
    }

    #[tokio::test]
    async fn test_copy_multiple_files() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        let dest_dir = tmp.path().join("dest");
        tokio::fs::create_dir_all(&src_dir).await.unwrap();

        for name in &["a.txt", "b.txt", "c.txt"] {
            let path = src_dir.join(name);
            tokio::fs::write(&path, name.as_bytes()).await.unwrap();
        }

        let executor = CopyTaskExecutor::new();
        let mut input = TaskInput::new("Copy");
        input.source_files.push(src_dir.join("a.txt"));
        input.source_files.push(src_dir.join("b.txt"));
        input.source_files.push(src_dir.join("c.txt"));
        input.target_dir = dest_dir;

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert_eq!(result.files_processed, 3);
        assert_eq!(result.output_files.len(), 3);
    }

    #[tokio::test]
    async fn test_copy_missing_source() {
        let tmp = tempfile::tempdir().unwrap();
        let dest_dir = tmp.path().join("dest");

        let executor = CopyTaskExecutor::new();
        let mut input = TaskInput::new("Copy");
        input
            .source_files
            .push(PathBuf::from("/nonexistent/file.txt"));
        input.target_dir = dest_dir;

        let result = executor.execute(&input).await;
        assert!(!result.success);
        assert!(result.error_message.contains("not found"));
    }

    #[tokio::test]
    async fn test_copy_creates_target_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let src_file = tmp.path().join("test.txt");
        tokio::fs::write(&src_file, b"data").await.unwrap();

        let dest_dir = tmp.path().join("nested/deep/dest");

        let executor = CopyTaskExecutor::new();
        let mut input = TaskInput::new("Copy");
        input.source_files.push(src_file);
        input.target_dir = dest_dir.clone();

        let result = executor.execute(&input).await;
        assert!(result.success);
        assert!(dest_dir.exists());
    }

    #[tokio::test]
    async fn test_copy_directory_recursively() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        let dest_dir = tmp.path().join("dest");
        tokio::fs::create_dir_all(src_dir.join("nested"))
            .await
            .unwrap();
        tokio::fs::write(src_dir.join("root.txt"), b"root")
            .await
            .unwrap();
        tokio::fs::write(src_dir.join("nested/child.txt"), b"child")
            .await
            .unwrap();

        let executor = CopyTaskExecutor::new();
        let mut input = TaskInput::new("Copy");
        input.source_files.push(src_dir);
        input.target_dir = dest_dir.clone();

        let result = executor.execute(&input).await;
        assert!(result.success, "{}", result.error_message);
        assert_eq!(result.files_processed, 2);
        assert_eq!(
            tokio::fs::read(dest_dir.join("root.txt")).await.unwrap(),
            b"root"
        );
        assert_eq!(
            tokio::fs::read(dest_dir.join("nested/child.txt"))
                .await
                .unwrap(),
            b"child"
        );
    }

    #[tokio::test]
    async fn test_copy_expands_declared_properties() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        let dest_dir = tmp.path().join("dest");
        tokio::fs::create_dir_all(&src_dir).await.unwrap();
        tokio::fs::write(
            src_dir.join("application.properties"),
            b"name=$appName\nversion=${appVersion}\n",
        )
        .await
        .unwrap();

        let executor = CopyTaskExecutor::new();
        let mut input = TaskInput::new("Copy");
        input.source_files.push(src_dir);
        input.target_dir = dest_dir.clone();
        input.options.insert(
            "expand_properties".to_string(),
            "appName=corpus,appVersion=1.0".to_string(),
        );

        let result = executor.execute(&input).await;
        assert!(result.success, "{}", result.error_message);
        assert_eq!(
            tokio::fs::read_to_string(dest_dir.join("application.properties"))
                .await
                .unwrap(),
            "name=corpus\nversion=1.0\n"
        );
    }

    #[tokio::test]
    async fn test_copy_honors_include_and_exclude_patterns() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        let dest_dir = tmp.path().join("dest");
        tokio::fs::create_dir_all(src_dir.join("config"))
            .await
            .unwrap();
        tokio::fs::create_dir_all(src_dir.join("tmp"))
            .await
            .unwrap();
        tokio::fs::write(src_dir.join("config/app.properties"), b"app")
            .await
            .unwrap();
        tokio::fs::write(src_dir.join("config/app.txt"), b"text")
            .await
            .unwrap();
        tokio::fs::write(src_dir.join("tmp/skip.properties"), b"skip")
            .await
            .unwrap();

        let executor = CopyTaskExecutor::new();
        let mut input = TaskInput::new("Copy");
        input.source_files.push(src_dir);
        input.target_dir = dest_dir.clone();
        input.options.insert(
            "include_patterns".to_string(),
            "**/*.properties".to_string(),
        );
        input
            .options
            .insert("exclude_patterns".to_string(), "tmp/**".to_string());

        let result = executor.execute(&input).await;
        assert!(result.success, "{}", result.error_message);
        assert!(dest_dir.join("config/app.properties").exists());
        assert!(!dest_dir.join("config/app.txt").exists());
        assert!(!dest_dir.join("tmp/skip.properties").exists());
    }

    #[tokio::test]
    async fn test_copy_includes_empty_dirs_by_default() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        let dest_dir = tmp.path().join("dest");
        tokio::fs::create_dir_all(src_dir.join("empty/nested"))
            .await
            .unwrap();

        let executor = CopyTaskExecutor::new();
        let mut input = TaskInput::new("Copy");
        input.source_files.push(src_dir);
        input.target_dir = dest_dir.clone();

        let result = executor.execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        assert!(dest_dir.join("empty/nested").is_dir());
    }

    #[tokio::test]
    async fn test_copy_can_skip_empty_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        let dest_dir = tmp.path().join("dest");
        tokio::fs::create_dir_all(src_dir.join("empty"))
            .await
            .unwrap();

        let executor = CopyTaskExecutor::new();
        let mut input = TaskInput::new("Copy");
        input.source_files.push(src_dir);
        input.target_dir = dest_dir.clone();
        input
            .options
            .insert("include_empty_dirs".to_string(), "false".to_string());

        let result = executor.execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        assert!(!dest_dir.join("empty").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_copy_applies_declared_file_and_dir_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        let dest_dir = tmp.path().join("dest");
        tokio::fs::create_dir_all(src_dir.join("nested"))
            .await
            .unwrap();
        tokio::fs::write(src_dir.join("nested/app.sh"), b"echo hi")
            .await
            .unwrap();

        let executor = CopyTaskExecutor::new();
        let mut input = TaskInput::new("Copy");
        input.source_files.push(src_dir);
        input.target_dir = dest_dir.clone();
        input
            .options
            .insert("file_permissions".to_string(), "493".to_string());
        input
            .options
            .insert("dir_permissions".to_string(), "448".to_string());

        let result = executor.execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        assert_eq!(
            std::fs::metadata(dest_dir.join("nested/app.sh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        assert_eq!(
            std::fs::metadata(dest_dir.join("nested"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[test]
    fn test_copy_pattern_matching_case_insensitive_basename() {
        assert!(path_included(
            Path::new("nested/APP.PROPERTIES"),
            &["*.properties".to_string()],
            &[],
            false
        ));
        assert!(!path_included(
            Path::new("nested/APP.PROPERTIES"),
            &["*.properties".to_string()],
            &[],
            true
        ));
    }

    #[tokio::test]
    async fn test_copy_duplicate_strategy_exclude_keeps_first_file() {
        let tmp = tempfile::tempdir().unwrap();
        let src_a = tmp.path().join("src-a");
        let src_b = tmp.path().join("src-b");
        let dest_dir = tmp.path().join("dest");
        tokio::fs::create_dir_all(&src_a).await.unwrap();
        tokio::fs::create_dir_all(&src_b).await.unwrap();
        tokio::fs::write(src_a.join("same.txt"), b"first")
            .await
            .unwrap();
        tokio::fs::write(src_b.join("same.txt"), b"second")
            .await
            .unwrap();

        let executor = CopyTaskExecutor::new();
        let mut input = TaskInput::new("Copy");
        input.source_files.push(src_a);
        input.source_files.push(src_b);
        input.target_dir = dest_dir.clone();
        input
            .options
            .insert("duplicates_strategy".to_string(), "EXCLUDE".to_string());

        let result = executor.execute(&input).await;
        assert!(result.success, "{}", result.error_message);
        assert_eq!(
            tokio::fs::read(dest_dir.join("same.txt")).await.unwrap(),
            b"first"
        );
    }

    #[tokio::test]
    async fn test_copy_duplicate_strategy_fail_reports_duplicate() {
        let tmp = tempfile::tempdir().unwrap();
        let src_a = tmp.path().join("src-a");
        let src_b = tmp.path().join("src-b");
        let dest_dir = tmp.path().join("dest");
        tokio::fs::create_dir_all(&src_a).await.unwrap();
        tokio::fs::create_dir_all(&src_b).await.unwrap();
        tokio::fs::write(src_a.join("same.txt"), b"first")
            .await
            .unwrap();
        tokio::fs::write(src_b.join("same.txt"), b"second")
            .await
            .unwrap();

        let executor = CopyTaskExecutor::new();
        let mut input = TaskInput::new("Copy");
        input.source_files.push(src_a);
        input.source_files.push(src_b);
        input.target_dir = dest_dir;
        input
            .options
            .insert("duplicates_strategy".to_string(), "FAIL".to_string());

        let result = executor.execute(&input).await;
        assert!(!result.success);
        assert!(result.error_message.contains("Duplicate copy destination"));
    }
}
