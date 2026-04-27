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

    async fn copy_file(
        src: &Path,
        dest: &Path,
        result: &mut TaskResult,
        expand_properties: &[(String, String)],
    ) -> Result<(), String> {
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("Failed to create directory {}: {}", parent.display(), e))?;
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

#[tonic::async_trait]
impl TaskExecutor for CopyTaskExecutor {
    fn task_type(&self) -> &str {
        "Copy"
    }

    async fn execute(&self, input: &TaskInput) -> TaskResult {
        let start = std::time::Instant::now();
        let mut result = TaskResult::default();

        if !input.target_dir.exists() {
            if let Err(e) = tokio::fs::create_dir_all(&input.target_dir).await {
                result.success = false;
                result.error_message = format!("Failed to create target directory: {}", e);
                return result;
            }
        }

        let expand_properties = parse_expand_properties(input.options.get("expand_properties"));
        let duplicate_strategy = duplicate_strategy(input);
        let mut seen_destinations = HashSet::new();

        for source in &input.source_files {
            if !source.exists() {
                result.success = false;
                result.error_message = format!("Source file not found: {}", source.display());
                return result;
            }

            if source.is_dir() {
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
                    if let Err(e) =
                        Self::copy_file(&file, &dest, &mut result, &expand_properties).await
                    {
                        result.success = false;
                        result.error_message = e;
                        return result;
                    }
                }
            } else {
                let dest = input
                    .target_dir
                    .join(source.file_name().unwrap_or_default());
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
                if let Err(e) =
                    Self::copy_file(source, &dest, &mut result, &expand_properties).await
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
