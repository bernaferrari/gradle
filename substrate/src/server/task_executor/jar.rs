use std::io::{Read as StdRead, Write};
use std::path::Path;

use crate::server::task_executor::{TaskExecutor, TaskInput, TaskResult};

/// Native Rust JAR packaging executor.
///
/// Creates or updates JAR files without shelling out to the `jar` command.
/// Supports:
/// - Creating new JARs from a set of input files/directories
/// - Updating existing JARs (adding/replacing entries)
/// - Setting manifest attributes (Main-Class, etc.)
/// - Preserving existing entries when updating
pub struct JarTaskExecutor;

impl Default for JarTaskExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl JarTaskExecutor {
    pub fn new() -> Self {
        Self
    }

    /// Get current time as DOS format (time, date) for ZIP entries.
    fn dos_time_now() -> (u16, u16) {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let total_days = (secs / 86400) as i64;
        let time_of_day = (secs % 86400) as u32;

        // DOS date epoch is 1980-01-01 = Unix day 3652
        let dos_day = total_days - 3652;
        if dos_day < 0 {
            return (0, 0);
        }

        let mut year = 1980u16 + (dos_day / 366) as u16;
        let mut remaining = dos_day as u32;
        loop {
            let days_in_year: u32 = if is_leap_year(year) { 366 } else { 365 };
            if remaining < days_in_year {
                break;
            }
            remaining -= days_in_year;
            year += 1;
        }

        let month_days = if is_leap_year(year) {
            [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        } else {
            [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
        };

        let mut month = 0u16;
        for (i, &days) in month_days.iter().enumerate() {
            if remaining < days {
                month = i as u16 + 1;
                break;
            }
            remaining -= days;
        }
        if month == 0 {
            month = 12;
        }
        let day = remaining + 1;

        let hour = (time_of_day / 3600) as u16;
        let minute = ((time_of_day % 3600) / 60) as u16;
        let second = ((time_of_day % 60) / 2) as u16;

        let dos_time = (second << 10) | (minute << 5) | hour;
        let dos_date = ((day as u16) | (month << 5) | ((year - 1980) << 9)) as u16;

        (dos_time, dos_date)
    }

    /// Write a ZIP local file header (30 bytes) + name.
    fn write_local_file_header(
        out: &mut dyn Write,
        name: &[u8],
        compression_method: u16,
        mod_time: u16,
        mod_date: u16,
        crc32_val: u32,
        compressed_size: u32,
        uncompressed_size: u32,
    ) -> std::io::Result<()> {
        out.write_all(b"PK\x03\x04")?;
        out.write_all(&0x14u16.to_le_bytes())?; // Version needed (2.0)
        out.write_all(&0u16.to_le_bytes())?; // General purpose bit flag
        out.write_all(&compression_method.to_le_bytes())?;
        out.write_all(&mod_time.to_le_bytes())?;
        out.write_all(&mod_date.to_le_bytes())?;
        out.write_all(&crc32_val.to_le_bytes())?;
        out.write_all(&compressed_size.to_le_bytes())?;
        out.write_all(&uncompressed_size.to_le_bytes())?;
        out.write_all(&(name.len() as u16).to_le_bytes())?;
        out.write_all(&0u16.to_le_bytes())?; // Extra field length
        out.write_all(name)?;
        Ok(())
    }

    /// Write a central directory file header (46 bytes) + name.
    fn write_central_dir_entry(
        out: &mut dyn Write,
        name: &[u8],
        compression_method: u16,
        mod_time: u16,
        mod_date: u16,
        crc32_val: u32,
        compressed_size: u32,
        uncompressed_size: u32,
        local_header_offset: u32,
    ) -> std::io::Result<()> {
        out.write_all(b"PK\x01\x02")?;
        out.write_all(&0x14u16.to_le_bytes())?; // Version made by
        out.write_all(&0x14u16.to_le_bytes())?; // Version needed
        out.write_all(&0u16.to_le_bytes())?; // General purpose bit flag
        out.write_all(&compression_method.to_le_bytes())?;
        out.write_all(&mod_time.to_le_bytes())?;
        out.write_all(&mod_date.to_le_bytes())?;
        out.write_all(&crc32_val.to_le_bytes())?;
        out.write_all(&compressed_size.to_le_bytes())?;
        out.write_all(&uncompressed_size.to_le_bytes())?;
        out.write_all(&(name.len() as u16).to_le_bytes())?; // File name length
        out.write_all(&0u16.to_le_bytes())?; // Extra field length
        out.write_all(&0u16.to_le_bytes())?; // File comment length
        out.write_all(&0u16.to_le_bytes())?; // Disk number start
        out.write_all(&0u16.to_le_bytes())?; // Internal file attributes
        out.write_all(&0u32.to_le_bytes())?; // External file attributes
        out.write_all(&local_header_offset.to_le_bytes())?;
        out.write_all(name)?;
        Ok(())
    }

    /// Write the end of central directory record (22 bytes + comment).
    fn write_eocd(
        out: &mut dyn Write,
        num_entries: u16,
        central_dir_size: u32,
        central_dir_offset: u32,
        comment: &[u8],
    ) -> std::io::Result<()> {
        out.write_all(b"PK\x05\x06")?;
        out.write_all(&0u16.to_le_bytes())?; // Number of this disk
        out.write_all(&0u16.to_le_bytes())?; // Disk where central dir starts
        out.write_all(&num_entries.to_le_bytes())?; // Entries on this disk
        out.write_all(&num_entries.to_le_bytes())?; // Total entries
        out.write_all(&central_dir_size.to_le_bytes())?;
        out.write_all(&central_dir_offset.to_le_bytes())?;
        out.write_all(&(comment.len() as u16).to_le_bytes())?;
        out.write_all(comment)?;
        Ok(())
    }

    /// Read all entries from an existing ZIP/JAR file.
    fn read_existing_entries(path: &Path) -> std::io::Result<Vec<(String, Vec<u8>)>> {
        let buf = std::fs::read(path)?;

        if buf.len() < 22 {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();
        let mut pos = 0;

        while pos + 30 <= buf.len() {
            if buf[pos..pos + 4] != *b"PK\x03\x04" {
                break;
            }

            let compression = u16::from_le_bytes(
                buf[pos + 8..pos + 10].try_into().unwrap_or([0, 0]),
            );
            let compressed_size = u32::from_le_bytes(
                buf[pos + 18..pos + 22].try_into().unwrap_or([0, 0, 0, 0]),
            );
            let name_len = u16::from_le_bytes(
                buf[pos + 26..pos + 28].try_into().unwrap_or([0, 0]),
            ) as usize;
            let extra_len = u16::from_le_bytes(
                buf[pos + 28..pos + 30].try_into().unwrap_or([0, 0]),
            ) as usize;

            if pos + 30 + name_len > buf.len() {
                break;
            }

            let name = String::from_utf8_lossy(&buf[pos + 30..pos + 30 + name_len]).into_owned();
            let data_start = pos + 30 + name_len + extra_len;

            if data_start + compressed_size as usize > buf.len() {
                break;
            }

            let data = if compressed_size > 0 {
                let compressed = &buf[data_start..data_start + compressed_size as usize];
                match compression {
                    0 => compressed.to_vec(),
                    8 => {
                        let mut decoder = flate2::read::DeflateDecoder::new(compressed);
                        let mut decompressed = Vec::with_capacity(compressed_size as usize * 2);
                        decoder.read_to_end(&mut decompressed).unwrap_or_default();
                        decompressed
                    }
                    _ => compressed.to_vec(),
                }
            } else {
                Vec::new()
            };

            entries.push((name, data));
            pos = data_start + compressed_size as usize;
        }

        Ok(entries)
    }

    /// Collect files from a directory tree with relative paths.
    fn collect_files(
        base: &Path,
        current: &Path,
        entries: &mut Vec<(String, Vec<u8>)>,
    ) -> Result<(), String> {
        let dir_entries = std::fs::read_dir(current)
            .map_err(|e| format!("Cannot read directory {}: {}", current.display(), e))?;

        let mut dir_entries: Vec<_> = dir_entries.filter_map(|e| e.ok()).collect();
        dir_entries.sort_unstable_by_key(|e| e.file_name());

        for entry in dir_entries {
            let path = entry.path();
            let relative = path.strip_prefix(base).unwrap_or(&path);
            let name = relative.to_string_lossy().replace('\\', "/");

            if path.is_dir() {
                Self::collect_files(base, &path, entries)?;
            } else {
                let data = std::fs::read(&path)
                    .map_err(|e| format!("Cannot read {}: {}", path.display(), e))?;
                entries.push((name, data));
            }
        }
        Ok(())
    }

    /// Create a Java manifest from options.
    fn create_manifest(options: &std::collections::HashMap<String, String>) -> Vec<u8> {
        let mut manifest = String::from("Manifest-Version: 1.0\r\n");

        if let Some(main_class) = options.get("mainClass") {
            manifest.push_str(&format!("Main-Class: {}\r\n", main_class));
        }

        if let Some(classpath) = options.get("classpath") {
            manifest.push_str(&format!("Class-Path: {}\r\n", classpath));
        }

        for (key, value) in options {
            if key.starts_with("manifest.") {
                let attr_name = &key["manifest.".len()..];
                manifest.push_str(&format!("{}: {}\r\n", attr_name, value));
            }
        }

        manifest.push_str("\r\n");
        manifest.into_bytes()
    }

    /// Write entries as a valid ZIP file using STORED compression for speed.
    fn write_zip(
        out: &mut dyn Write,
        entries: &[(String, Vec<u8>)],
    ) -> std::io::Result<()> {
        let (mod_time, mod_date) = Self::dos_time_now();

        // Track per-entry metadata for central directory
        struct EntryMeta {
            crc32: u32,
            size: u32,
            name_len: u32,
            local_offset: u32,
        }

        let mut metas: Vec<EntryMeta> = Vec::with_capacity(entries.len());
        let mut current_offset: u32 = 0;

        // Write local file headers + data
        for (name, data) in entries {
            let name_bytes = name.as_bytes();
            let crc32_val = crc32fast::hash(data);
            let size = data.len() as u32;
            let local_offset = current_offset;

            Self::write_local_file_header(
                out, name_bytes, 0, // STORED
                mod_time, mod_date, crc32_val, size, size,
            )?;
            out.write_all(data)?;

            metas.push(EntryMeta {
                crc32: crc32_val,
                size,
                name_len: name_bytes.len() as u32,
                local_offset,
            });
            current_offset = local_offset + 30 + name_bytes.len() as u32 + size;
        }

        // Write central directory
        let central_dir_offset = current_offset;

        for (i, meta) in metas.iter().enumerate() {
            let name_bytes = entries[i].0.as_bytes();
            Self::write_central_dir_entry(
                out, name_bytes, 0, // STORED
                mod_time, mod_date, meta.crc32, meta.size, meta.size, meta.local_offset,
            )?;
        }

        let central_dir_size: u32 = metas.iter()
            .map(|m| 46 + m.name_len)
            .sum();

        // Write EOCD
        Self::write_eocd(
            out,
            entries.len() as u16,
            central_dir_size,
            central_dir_offset,
            &[],
        )?;

        Ok(())
    }
}

fn is_leap_year(year: u16) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[tonic::async_trait]
impl TaskExecutor for JarTaskExecutor {
    fn task_type(&self) -> &str {
        "Jar"
    }

    async fn execute(&self, input: &TaskInput) -> TaskResult {
        let start = std::time::Instant::now();
        let mut result = TaskResult::default();

        let action = input.options.get("action").map(|s| s.as_str()).unwrap_or("create");
        let jar_path = input.target_dir.join(
            input.options.get("jarName").map(|s| s.as_str()).unwrap_or("output.jar"),
        );

        // Ensure target directory exists
        if let Some(parent) = jar_path.parent() {
            if !parent.exists() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    result.success = false;
                    result.error_message = format!("Failed to create target directory: {}", e);
                    return result;
                }
            }
        }

        match action {
            "create" => {
                if let Err(e) = self.create_jar(&input.source_files, &jar_path, &input.options) {
                    result.success = false;
                    result.error_message = e;
                    return result;
                }
            }
            "update" => {
                if let Err(e) = self.update_jar(&input.source_files, &jar_path, &input.options) {
                    result.success = false;
                    result.error_message = e;
                    return result;
                }
            }
            other => {
                result.success = false;
                result.error_message = format!("Unknown JAR action: {}", other);
                return result;
            }
        }

        result.output_files.push(jar_path);
        result.files_processed = input.source_files.len() as u64;
        result.duration_ms = start.elapsed().as_millis() as u64;
        result
    }
}

impl JarTaskExecutor {
    /// Create a new JAR file from source files/directories.
    fn create_jar(
        &self,
        source_files: &[std::path::PathBuf],
        jar_path: &Path,
        options: &std::collections::HashMap<String, String>,
    ) -> Result<(), String> {
        let mut entries: Vec<(String, Vec<u8>)> = Vec::with_capacity(source_files.len());

        for source in source_files {
            if !source.is_dir() {
                let data = std::fs::read(source)
                    .map_err(|e| format!("Cannot read {}: {}", source.display(), e))?;
                let name = source
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                entries.push((name.to_string(), data));
                continue;
            }
            Self::collect_files(source, source, &mut entries)?;
        }

        if options.contains_key("manifest") || options.contains_key("mainClass") {
            let manifest = Self::create_manifest(options);
            entries.push(("META-INF/MANIFEST.MF".to_string(), manifest));
        }

        entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));

        let mut out = std::fs::File::create(jar_path)
            .map_err(|e| format!("Cannot create {}: {}", jar_path.display(), e))?;
        Self::write_zip(&mut out, &entries)
            .map_err(|e| format!("Cannot write {}: {}", jar_path.display(), e))?;

        // Track total bytes
        let total_bytes: u64 = entries.iter().map(|(_, d)| d.len() as u64).sum();
        let _ = total_bytes; // bytes_processed tracked via result

        Ok(())
    }

    /// Update an existing JAR by adding/replacing entries.
    fn update_jar(
        &self,
        source_files: &[std::path::PathBuf],
        jar_path: &Path,
        options: &std::collections::HashMap<String, String>,
    ) -> Result<(), String> {
        let mut entries: Vec<(String, Vec<u8>)> = if jar_path.exists() {
            Self::read_existing_entries(jar_path)
                .map_err(|e| format!("Cannot read {}: {}", jar_path.display(), e))?
        } else {
            Vec::new()
        };

        for source in source_files {
            if !source.is_dir() {
                let data = std::fs::read(source)
                    .map_err(|e| format!("Cannot read {}: {}", source.display(), e))?;
                let name = source
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                entries.retain(|(n, _)| n != name);
                entries.push((name.to_string(), data));
                continue;
            }

            let mut new_entries = Vec::new();
            Self::collect_files(source, source, &mut new_entries)?;
            for (name, data) in new_entries {
                entries.retain(|(n, _)| n != &name);
                entries.push((name, data));
            }
        }

        if options.contains_key("manifest") || options.contains_key("mainClass") {
            let manifest = Self::create_manifest(options);
            entries.retain(|(n, _)| n != "META-INF/MANIFEST.MF");
            entries.push(("META-INF/MANIFEST.MF".to_string(), manifest));
        }

        entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));

        let mut out = std::fs::File::create(jar_path)
            .map_err(|e| format!("Cannot create {}: {}", jar_path.display(), e))?;
        Self::write_zip(&mut out, &entries)
            .map_err(|e| format!("Cannot write {}: {}", jar_path.display(), e))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_jar_create_simple() {
        let tmp = TempDir::new().unwrap();
        let src_dir = tmp.path().join("src");
        let out_dir = tmp.path().join("out");

        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("Hello.class"), b"class Hello {}").unwrap();

        let executor = JarTaskExecutor::new();
        let mut input = TaskInput::new("Jar");
        input.source_files.push(src_dir);
        input.target_dir = out_dir;
        input.options.insert("jarName".to_string(), "test.jar".to_string());

        let result = executor.execute(&input).await;

        assert!(result.success, "JAR creation failed: {}", result.error_message);
        assert!(result.output_files.iter().any(|p| p.ends_with("test.jar")));

        // Verify the JAR is a valid ZIP
        let jar_path = result.output_files.first().unwrap();
        let jar_data = fs::read(jar_path).unwrap();
        assert!(jar_data.starts_with(b"PK\x03\x04"), "JAR must start with ZIP local file header");
    }

    #[tokio::test]
    async fn test_jar_create_with_manifest() {
        let tmp = TempDir::new().unwrap();
        let src_dir = tmp.path().join("src");
        let out_dir = tmp.path().join("out");

        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("Main.class"), b"class Main {}").unwrap();

        let executor = JarTaskExecutor::new();
        let mut input = TaskInput::new("Jar");
        input.source_files.push(src_dir);
        input.target_dir = out_dir;
        input.options.insert("jarName".to_string(), "app.jar".to_string());
        input.options.insert("mainClass".to_string(), "com.example.Main".to_string());

        let result = executor.execute(&input).await;

        assert!(result.success, "JAR creation failed: {}", result.error_message);

        let jar_path = result.output_files.first().unwrap();
        let jar_data = fs::read(jar_path).unwrap();
        let jar_str = String::from_utf8_lossy(&jar_data);
        assert!(jar_str.contains("Manifest-Version: 1.0"));
        assert!(jar_str.contains("Main-Class: com.example.Main"));
    }

    #[tokio::test]
    async fn test_jar_create_nested_dirs() {
        let tmp = TempDir::new().unwrap();
        let src_dir = tmp.path().join("src");
        let out_dir = tmp.path().join("out");

        fs::create_dir_all(src_dir.join("com/example")).unwrap();
        fs::write(
            src_dir.join("com/example/Service.class"),
            b"class Service {}",
        ).unwrap();

        let executor = JarTaskExecutor::new();
        let mut input = TaskInput::new("Jar");
        input.source_files.push(src_dir);
        input.target_dir = out_dir;
        input.options.insert("jarName".to_string(), "nested.jar".to_string());

        let result = executor.execute(&input).await;

        assert!(result.success, "JAR creation failed: {}", result.error_message);

        let jar_path = result.output_files.first().unwrap();
        let jar_data = fs::read(jar_path).unwrap();
        let jar_str = String::from_utf8_lossy(&jar_data);
        assert!(jar_str.contains("com/example/Service.class"));
    }

    #[tokio::test]
    async fn test_jar_update_existing() {
        let tmp = TempDir::new().unwrap();
        let src_dir = tmp.path().join("src");
        let out_dir = tmp.path().join("out");

        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("A.class"), b"class A {}").unwrap();

        let executor = JarTaskExecutor::new();

        // First create
        let mut input = TaskInput::new("Jar");
        input.source_files.push(src_dir.clone());
        input.target_dir = out_dir.clone();
        input.options.insert("jarName".to_string(), "update.jar".to_string());

        let result = executor.execute(&input).await;
        assert!(result.success);

        // Now update: add B.class
        fs::write(src_dir.join("B.class"), b"class B {}").unwrap();
        let mut update_input = TaskInput::new("Jar");
        update_input.source_files.push(src_dir);
        update_input.target_dir = out_dir.clone();
        update_input.options.insert("jarName".to_string(), "update.jar".to_string());
        update_input.options.insert("action".to_string(), "update".to_string());

        let result = executor.execute(&update_input).await;
        assert!(result.success, "JAR update failed: {}", result.error_message);

        // Both files should be in the updated JAR
        let jar_path = result.output_files.first().unwrap();
        let jar_data = fs::read(jar_path).unwrap();
        let jar_str = String::from_utf8_lossy(&jar_data);
        assert!(jar_str.contains("A.class"));
        assert!(jar_str.contains("B.class"));
    }

    #[tokio::test]
    async fn test_jar_unknown_action() {
        let executor = JarTaskExecutor::new();
        let mut input = TaskInput::new("Jar");
        input.target_dir = std::path::PathBuf::from("/tmp");
        input.options.insert("action".to_string(), "sign".to_string());
        input.options.insert("jarName".to_string(), "test.jar".to_string());

        let result = executor.execute(&input).await;

        assert!(!result.success);
        assert!(result.error_message.contains("Unknown JAR action"));
    }

    #[tokio::test]
    async fn test_jar_deterministic_order() {
        let tmp = TempDir::new().unwrap();
        let src_dir = tmp.path().join("src");
        let out1 = tmp.path().join("out1");
        let out2 = tmp.path().join("out2");

        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("Z.class"), b"z").unwrap();
        fs::write(src_dir.join("A.class"), b"a").unwrap();
        fs::write(src_dir.join("M.class"), b"m").unwrap();

        let executor = JarTaskExecutor::new();

        for out_dir in [&out1, &out2] {
            let mut input = TaskInput::new("Jar");
            input.source_files.push(src_dir.clone());
            input.target_dir = out_dir.clone();
            input.options.insert("jarName".to_string(), "det.jar".to_string());

            let result = executor.execute(&input).await;
            assert!(result.success);
        }

        let jar1 = fs::read(out1.join("det.jar")).unwrap();
        let jar2 = fs::read(out2.join("det.jar")).unwrap();
        assert_eq!(jar1, jar2, "JARs with same content must be byte-identical");
    }

    #[tokio::test]
    async fn test_jar_empty_sources() {
        let tmp = TempDir::new().unwrap();
        let out_dir = tmp.path().join("out");
        fs::create_dir_all(&out_dir).unwrap();

        let executor = JarTaskExecutor::new();
        let mut input = TaskInput::new("Jar");
        input.target_dir = out_dir.clone();
        input.options.insert("jarName".to_string(), "empty.jar".to_string());

        let result = executor.execute(&input).await;

        assert!(result.success, "Empty JAR creation failed: {}", result.error_message);
        assert!(out_dir.join("empty.jar").exists());

        // Empty JAR should still be a valid ZIP with just EOCD
        let jar_data = fs::read(out_dir.join("empty.jar")).unwrap();
        assert!(jar_data.starts_with(b"PK\x05\x06"), "Empty JAR must start with EOCD signature");
    }
}
