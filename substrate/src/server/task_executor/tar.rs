use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use crate::server::task_executor::{TaskExecutor, TaskInput, TaskResult};

/// Native Rust TAR packaging executor.
///
/// This is deliberately separate from the ZIP/JAR executor: Gradle's `Tar`
/// task emits POSIX tar streams, optionally gzip-compressed, not ZIP archives.
pub struct TarTaskExecutor;

impl Default for TarTaskExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl TarTaskExecutor {
    pub fn new() -> Self {
        Self
    }
}

#[tonic::async_trait]
impl TaskExecutor for TarTaskExecutor {
    fn task_type(&self) -> &str {
        "Tar"
    }

    async fn execute(&self, input: &TaskInput) -> TaskResult {
        let start = std::time::Instant::now();
        let mut result = TaskResult::default();
        let tar_path = input.target_dir.join(archive_name(input));

        if let Some(parent) = tar_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                result.success = false;
                result.error_message = format!("Failed to create target directory: {}", e);
                return result;
            }
        }

        let entries = match collect_entries(&input.source_files) {
            Ok(entries) => entries,
            Err(e) => {
                result.success = false;
                result.error_message = e;
                return result;
            }
        };
        let files_processed = entries.iter().filter(|entry| !entry.is_dir).count() as u64;
        let bytes_processed = entries.iter().map(|entry| entry.data.len() as u64).sum();

        if let Err(e) = write_archive(&tar_path, &entries, gzip_enabled(input, &tar_path)) {
            result.success = false;
            result.error_message = e;
            return result;
        }

        result.output_files.push(tar_path);
        result.files_processed = files_processed;
        result.bytes_processed = bytes_processed;
        result.duration_ms = start.elapsed().as_millis() as u64;
        result
    }
}

#[derive(Debug, Eq, PartialEq)]
struct TarEntry {
    name: String,
    data: Vec<u8>,
    is_dir: bool,
}

fn archive_name(input: &TaskInput) -> &str {
    input
        .options
        .get("tarName")
        .or_else(|| input.options.get("archiveName"))
        .or_else(|| input.options.get("jarName"))
        .or_else(|| input.options.get("archive_file_name"))
        .map(String::as_str)
        .unwrap_or("output.tar")
}

fn gzip_enabled(input: &TaskInput, archive_path: &Path) -> bool {
    let compression = input
        .options
        .get("compression")
        .or_else(|| input.options.get("archive_compression"))
        .map(|value| value.to_ascii_lowercase());
    if matches!(compression.as_deref(), Some("gzip" | "gz")) {
        return true;
    }
    let name = archive_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    name.ends_with(".tar.gz") || name.ends_with(".tgz")
}

fn collect_entries(source_files: &[PathBuf]) -> Result<Vec<TarEntry>, String> {
    let mut entries = Vec::new();
    for source in source_files {
        if !source.exists() {
            return Err(format!("Source file not found: {}", source.display()));
        }
        if source.is_dir() {
            collect_dir(source, source, &mut entries)?;
        } else {
            let name = source
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| format!("Invalid file name: {}", source.display()))?;
            entries.push(TarEntry {
                name: normalize_entry_name(name),
                data: std::fs::read(source)
                    .map_err(|e| format!("Cannot read {}: {}", source.display(), e))?,
                is_dir: false,
            });
        }
    }
    entries.sort_unstable_by(|left, right| left.name.cmp(&right.name));
    entries.dedup_by(|left, right| left.name == right.name);
    Ok(entries)
}

fn collect_dir(root: &Path, dir: &Path, entries: &mut Vec<TarEntry>) -> Result<(), String> {
    let mut children = std::fs::read_dir(dir)
        .map_err(|e| format!("Cannot read directory {}: {}", dir.display(), e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Cannot read directory {}: {}", dir.display(), e))?;
    children.sort_unstable_by_key(|entry| entry.path());

    for child in children {
        let path = child.path();
        let relative = path
            .strip_prefix(root)
            .map_err(|e| format!("Cannot relativize {}: {}", path.display(), e))?;
        let name = normalize_entry_path(relative);
        if path.is_dir() {
            entries.push(TarEntry {
                name,
                data: Vec::new(),
                is_dir: true,
            });
            collect_dir(root, &path, entries)?;
        } else if path.is_file() {
            entries.push(TarEntry {
                name,
                data: std::fs::read(&path)
                    .map_err(|e| format!("Cannot read {}: {}", path.display(), e))?,
                is_dir: false,
            });
        }
    }
    Ok(())
}

fn normalize_entry_path(path: &Path) -> String {
    normalize_entry_name(&path.to_string_lossy())
}

fn normalize_entry_name(name: &str) -> String {
    name.replace(std::path::MAIN_SEPARATOR, "/")
}

fn write_archive(archive_path: &Path, entries: &[TarEntry], gzip: bool) -> Result<(), String> {
    let file = std::fs::File::create(archive_path)
        .map_err(|e| format!("Cannot create {}: {}", archive_path.display(), e))?;
    if gzip {
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let encoder = write_tar(encoder, entries)?;
        encoder.finish().map_err(|e| {
            format!(
                "Cannot finish gzip archive {}: {}",
                archive_path.display(),
                e
            )
        })?;
        Ok(())
    } else {
        write_tar(file, entries)?;
        Ok(())
    }
}

fn write_tar<W: Write>(writer: W, entries: &[TarEntry]) -> Result<W, String> {
    let mut builder = ::tar::Builder::new(writer);
    for entry in entries {
        let mut header = ::tar::Header::new_gnu();
        header
            .set_path(&entry.name)
            .map_err(|e| format!("Cannot set tar entry path {}: {}", entry.name, e))?;
        header.set_mtime(0);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mode(if entry.is_dir { 0o755 } else { 0o644 });
        header.set_size(entry.data.len() as u64);
        header.set_entry_type(if entry.is_dir {
            ::tar::EntryType::Directory
        } else {
            ::tar::EntryType::Regular
        });
        header.set_cksum();
        builder
            .append_data(&mut header, &entry.name, Cursor::new(&entry.data))
            .map_err(|e| format!("Cannot append tar entry {}: {}", entry.name, e))?;
    }
    builder
        .into_inner()
        .map_err(|e| format!("Cannot finish tar archive: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn test_tar_create_simple() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        let out_dir = tmp.path().join("out");

        fs::create_dir_all(src_dir.join("com/example")).unwrap();
        fs::write(src_dir.join("com/example/App.class"), b"class App {}").unwrap();
        fs::write(src_dir.join("application.properties"), b"name=app").unwrap();

        let executor = TarTaskExecutor::new();
        let mut input = TaskInput::new("Tar");
        input.source_files.push(src_dir);
        input.target_dir = out_dir.clone();
        input
            .options
            .insert("tarName".to_string(), "app.tar".to_string());

        let result = executor.execute(&input).await;

        assert!(
            result.success,
            "TAR creation failed: {}",
            result.error_message
        );
        assert_eq!(result.files_processed, 2);
        assert_eq!(result.bytes_processed, 20);

        let file = fs::File::open(out_dir.join("app.tar")).unwrap();
        let mut archive = ::tar::Archive::new(file);
        let names = archive
            .entries()
            .unwrap()
            .map(|entry| {
                entry
                    .unwrap()
                    .path()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();

        assert!(names.contains(&"application.properties".to_string()));
        assert!(names.contains(&"com/example".to_string()));
        assert!(names.contains(&"com/example/App.class".to_string()));
    }

    #[tokio::test]
    async fn test_tar_create_gzip_from_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src");
        let out_dir = tmp.path().join("out");

        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("App.class"), b"class App {}").unwrap();

        let executor = TarTaskExecutor::new();
        let mut input = TaskInput::new("Tar");
        input.source_files.push(src_dir);
        input.target_dir = out_dir.clone();
        input
            .options
            .insert("tarName".to_string(), "app.tar.gz".to_string());

        let result = executor.execute(&input).await;

        assert!(
            result.success,
            "TAR.GZ creation failed: {}",
            result.error_message
        );

        let file = fs::File::open(out_dir.join("app.tar.gz")).unwrap();
        let decoder = flate2::read::GzDecoder::new(file);
        let mut archive = ::tar::Archive::new(decoder);
        let names = archive
            .entries()
            .unwrap()
            .map(|entry| {
                entry
                    .unwrap()
                    .path()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();

        assert_eq!(names, vec!["App.class"]);
    }

    #[tokio::test]
    async fn test_tar_create_gzip_from_option() {
        let tmp = tempfile::tempdir().unwrap();
        let src_file = tmp.path().join("readme.txt");
        let out_dir = tmp.path().join("out");
        fs::write(&src_file, b"hello").unwrap();

        let executor = TarTaskExecutor::new();
        let mut input = TaskInput::new("Tar");
        input.source_files.push(src_file);
        input.target_dir = out_dir.clone();
        input
            .options
            .insert("tarName".to_string(), "readme.tgz".to_string());
        input
            .options
            .insert("compression".to_string(), "gzip".to_string());

        let result = executor.execute(&input).await;
        assert!(result.success, "{}", result.error_message);

        let file = fs::File::open(out_dir.join("readme.tgz")).unwrap();
        let decoder = flate2::read::GzDecoder::new(file);
        let mut archive = ::tar::Archive::new(decoder);
        let mut entries = archive.entries().unwrap();
        let entry = entries.next().unwrap().unwrap();
        assert_eq!(entry.path().unwrap().to_string_lossy(), "readme.txt");
    }

    #[tokio::test]
    async fn test_tar_missing_source_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let executor = TarTaskExecutor::new();
        let mut input = TaskInput::new("Tar");
        input.source_files.push(tmp.path().join("missing"));
        input.target_dir = tmp.path().join("out");

        let result = executor.execute(&input).await;

        assert!(!result.success);
        assert!(result.error_message.contains("Source file not found"));
    }
}
