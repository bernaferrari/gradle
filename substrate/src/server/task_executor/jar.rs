use std::io::{Read as StdRead, Write};
use std::path::{Path, PathBuf};

use std::collections::HashSet;

use crate::server::task_executor::copy::{parse_copy_file_mappings, parse_unix_mode};
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct ZipEntry {
    name: String,
    data: Vec<u8>,
    mode: u32,
}

impl ZipEntry {
    fn file(name: impl Into<String>, data: Vec<u8>, mode: u32) -> Self {
        Self {
            name: name.into(),
            data,
            mode,
        }
    }

    fn dir(name: impl Into<String>, mode: u32) -> Self {
        Self {
            name: name.into(),
            data: Vec::new(),
            mode,
        }
    }

    fn is_dir(&self) -> bool {
        self.name.ends_with('/')
    }
}

fn is_directory_symlink(path: &Path) -> Result<bool, String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|e| format!("Cannot inspect {}: {}", path.display(), e))?;
    if !metadata.file_type().is_symlink() {
        return Ok(false);
    }
    std::fs::metadata(path)
        .map(|target| target.is_dir())
        .map_err(|e| format!("Cannot inspect symlink target {}: {}", path.display(), e))
}

fn fail_directory_symlink(path: &Path) -> Result<(), String> {
    if is_directory_symlink(path)? {
        return Err(format!(
            "Directory symlink inputs are not supported by the Rust archive executor: {}",
            path.display()
        ));
    }
    Ok(())
}

impl Default for JarTaskExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl JarTaskExecutor {
    pub fn new() -> Self {
        Self
    }

    /// Fixed DOS timestamp for reproducible ZIP-compatible archives.
    fn reproducible_dos_timestamp() -> (u16, u16) {
        const DOS_TIME_MIDNIGHT: u16 = 0;
        const DOS_DATE_1980_01_01: u16 = 1 | (1 << 5);
        (DOS_TIME_MIDNIGHT, DOS_DATE_1980_01_01)
    }

    /// Write a ZIP local file header (30 bytes) + name.
    #[allow(clippy::too_many_arguments)]
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
    #[allow(clippy::too_many_arguments)]
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
        external_file_attributes: u32,
    ) -> std::io::Result<()> {
        out.write_all(b"PK\x01\x02")?;
        out.write_all(&((3u16 << 8) | 20u16).to_le_bytes())?; // Version made by: Unix, ZIP 2.0
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
        out.write_all(&external_file_attributes.to_le_bytes())?;
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
    fn read_existing_entries(path: &Path) -> std::io::Result<Vec<ZipEntry>> {
        let buf = std::fs::read(path)?;
        Ok(Self::read_existing_entries_from_bytes(&buf))
    }

    fn read_existing_entries_from_bytes(buf: &[u8]) -> Vec<ZipEntry> {
        if buf.len() < 22 {
            return Vec::new();
        }

        let mut entries = Vec::new();
        let mut pos = 0;

        while pos + 30 <= buf.len() {
            if buf[pos..pos + 4] != *b"PK\x03\x04" {
                break;
            }

            let compression =
                u16::from_le_bytes(buf[pos + 8..pos + 10].try_into().unwrap_or([0, 0]));
            let compressed_size =
                u32::from_le_bytes(buf[pos + 18..pos + 22].try_into().unwrap_or([0, 0, 0, 0]));
            let name_len =
                u16::from_le_bytes(buf[pos + 26..pos + 28].try_into().unwrap_or([0, 0])) as usize;
            let extra_len =
                u16::from_le_bytes(buf[pos + 28..pos + 30].try_into().unwrap_or([0, 0])) as usize;

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

            let mode = if name.ends_with('/') { 0o755 } else { 0o644 };
            entries.push(ZipEntry::file(name, data, mode));
            pos = data_start + compressed_size as usize;
        }

        entries
    }

    /// Collect files from a directory tree with relative paths.
    fn collect_files(
        base: &Path,
        current: &Path,
        entries: &mut Vec<ZipEntry>,
        emitted_dirs: &mut HashSet<String>,
        include_empty_dirs: bool,
        file_mode: u32,
        dir_mode: u32,
    ) -> Result<(), String> {
        fail_directory_symlink(current)?;
        let dir_entries = std::fs::read_dir(current)
            .map_err(|e| format!("Cannot read directory {}: {}", current.display(), e))?;

        let mut dir_entries: Vec<_> = dir_entries.filter_map(|e| e.ok()).collect();
        dir_entries.sort_unstable_by_key(|e| e.file_name());

        for entry in dir_entries {
            let path = entry.path();
            let relative = path.strip_prefix(base).unwrap_or(&path);
            let name = relative.to_string_lossy().replace('\\', "/");
            let file_type = entry
                .file_type()
                .map_err(|e| format!("Cannot inspect {}: {}", path.display(), e))?;

            if file_type.is_symlink() {
                fail_directory_symlink(&path)?;
                push_zip_parent_dirs(entries, emitted_dirs, &name, dir_mode);
                let data = std::fs::read(&path)
                    .map_err(|e| format!("Cannot read {}: {}", path.display(), e))?;
                entries.push(ZipEntry::file(name, data, file_mode));
            } else if file_type.is_dir() {
                if include_empty_dirs {
                    push_zip_dir_entry(entries, emitted_dirs, &name, dir_mode);
                }
                Self::collect_files(
                    base,
                    &path,
                    entries,
                    emitted_dirs,
                    include_empty_dirs,
                    file_mode,
                    dir_mode,
                )?;
            } else if file_type.is_file() {
                push_zip_parent_dirs(entries, emitted_dirs, &name, dir_mode);
                let data = std::fs::read(&path)
                    .map_err(|e| format!("Cannot read {}: {}", path.display(), e))?;
                entries.push(ZipEntry::file(name, data, file_mode));
            }
        }
        Ok(())
    }

    fn collect_file_entries_with_inferred_root(
        source_files: &[PathBuf],
        entries: &mut Vec<ZipEntry>,
        file_mode: u32,
        dir_mode: u32,
    ) -> Result<bool, String> {
        if source_files.len() < 2 || source_files.iter().any(|source| source.is_dir()) {
            return Ok(false);
        }
        let Some(root) = common_file_parent(source_files) else {
            return Ok(false);
        };
        if !looks_like_archive_source_root(&root) {
            return Ok(false);
        }

        let mut emitted_dirs = HashSet::new();
        for source in source_files {
            fail_directory_symlink(source)?;
            if !source.exists() {
                return Err(format!("Source file not found: {}", source.display()));
            }
            let relative = source
                .strip_prefix(&root)
                .map_err(|e| format!("Cannot relativize {}: {}", source.display(), e))?;
            let name = relative.to_string_lossy().replace('\\', "/");
            push_zip_parent_dirs(entries, &mut emitted_dirs, &name, dir_mode);
            let data = std::fs::read(source)
                .map_err(|e| format!("Cannot read {}: {}", source.display(), e))?;
            entries.push(ZipEntry::file(name, data, file_mode));
        }
        Ok(true)
    }

    fn collect_mapped_entries(
        options: &std::collections::HashMap<String, String>,
        entries: &mut Vec<ZipEntry>,
        include_empty_dirs: bool,
        file_mode: u32,
        dir_mode: u32,
    ) -> Result<bool, String> {
        let mappings = parse_copy_file_mappings(options.get("copy_file_mappings"))?;
        if mappings.is_empty() {
            return Ok(false);
        }
        let mut emitted_dirs = HashSet::new();
        let duplicate_strategy = archive_duplicate_strategy(options);
        let available_mapping_names = mappings
            .iter()
            .filter(|mapping| mapping.source.exists())
            .map(|mapping| mapping.relative_path.to_string_lossy().replace('\\', "/"))
            .collect::<HashSet<_>>();
        let mut seen_entries = entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect::<HashSet<_>>();
        for mapping in mappings {
            let name = mapping.relative_path.to_string_lossy().replace('\\', "/");
            if !mapping.source.exists() {
                if is_manifest_entry(&name) && has_manifest_options(options) {
                    if include_empty_dirs {
                        push_zip_parent_dirs(entries, &mut emitted_dirs, &name, dir_mode);
                    }
                    continue;
                }
                if duplicate_strategy == "EXCLUDE"
                    && (seen_entries.contains(&name) || available_mapping_names.contains(&name))
                {
                    continue;
                }
                return Err(format!(
                    "Source file not found: {}",
                    mapping.source.display()
                ));
            }
            fail_directory_symlink(&mapping.source)?;
            if mapping.is_dir {
                if include_empty_dirs {
                    push_zip_dir_entry(entries, &mut emitted_dirs, &name, dir_mode);
                }
                continue;
            }
            if include_empty_dirs {
                push_zip_parent_dirs(entries, &mut emitted_dirs, &name, dir_mode);
            }
            let data = std::fs::read(&mapping.source)
                .map_err(|e| format!("Cannot read {}: {}", mapping.source.display(), e))?;
            seen_entries.insert(name.clone());
            entries.push(ZipEntry::file(name, data, file_mode));
        }
        Ok(true)
    }

    /// Create a Java manifest from options.
    fn create_manifest(options: &std::collections::HashMap<String, String>) -> Vec<u8> {
        let mut manifest = Vec::new();
        append_manifest_attribute(&mut manifest, "Manifest-Version", "1.0");

        if let Some(main_class) = options.get("mainClass") {
            append_manifest_attribute(&mut manifest, "Main-Class", main_class);
        }

        if let Some(classpath) = options.get("classpath") {
            append_manifest_attribute(&mut manifest, "Class-Path", classpath);
        }

        let mut custom_attributes: Vec<_> = options
            .iter()
            .filter_map(|(key, value)| key.strip_prefix("manifest.").map(|name| (name, value)))
            .filter(|(name, _)| !name.eq_ignore_ascii_case("Manifest-Version"))
            .collect();
        custom_attributes.sort_unstable_by(|left, right| left.0.cmp(right.0));
        for (attr_name, value) in custom_attributes {
            append_manifest_attribute(&mut manifest, attr_name, value);
        }

        manifest.extend_from_slice(b"\r\n");
        manifest
    }

    /// Write entries as a valid ZIP file using STORED compression for speed.
    fn write_zip(out: &mut dyn Write, entries: &[ZipEntry]) -> std::io::Result<()> {
        let (mod_time, mod_date) = Self::reproducible_dos_timestamp();

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
        for entry in entries {
            let name_bytes = entry.name.as_bytes();
            let crc32_val = crc32fast::hash(&entry.data);
            let size = entry.data.len() as u32;
            let local_offset = current_offset;

            Self::write_local_file_header(
                out, name_bytes, 0, // STORED
                mod_time, mod_date, crc32_val, size, size,
            )?;
            out.write_all(&entry.data)?;

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
            let entry = &entries[i];
            let name_bytes = entry.name.as_bytes();
            let is_dir = entry.is_dir();
            let external_file_attributes = if is_dir {
                ((0o040000u32 | entry.mode) << 16) | 0x10
            } else {
                (0o100000u32 | entry.mode) << 16
            };
            Self::write_central_dir_entry(
                out,
                name_bytes,
                0, // STORED
                mod_time,
                mod_date,
                meta.crc32,
                meta.size,
                meta.size,
                meta.local_offset,
                external_file_attributes,
            )?;
        }

        let central_dir_size: u32 = metas.iter().map(|m| 46 + m.name_len).sum();

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

fn append_manifest_attribute(out: &mut Vec<u8>, name: &str, value: &str) {
    append_wrapped_manifest_line(out, &format!("{}: {}", name, value));
}

fn append_wrapped_manifest_line(out: &mut Vec<u8>, line: &str) {
    const MAX_MANIFEST_LINE_BYTES: usize = 72;
    let mut remaining = line.as_bytes();
    let mut first = true;

    while !remaining.is_empty() {
        let available = if first {
            MAX_MANIFEST_LINE_BYTES
        } else {
            MAX_MANIFEST_LINE_BYTES - 1
        };
        let take = remaining.len().min(available);
        if !first {
            out.push(b' ');
        }
        out.extend_from_slice(&remaining[..take]);
        out.extend_from_slice(b"\r\n");
        remaining = &remaining[take..];
        first = false;
    }
}

fn push_zip_parent_dirs(
    entries: &mut Vec<ZipEntry>,
    emitted_dirs: &mut HashSet<String>,
    entry_name: &str,
    dir_mode: u32,
) {
    let mut prefix = String::new();
    for segment in entry_name.split('/').filter(|segment| !segment.is_empty()) {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(segment);
        if prefix == entry_name.trim_end_matches('/') {
            break;
        }
        push_zip_dir_entry(entries, emitted_dirs, &prefix, dir_mode);
    }
}

fn push_zip_dir_entry(
    entries: &mut Vec<ZipEntry>,
    emitted_dirs: &mut HashSet<String>,
    name: &str,
    dir_mode: u32,
) {
    let normalized = format!("{}/", name.trim_end_matches('/'));
    if normalized == "/" || !emitted_dirs.insert(normalized.clone()) {
        return;
    }
    entries.push(ZipEntry::dir(normalized, dir_mode));
}

fn collect_application_distribution_zip_entries(
    archive_path: &Path,
    options: &std::collections::HashMap<String, String>,
    entries: &mut Vec<ZipEntry>,
    include_empty_dirs: bool,
    file_mode: u32,
    dir_mode: u32,
) -> Result<bool, String> {
    if archive_path
        .parent()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        != Some("distributions")
    {
        return Ok(false);
    }
    let Some(root_name) = distribution_root_name(archive_path) else {
        return Ok(false);
    };
    let Some(build_dir) = archive_path.parent().and_then(|path| path.parent()) else {
        return Ok(false);
    };

    let mut files = Vec::new();
    collect_existing_files(
        &build_dir.join("scripts"),
        &format!("{root_name}/bin"),
        &mut files,
    )?;
    collect_existing_files(
        &build_dir.join("libs"),
        &format!("{root_name}/lib"),
        &mut files,
    )?;
    collect_graph_distribution_libs(options, &format!("{root_name}/lib"), &mut files);
    if files.is_empty() {
        return Ok(false);
    }

    let mut emitted_dirs = HashSet::new();
    for (source, name) in files {
        if include_empty_dirs {
            push_zip_parent_dirs(entries, &mut emitted_dirs, &name, dir_mode);
        }
        let data = std::fs::read(&source)
            .map_err(|e| format!("Cannot read {}: {}", source.display(), e))?;
        entries.push(ZipEntry::file(name, data, file_mode));
    }
    Ok(true)
}

fn collect_existing_files(
    source_dir: &Path,
    destination_dir: &str,
    files: &mut Vec<(PathBuf, String)>,
) -> Result<(), String> {
    if !source_dir.exists() {
        return Ok(());
    }
    fail_directory_symlink(source_dir)?;
    for entry in std::fs::read_dir(source_dir)
        .map_err(|e| format!("Cannot read directory {}: {}", source_dir.display(), e))?
    {
        let entry = entry.map_err(|e| format!("Cannot read directory entry: {}", e))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|e| format!("Cannot inspect {}: {}", path.display(), e))?;
        if file_type.is_symlink() {
            fail_directory_symlink(&path)?;
            files.push((
                path,
                join_archive_path(destination_dir, &entry.file_name().to_string_lossy()),
            ));
            continue;
        }
        if file_type.is_dir() {
            collect_existing_files(
                &path,
                &join_archive_path(destination_dir, &entry.file_name().to_string_lossy()),
                files,
            )?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        files.push((
            path,
            join_archive_path(destination_dir, &entry.file_name().to_string_lossy()),
        ));
    }
    files.sort_unstable_by(|left, right| left.1.cmp(&right.1));
    Ok(())
}

fn collect_graph_distribution_libs(
    options: &std::collections::HashMap<String, String>,
    destination_dir: &str,
    files: &mut Vec<(PathBuf, String)>,
) {
    let Some(libs) = options.get("graph_distribution_libs") else {
        return;
    };
    let mut seen = files
        .iter()
        .map(|(_, name)| name.clone())
        .collect::<HashSet<_>>();
    for path in std::env::split_paths(libs) {
        if !path.is_file() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let entry_name = join_archive_path(destination_dir, file_name);
        if seen.insert(entry_name.clone()) {
            files.push((path, entry_name));
        }
    }
    files.sort_unstable_by(|left, right| left.1.cmp(&right.1));
}

fn join_archive_path(parent: &str, child: &str) -> String {
    let parent = parent.trim_end_matches('/');
    if parent.is_empty() {
        child.trim_start_matches('/').to_string()
    } else {
        format!("{}/{}", parent, child.trim_start_matches('/'))
    }
}

fn collect_convention_archive_entries(
    archive_path: &Path,
    entries: &mut Vec<ZipEntry>,
    file_mode: u32,
    dir_mode: u32,
) -> Result<bool, String> {
    let Some(build_dir) = archive_path.parent().and_then(|path| path.parent()) else {
        return Ok(false);
    };
    let Some(project_dir) = build_dir.parent() else {
        return Ok(false);
    };
    let archive_name = archive_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    let mut files = Vec::new();
    if archive_name.ends_with(".war") {
        collect_existing_files(&project_dir.join("src/main/webapp"), "", &mut files)?;
        collect_existing_files(
            &build_dir.join("classes/java/main"),
            "WEB-INF/classes",
            &mut files,
        )?;
        collect_existing_files(
            &build_dir.join("resources/main"),
            "WEB-INF/classes",
            &mut files,
        )?;
    } else if archive_name.ends_with(".ear") {
        collect_existing_files(&project_dir.join("src/main/application"), "", &mut files)?;
    } else if archive_name.ends_with("-sources.jar") {
        collect_existing_files(&project_dir.join("src/main/java"), "", &mut files)?;
        collect_existing_files(&project_dir.join("src/main/resources"), "", &mut files)?;
    } else if archive_name.ends_with(".jar") {
        collect_existing_files(&build_dir.join("classes/java/main"), "", &mut files)?;
        collect_existing_files(&build_dir.join("resources/main"), "", &mut files)?;
    }

    if files.is_empty() {
        return Ok(false);
    }

    let mut emitted_dirs = HashSet::new();
    for existing in entries.iter().filter(|entry| entry.is_dir()) {
        emitted_dirs.insert(existing.name.clone());
    }
    for (source, name) in files {
        push_zip_parent_dirs(entries, &mut emitted_dirs, &name, dir_mode);
        let data = std::fs::read(&source)
            .map_err(|e| format!("Cannot read {}: {}", source.display(), e))?;
        entries.push(ZipEntry::file(name, data, file_mode));
    }
    Ok(true)
}

fn collect_spring_boot_archive_entries(
    archive_path: &Path,
    options: &std::collections::HashMap<String, String>,
    entries: &mut Vec<ZipEntry>,
    file_mode: u32,
    dir_mode: u32,
) -> Result<bool, String> {
    if !is_spring_boot_archive(options) {
        return Ok(false);
    }
    let Some(build_dir) = archive_path.parent().and_then(|path| path.parent()) else {
        return Ok(false);
    };

    let mut added = false;
    let mut existing = entries
        .iter()
        .map(|entry| entry.name.clone())
        .collect::<HashSet<_>>();
    let mut emitted_dirs = entries
        .iter()
        .filter(|entry| entry.is_dir())
        .map(|entry| entry.name.clone())
        .collect::<HashSet<_>>();

    let mut files = Vec::new();
    collect_existing_files(
        &build_dir.join("classes/java/main"),
        "BOOT-INF/classes",
        &mut files,
    )?;
    collect_existing_files(
        &build_dir.join("resources/main"),
        "BOOT-INF/classes",
        &mut files,
    )?;
    for (source, name) in files {
        if existing.contains(&name) {
            continue;
        }
        push_zip_parent_dirs(entries, &mut emitted_dirs, &name, dir_mode);
        let data = std::fs::read(&source)
            .map_err(|e| format!("Cannot read {}: {}", source.display(), e))?;
        existing.insert(name.clone());
        entries.push(ZipEntry::file(name, data, file_mode));
        added = true;
    }

    for entry in spring_boot_loader_entries(options)? {
        if existing.insert(entry.name.clone()) {
            push_zip_parent_dirs(entries, &mut emitted_dirs, &entry.name, dir_mode);
            entries.push(entry);
            added = true;
        }
    }
    if let Some(jarmode_tools) = spring_boot_jarmode_tools_entry(options) {
        if existing.insert(jarmode_tools.name.clone()) {
            push_zip_parent_dirs(entries, &mut emitted_dirs, &jarmode_tools.name, dir_mode);
            entries.push(jarmode_tools);
            added = true;
        }
    }

    if existing
        .iter()
        .any(|name| name.starts_with("BOOT-INF/lib/"))
    {
        let classpath_index = spring_boot_classpath_index(&existing);
        added |= push_generated_boot_entry(
            entries,
            &mut existing,
            &mut emitted_dirs,
            "BOOT-INF/classpath.idx",
            classpath_index,
            file_mode,
            dir_mode,
        );
        added |= push_generated_boot_entry(
            entries,
            &mut existing,
            &mut emitted_dirs,
            "BOOT-INF/layers.idx",
            spring_boot_layers_index(),
            file_mode,
            dir_mode,
        );
    }

    Ok(added)
}

fn is_spring_boot_archive(options: &std::collections::HashMap<String, String>) -> bool {
    options.contains_key("manifest.Spring-Boot-Version")
        || parse_copy_file_mappings(options.get("copy_file_mappings"))
            .map(|mappings| {
                mappings.iter().any(|mapping| {
                    mapping
                        .relative_path
                        .to_string_lossy()
                        .replace('\\', "/")
                        .starts_with("BOOT-INF/")
                })
            })
            .unwrap_or(false)
}

fn push_generated_boot_entry(
    entries: &mut Vec<ZipEntry>,
    existing: &mut HashSet<String>,
    emitted_dirs: &mut HashSet<String>,
    name: &str,
    data: Vec<u8>,
    file_mode: u32,
    dir_mode: u32,
) -> bool {
    if !existing.insert(name.to_string()) {
        return false;
    }
    push_zip_parent_dirs(entries, emitted_dirs, name, dir_mode);
    entries.push(ZipEntry::file(name, data, file_mode));
    true
}

fn spring_boot_classpath_index(existing: &HashSet<String>) -> Vec<u8> {
    let mut libs = existing
        .iter()
        .filter(|name| name.starts_with("BOOT-INF/lib/") && name.ends_with(".jar"))
        .cloned()
        .collect::<Vec<_>>();
    libs.sort_unstable();
    let mut text = String::new();
    for lib in libs {
        text.push_str("- \"");
        text.push_str(&lib);
        text.push_str("\"\n");
    }
    text.into_bytes()
}

fn spring_boot_layers_index() -> Vec<u8> {
    b"- \"dependencies\":\n  - \"BOOT-INF/lib/\"\n- \"spring-boot-loader\":\n  - \"org/\"\n- \"application\":\n  - \"BOOT-INF/classes/\"\n  - \"BOOT-INF/classpath.idx\"\n  - \"BOOT-INF/layers.idx\"\n"
        .to_vec()
}

fn spring_boot_loader_entries(
    options: &std::collections::HashMap<String, String>,
) -> Result<Vec<ZipEntry>, String> {
    let version = spring_boot_version(options);
    let Some(loader_tools) = find_spring_boot_loader_tools(version) else {
        return Ok(Vec::new());
    };
    let tools_entries = JarTaskExecutor::read_existing_entries(&loader_tools)
        .map_err(|e| format!("Cannot read {}: {}", loader_tools.display(), e))?;
    let Some(loader_jar) = tools_entries
        .into_iter()
        .find(|entry| entry.name == "META-INF/loader/spring-boot-loader.jar")
    else {
        return Ok(Vec::new());
    };
    Ok(
        JarTaskExecutor::read_existing_entries_from_bytes(&loader_jar.data)
            .into_iter()
            .filter(|entry| {
                !entry.is_dir()
                    && (entry.name == "META-INF/services/java.nio.file.spi.FileSystemProvider"
                        || entry.name.starts_with("org/springframework/boot/loader/"))
            })
            .collect(),
    )
}

fn spring_boot_jarmode_tools_entry(
    options: &std::collections::HashMap<String, String>,
) -> Option<ZipEntry> {
    let version = spring_boot_version(options);
    let data = if let Some(path) = find_spring_boot_module_jar(
        "spring-boot-jarmode-tools",
        version,
        &format!("spring-boot-jarmode-tools-{version}.jar"),
    ) {
        std::fs::read(&path).ok()?
    } else {
        let loader_tools = find_spring_boot_loader_tools(version)?;
        JarTaskExecutor::read_existing_entries(&loader_tools)
            .ok()?
            .into_iter()
            .find(|entry| entry.name == "META-INF/jarmode/spring-boot-jarmode-tools.jar")?
            .data
    };
    Some(ZipEntry::file(
        format!("BOOT-INF/lib/spring-boot-jarmode-tools-{version}.jar"),
        data,
        0o644,
    ))
}

fn spring_boot_version(options: &std::collections::HashMap<String, String>) -> &str {
    options
        .get("manifest.Spring-Boot-Version")
        .map(String::as_str)
        .unwrap_or("4.0.6")
}

fn find_spring_boot_loader_tools(version: &str) -> Option<PathBuf> {
    find_spring_boot_module_jar(
        "spring-boot-loader-tools",
        version,
        &format!("spring-boot-loader-tools-{version}.jar"),
    )
}

fn find_spring_boot_module_jar(module: &str, version: &str, file_name: &str) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let base = home
        .join(".gradle/caches/modules-2/files-2.1/org.springframework.boot")
        .join(module)
        .join(version);
    let entries = std::fs::read_dir(base).ok()?;
    for entry in entries.flatten() {
        let path = entry.path().join(file_name);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

fn distribution_root_name(archive_path: &Path) -> Option<String> {
    let file_name = archive_path.file_name()?.to_str()?;
    for suffix in [
        ".tar.gz", ".tar.bz2", ".tgz", ".tbz2", ".tbz", ".zip", ".tar",
    ] {
        if let Some(root) = file_name.strip_suffix(suffix) {
            return Some(root.to_string());
        }
    }
    None
}

fn archive_duplicate_strategy(options: &std::collections::HashMap<String, String>) -> String {
    options
        .get("duplicates_strategy")
        .map(|strategy| strategy.to_ascii_uppercase())
        .unwrap_or_else(|| "INCLUDE".to_string())
}

fn archive_include_empty_dirs(options: &std::collections::HashMap<String, String>) -> bool {
    options
        .get("include_empty_dirs")
        .map(|value| value != "false")
        .unwrap_or(true)
}

fn archive_file_mode(options: &std::collections::HashMap<String, String>) -> u32 {
    parse_unix_mode(options.get("file_permissions")).unwrap_or(0o644)
}

fn archive_dir_mode(options: &std::collections::HashMap<String, String>) -> u32 {
    parse_unix_mode(options.get("dir_permissions")).unwrap_or(0o755)
}

fn resolve_duplicate_entries(
    entries: Vec<ZipEntry>,
    strategy: &str,
) -> Result<Vec<ZipEntry>, String> {
    if strategy == "INCLUDE" {
        return Ok(entries);
    }

    let mut seen = HashSet::new();
    let mut resolved = Vec::with_capacity(entries.len());
    for entry in entries {
        if !seen.insert(entry.name.clone()) {
            if strategy == "FAIL" {
                return Err(format!("Duplicate archive entry: {}", entry.name));
            }
            if strategy == "EXCLUDE" {
                continue;
            }
        }
        resolved.push(entry);
    }
    Ok(resolved)
}

#[tonic::async_trait]
impl TaskExecutor for JarTaskExecutor {
    fn task_type(&self) -> &str {
        "Jar"
    }

    async fn execute(&self, input: &TaskInput) -> TaskResult {
        let start = std::time::Instant::now();
        let mut result = TaskResult::default();

        let action = input
            .options
            .get("action")
            .map(|s| s.as_str())
            .unwrap_or("create");
        let jar_path = input.target_dir.join(
            input
                .options
                .get("jarName")
                .map(|s| s.as_str())
                .unwrap_or("output.jar"),
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
        let mut entries: Vec<ZipEntry> = Vec::with_capacity(source_files.len());
        let include_empty_dirs = archive_include_empty_dirs(options);
        let file_mode = archive_file_mode(options);
        let dir_mode = archive_dir_mode(options);

        let used_mappings = Self::collect_mapped_entries(
            options,
            &mut entries,
            include_empty_dirs,
            file_mode,
            dir_mode,
        )?;
        let mapped_payload_entries = entries
            .iter()
            .any(|entry| !entry.is_dir() && !is_manifest_entry(&entry.name));
        let collected_convention_entries = if mapped_payload_entries {
            false
        } else {
            collect_convention_archive_entries(jar_path, &mut entries, file_mode, dir_mode)?
        };
        let collected_boot_entries = collect_spring_boot_archive_entries(
            jar_path,
            options,
            &mut entries,
            file_mode,
            dir_mode,
        )?;

        if !used_mappings
            && !collected_convention_entries
            && !collected_boot_entries
            && !collect_application_distribution_zip_entries(
                jar_path,
                options,
                &mut entries,
                include_empty_dirs,
                file_mode,
                dir_mode,
            )?
            && !Self::collect_file_entries_with_inferred_root(
                source_files,
                &mut entries,
                file_mode,
                dir_mode,
            )?
        {
            let mut emitted_dirs = HashSet::new();
            for source in source_files {
                if !source.is_dir() {
                    let data = std::fs::read(source)
                        .map_err(|e| format!("Cannot read {}: {}", source.display(), e))?;
                    let name = source
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown");
                    entries.push(ZipEntry::file(name, data, file_mode));
                    continue;
                }
                Self::collect_files(
                    source,
                    source,
                    &mut entries,
                    &mut emitted_dirs,
                    include_empty_dirs,
                    file_mode,
                    dir_mode,
                )?;
            }
        }

        if has_manifest_options(options) {
            let manifest = Self::create_manifest(options);
            entries.retain(|entry| !is_manifest_entry(&entry.name));
            let mut emitted_dirs = entries
                .iter()
                .filter(|entry| entry.is_dir())
                .map(|entry| entry.name.clone())
                .collect::<HashSet<_>>();
            push_zip_parent_dirs(
                &mut entries,
                &mut emitted_dirs,
                "META-INF/MANIFEST.MF",
                dir_mode,
            );
            entries.push(ZipEntry::file("META-INF/MANIFEST.MF", manifest, file_mode));
        }

        entries = resolve_duplicate_entries(entries, &archive_duplicate_strategy(options))?;
        entries.sort_unstable_by(|a, b| a.name.cmp(&b.name));

        let mut out = std::fs::File::create(jar_path)
            .map_err(|e| format!("Cannot create {}: {}", jar_path.display(), e))?;
        Self::write_zip(&mut out, &entries)
            .map_err(|e| format!("Cannot write {}: {}", jar_path.display(), e))?;

        // Track total bytes
        let total_bytes: u64 = entries.iter().map(|entry| entry.data.len() as u64).sum();
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
        let mut entries: Vec<ZipEntry> = if jar_path.exists() {
            Self::read_existing_entries(jar_path)
                .map_err(|e| format!("Cannot read {}: {}", jar_path.display(), e))?
        } else {
            Vec::new()
        };
        let include_empty_dirs = archive_include_empty_dirs(options);
        let file_mode = archive_file_mode(options);
        let dir_mode = archive_dir_mode(options);

        let mut mapped_entries = Vec::new();
        if Self::collect_mapped_entries(
            options,
            &mut mapped_entries,
            include_empty_dirs,
            file_mode,
            dir_mode,
        )? {
            for new_entry in mapped_entries {
                entries.retain(|entry| entry.name != new_entry.name);
                entries.push(new_entry);
            }
        } else {
            if Self::collect_file_entries_with_inferred_root(
                source_files,
                &mut mapped_entries,
                file_mode,
                dir_mode,
            )? {
                for new_entry in mapped_entries {
                    entries.retain(|entry| entry.name != new_entry.name);
                    entries.push(new_entry);
                }
            } else {
                let mut emitted_dirs = HashSet::new();
                for source in source_files {
                    if !source.is_dir() {
                        let data = std::fs::read(source)
                            .map_err(|e| format!("Cannot read {}: {}", source.display(), e))?;
                        let name = source
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("unknown");
                        entries.retain(|entry| entry.name != name);
                        entries.push(ZipEntry::file(name, data, file_mode));
                        continue;
                    }

                    let mut new_entries = Vec::new();
                    Self::collect_files(
                        source,
                        source,
                        &mut new_entries,
                        &mut emitted_dirs,
                        include_empty_dirs,
                        file_mode,
                        dir_mode,
                    )?;
                    for new_entry in new_entries {
                        entries.retain(|entry| entry.name != new_entry.name);
                        entries.push(new_entry);
                    }
                }
            }
        }

        if has_manifest_options(options) {
            let manifest = Self::create_manifest(options);
            entries.retain(|entry| !is_manifest_entry(&entry.name));
            entries.push(ZipEntry::file("META-INF/MANIFEST.MF", manifest, file_mode));
        }

        entries = resolve_duplicate_entries(entries, &archive_duplicate_strategy(options))?;
        entries.sort_unstable_by(|a, b| a.name.cmp(&b.name));

        let mut out = std::fs::File::create(jar_path)
            .map_err(|e| format!("Cannot create {}: {}", jar_path.display(), e))?;
        Self::write_zip(&mut out, &entries)
            .map_err(|e| format!("Cannot write {}: {}", jar_path.display(), e))?;

        Ok(())
    }
}

fn has_manifest_options(options: &std::collections::HashMap<String, String>) -> bool {
    options.contains_key("manifest")
        || options.contains_key("mainClass")
        || options.contains_key("classpath")
        || options.keys().any(|key| key.starts_with("manifest."))
}

fn is_manifest_entry(name: &str) -> bool {
    name.eq_ignore_ascii_case("META-INF/MANIFEST.MF")
}

fn common_file_parent(source_files: &[PathBuf]) -> Option<PathBuf> {
    let mut root = source_files.first()?.parent()?.to_path_buf();
    for source in source_files.iter().skip(1) {
        let parent = source.parent()?;
        while !parent.starts_with(&root) {
            if !root.pop() {
                return None;
            }
        }
    }
    Some(root)
}

fn looks_like_archive_source_root(root: &Path) -> bool {
    let components = root
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>();
    if components.len() < 2 {
        return false;
    }
    components.windows(2).any(|pair| {
        pair == ["src", "dist"] || pair == ["classes", "java"] || pair == ["resources", "main"]
    }) || components.iter().any(|component| *component == "src")
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
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
        input
            .options
            .insert("jarName".to_string(), "test.jar".to_string());

        let result = executor.execute(&input).await;

        assert!(
            result.success,
            "JAR creation failed: {}",
            result.error_message
        );
        assert!(result.output_files.iter().any(|p| p.ends_with("test.jar")));

        // Verify the JAR is a valid ZIP
        let jar_path = result.output_files.first().unwrap();
        let jar_data = fs::read(jar_path).unwrap();
        assert!(
            jar_data.starts_with(b"PK\x03\x04"),
            "JAR must start with ZIP local file header"
        );
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
        input
            .options
            .insert("jarName".to_string(), "app.jar".to_string());
        input
            .options
            .insert("mainClass".to_string(), "com.example.Main".to_string());

        let result = executor.execute(&input).await;

        assert!(
            result.success,
            "JAR creation failed: {}",
            result.error_message
        );

        let jar_path = result.output_files.first().unwrap();
        let jar_data = fs::read(jar_path).unwrap();
        let jar_str = String::from_utf8_lossy(&jar_data);
        assert!(jar_str.contains("Manifest-Version: 1.0"));
        assert!(jar_str.contains("Main-Class: com.example.Main"));
    }

    #[test]
    fn test_manifest_attributes_are_sorted_and_wrapped() {
        let mut options_a = std::collections::HashMap::new();
        options_a.insert("manifest.Zed".to_string(), "last".to_string());
        options_a.insert(
            "manifest.Long-Value".to_string(),
            "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz"
                .to_string(),
        );
        options_a.insert("manifest.Alpha".to_string(), "first".to_string());

        let mut options_b = std::collections::HashMap::new();
        options_b.insert("manifest.Alpha".to_string(), "first".to_string());
        options_b.insert("manifest.Zed".to_string(), "last".to_string());
        options_b.insert(
            "manifest.Long-Value".to_string(),
            "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz"
                .to_string(),
        );

        let manifest_a = JarTaskExecutor::create_manifest(&options_a);
        let manifest_b = JarTaskExecutor::create_manifest(&options_b);
        assert_eq!(manifest_a, manifest_b);

        let text = String::from_utf8(manifest_a).unwrap();
        assert!(
            text.find("Alpha: first").unwrap() < text.find("Long-Value:").unwrap()
                && text.find("Long-Value:").unwrap() < text.find("Zed: last").unwrap()
        );
        for line in text.split("\r\n").filter(|line| !line.is_empty()) {
            assert!(
                line.as_bytes().len() <= 72,
                "manifest line too long: {}",
                line
            );
        }
        assert!(text.contains("\r\n "));
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
        )
        .unwrap();

        let executor = JarTaskExecutor::new();
        let mut input = TaskInput::new("Jar");
        input.source_files.push(src_dir);
        input.target_dir = out_dir;
        input
            .options
            .insert("jarName".to_string(), "nested.jar".to_string());

        let result = executor.execute(&input).await;

        assert!(
            result.success,
            "JAR creation failed: {}",
            result.error_message
        );

        let jar_path = result.output_files.first().unwrap();
        let jar_data = fs::read(jar_path).unwrap();
        let jar_str = String::from_utf8_lossy(&jar_data);
        assert!(jar_str.contains("com/example/Service.class"));
    }

    #[tokio::test]
    async fn test_jar_includes_directory_entries_by_default() {
        let tmp = TempDir::new().unwrap();
        let src_dir = tmp.path().join("src");
        let out_dir = tmp.path().join("out");

        fs::create_dir_all(src_dir.join("empty/nested")).unwrap();

        let executor = JarTaskExecutor::new();
        let mut input = TaskInput::new("Jar");
        input.source_files.push(src_dir);
        input.target_dir = out_dir;
        input
            .options
            .insert("jarName".to_string(), "dirs.jar".to_string());

        let result = executor.execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        let entries = JarTaskExecutor::read_existing_entries(result.output_files.first().unwrap())
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        assert!(entries.contains(&"empty/".to_string()));
        assert!(entries.contains(&"empty/nested/".to_string()));

        let jar_bytes = fs::read(result.output_files.first().unwrap()).unwrap();
        let external_attrs = central_directory_external_attrs(&jar_bytes, "empty/").unwrap();
        assert_eq!(external_attrs & 0x10, 0x10);
    }

    #[tokio::test]
    async fn test_jar_can_skip_directory_entries() {
        let tmp = TempDir::new().unwrap();
        let src_dir = tmp.path().join("src");
        let out_dir = tmp.path().join("out");

        fs::create_dir_all(src_dir.join("empty")).unwrap();

        let executor = JarTaskExecutor::new();
        let mut input = TaskInput::new("Jar");
        input.source_files.push(src_dir);
        input.target_dir = out_dir;
        input
            .options
            .insert("jarName".to_string(), "dirs.jar".to_string());
        input
            .options
            .insert("include_empty_dirs".to_string(), "false".to_string());

        let result = executor.execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        let entries = JarTaskExecutor::read_existing_entries(result.output_files.first().unwrap())
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        assert!(!entries.contains(&"empty/".to_string()));
    }

    #[tokio::test]
    async fn test_jar_uses_explicit_copyspec_file_mappings() {
        let tmp = TempDir::new().unwrap();
        let src_dir = tmp.path().join("src");
        let out_dir = tmp.path().join("out");
        fs::create_dir_all(&src_dir).unwrap();
        let src_file = src_dir.join("app.txt");
        fs::write(&src_file, b"mapped").unwrap();

        let mapping = format!(
            "{}>{}>F",
            URL_SAFE_NO_PAD.encode(src_file.to_string_lossy().as_bytes()),
            URL_SAFE_NO_PAD.encode("nested/app.txt")
        );
        let executor = JarTaskExecutor::new();
        let mut input = TaskInput::new("Jar");
        input.target_dir = out_dir;
        input
            .options
            .insert("jarName".to_string(), "mapped.jar".to_string());
        input
            .options
            .insert("copy_file_mappings".to_string(), mapping);

        let result = executor.execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        let entries = JarTaskExecutor::read_existing_entries(result.output_files.first().unwrap())
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        assert!(entries.contains(&"nested/app.txt".to_string()));
        assert!(!entries.contains(&"app.txt".to_string()));
    }

    #[tokio::test]
    async fn test_convention_jar_collects_classes_when_mappings_only_have_directories() {
        let tmp = TempDir::new().unwrap();
        let build_dir = tmp.path().join("build");
        let classes_dir = build_dir.join("classes/java/main");
        let package_dir = classes_dir.join("com/example");
        let out_dir = build_dir.join("libs");
        fs::create_dir_all(&package_dir).unwrap();
        fs::write(package_dir.join("App.class"), b"class bytes").unwrap();

        let mapping = format!(
            "{}>{}>D",
            URL_SAFE_NO_PAD.encode(package_dir.to_string_lossy().as_bytes()),
            URL_SAFE_NO_PAD.encode("com/example")
        );
        let executor = JarTaskExecutor::new();
        let mut input = TaskInput::new("Jar");
        input.target_dir = out_dir;
        input
            .options
            .insert("jarName".to_string(), "app.jar".to_string());
        input
            .options
            .insert("copy_file_mappings".to_string(), mapping);

        let result = executor.execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        let entries = JarTaskExecutor::read_existing_entries(result.output_files.first().unwrap())
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        assert!(entries.contains(&"com/".to_string()));
        assert!(entries.contains(&"com/example/".to_string()));
        assert!(entries.contains(&"com/example/App.class".to_string()));
    }

    #[tokio::test]
    async fn test_spring_boot_archive_adds_boot_inf_classes_and_indexes() {
        let tmp = TempDir::new().unwrap();
        let build_dir = tmp.path().join("build");
        let classes_dir = build_dir.join("classes/java/main/com/example");
        let libs_dir = tmp.path().join("libs");
        let lib = libs_dir.join("demo.jar");
        fs::create_dir_all(&classes_dir).unwrap();
        fs::create_dir_all(&libs_dir).unwrap();
        fs::write(classes_dir.join("App.class"), b"class bytes").unwrap();
        fs::write(&lib, b"jar bytes").unwrap();

        let mapping = format!(
            "{}>{}>F",
            URL_SAFE_NO_PAD.encode(lib.to_string_lossy().as_bytes()),
            URL_SAFE_NO_PAD.encode("BOOT-INF/lib/demo.jar")
        );
        let executor = JarTaskExecutor::new();
        let mut input = TaskInput::new("Jar");
        input.target_dir = build_dir.join("libs");
        input
            .options
            .insert("jarName".to_string(), "app.jar".to_string());
        input
            .options
            .insert("copy_file_mappings".to_string(), mapping);

        let result = executor.execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        let entries = JarTaskExecutor::read_existing_entries(result.output_files.first().unwrap())
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        assert!(entries.contains(&"BOOT-INF/classes/com/example/App.class".to_string()));
        assert!(entries.contains(&"BOOT-INF/lib/demo.jar".to_string()));
        assert!(entries.contains(&"BOOT-INF/classpath.idx".to_string()));
        assert!(entries.contains(&"BOOT-INF/layers.idx".to_string()));
    }

    #[tokio::test]
    async fn test_ear_excludes_missing_generated_duplicate_descriptor() {
        let tmp = TempDir::new().unwrap();
        let app_dir = tmp.path().join("src/main/application");
        let out_dir = tmp.path().join("out");
        let descriptor = app_dir.join("META-INF/application.xml");
        let generated_descriptor = tmp.path().join("build/tmp/ear/application.xml");
        fs::create_dir_all(descriptor.parent().unwrap()).unwrap();
        fs::write(&descriptor, b"<application/>").unwrap();

        let mappings = [
            format!(
                "{}>{}>F",
                URL_SAFE_NO_PAD.encode(descriptor.to_string_lossy().as_bytes()),
                URL_SAFE_NO_PAD.encode("META-INF/application.xml")
            ),
            format!(
                "{}>{}>F",
                URL_SAFE_NO_PAD.encode(generated_descriptor.to_string_lossy().as_bytes()),
                URL_SAFE_NO_PAD.encode("META-INF/application.xml")
            ),
        ]
        .join(",");

        let executor = JarTaskExecutor::new();
        let mut input = TaskInput::new("Ear");
        input.target_dir = out_dir;
        input
            .options
            .insert("jarName".to_string(), "corpus.ear".to_string());
        input
            .options
            .insert("copy_file_mappings".to_string(), mappings);
        input
            .options
            .insert("duplicates_strategy".to_string(), "EXCLUDE".to_string());

        let result = executor.execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        let entries = JarTaskExecutor::read_existing_entries(result.output_files.first().unwrap())
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        assert_eq!(
            entries
                .iter()
                .filter(|entry| entry.as_str() == "META-INF/application.xml")
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn test_zip_infers_relative_root_for_file_only_archive_sources() {
        let tmp = TempDir::new().unwrap();
        let dist_dir = tmp.path().join("src/dist");
        let out_dir = tmp.path().join("out");
        let run_script = dist_dir.join("bin/run.sh");
        let config = dist_dir.join("conf/app.properties");
        fs::create_dir_all(run_script.parent().unwrap()).unwrap();
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&run_script, b"run").unwrap();
        fs::write(&config, b"name=app").unwrap();

        let executor = JarTaskExecutor::new();
        let mut input = TaskInput::new("Zip");
        input.source_files.push(run_script);
        input.source_files.push(config);
        input.target_dir = out_dir;
        input
            .options
            .insert("jarName".to_string(), "corpus.zip".to_string());
        input
            .options
            .insert("include_empty_dirs".to_string(), "false".to_string());

        let result = executor.execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        let entries = JarTaskExecutor::read_existing_entries(result.output_files.first().unwrap())
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        assert!(entries.contains(&"bin/".to_string()));
        assert!(entries.contains(&"bin/run.sh".to_string()));
        assert!(entries.contains(&"conf/".to_string()));
        assert!(entries.contains(&"conf/app.properties".to_string()));
        assert!(!entries.contains(&"run.sh".to_string()));
        assert!(!entries.contains(&"app.properties".to_string()));
    }

    #[tokio::test]
    async fn test_jar_synthesizes_missing_manifest_mapping_from_options() {
        let tmp = TempDir::new().unwrap();
        let src_dir = tmp.path().join("classes");
        let out_dir = tmp.path().join("out");
        let missing_manifest = tmp.path().join("build/tmp/jar/MANIFEST.MF");
        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("App.class"), b"class").unwrap();

        let mapping = format!(
            "{}>{}>F,{}>{}>F",
            URL_SAFE_NO_PAD.encode(src_dir.join("App.class").to_string_lossy().as_bytes()),
            URL_SAFE_NO_PAD.encode("App.class"),
            URL_SAFE_NO_PAD.encode(missing_manifest.to_string_lossy().as_bytes()),
            URL_SAFE_NO_PAD.encode("META-INF/MANIFEST.MF")
        );
        let executor = JarTaskExecutor::new();
        let mut input = TaskInput::new("Jar");
        input.target_dir = out_dir;
        input
            .options
            .insert("jarName".to_string(), "mapped.jar".to_string());
        input
            .options
            .insert("copy_file_mappings".to_string(), mapping);
        input
            .options
            .insert("manifest.Manifest-Version".to_string(), "1.0".to_string());
        input.options.insert(
            "manifest.Implementation-Title".to_string(),
            "demo".to_string(),
        );

        let result = executor.execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        let entries =
            JarTaskExecutor::read_existing_entries(result.output_files.first().unwrap()).unwrap();
        let manifest = entries
            .iter()
            .find(|entry| entry.name == "META-INF/MANIFEST.MF")
            .expect("manifest entry should be generated");
        let manifest_text = String::from_utf8(manifest.data.clone()).unwrap();
        assert!(manifest_text.contains("Manifest-Version: 1.0"));
        assert!(manifest_text.contains("Implementation-Title: demo"));
        assert!(entries.iter().any(|entry| entry.name == "App.class"));
    }

    #[tokio::test]
    async fn test_jar_applies_declared_entry_permissions() {
        let tmp = TempDir::new().unwrap();
        let src_dir = tmp.path().join("src");
        let out_dir = tmp.path().join("out");

        fs::create_dir_all(src_dir.join("bin")).unwrap();
        fs::write(src_dir.join("bin/app"), b"run").unwrap();

        let executor = JarTaskExecutor::new();
        let mut input = TaskInput::new("Jar");
        input.source_files.push(src_dir);
        input.target_dir = out_dir;
        input
            .options
            .insert("jarName".to_string(), "modes.jar".to_string());
        input
            .options
            .insert("file_permissions".to_string(), "493".to_string());
        input
            .options
            .insert("dir_permissions".to_string(), "448".to_string());

        let result = executor.execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        let jar_bytes = fs::read(result.output_files.first().unwrap()).unwrap();
        let file_attrs = central_directory_external_attrs(&jar_bytes, "bin/app").unwrap();
        let dir_attrs = central_directory_external_attrs(&jar_bytes, "bin/").unwrap();

        assert_eq!((file_attrs >> 16) & 0o777, 0o755);
        assert_eq!((dir_attrs >> 16) & 0o777, 0o700);
        assert_eq!(dir_attrs & 0x10, 0x10);
    }

    fn central_directory_external_attrs(jar: &[u8], entry_name: &str) -> Option<u32> {
        let mut pos = 0;
        while pos + 46 <= jar.len() {
            if jar[pos..pos + 4] != *b"PK\x01\x02" {
                pos += 1;
                continue;
            }
            let name_len = u16::from_le_bytes([jar[pos + 28], jar[pos + 29]]) as usize;
            let extra_len = u16::from_le_bytes([jar[pos + 30], jar[pos + 31]]) as usize;
            let comment_len = u16::from_le_bytes([jar[pos + 32], jar[pos + 33]]) as usize;
            let name_start = pos + 46;
            let name_end = name_start + name_len;
            if name_end > jar.len() {
                return None;
            }
            if &jar[name_start..name_end] == entry_name.as_bytes() {
                return Some(u32::from_le_bytes([
                    jar[pos + 38],
                    jar[pos + 39],
                    jar[pos + 40],
                    jar[pos + 41],
                ]));
            }
            pos = name_end + extra_len + comment_len;
        }
        None
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
        input
            .options
            .insert("jarName".to_string(), "update.jar".to_string());

        let result = executor.execute(&input).await;
        assert!(result.success);

        // Now update: add B.class
        fs::write(src_dir.join("B.class"), b"class B {}").unwrap();
        let mut update_input = TaskInput::new("Jar");
        update_input.source_files.push(src_dir);
        update_input.target_dir = out_dir.clone();
        update_input
            .options
            .insert("jarName".to_string(), "update.jar".to_string());
        update_input
            .options
            .insert("action".to_string(), "update".to_string());

        let result = executor.execute(&update_input).await;
        assert!(
            result.success,
            "JAR update failed: {}",
            result.error_message
        );

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
        input
            .options
            .insert("action".to_string(), "sign".to_string());
        input
            .options
            .insert("jarName".to_string(), "test.jar".to_string());

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
            input
                .options
                .insert("jarName".to_string(), "det.jar".to_string());

            let result = executor.execute(&input).await;
            assert!(result.success);
        }

        let jar1 = fs::read(out1.join("det.jar")).unwrap();
        let jar2 = fs::read(out2.join("det.jar")).unwrap();
        assert_eq!(jar1, jar2, "JARs with same content must be byte-identical");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_zip_file_symlink_follows_target_bytes_like_gradle() {
        let tmp = TempDir::new().unwrap();
        let src_dir = tmp.path().join("src");
        let out_dir = tmp.path().join("out");

        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("real.txt"), b"archive bytes").unwrap();
        std::os::unix::fs::symlink("real.txt", src_dir.join("link.txt")).unwrap();

        let executor = JarTaskExecutor::new();
        let mut input = TaskInput::new("Zip");
        input.source_files.push(src_dir);
        input.target_dir = out_dir;
        input
            .options
            .insert("archive_file_name".to_string(), "symlink.zip".to_string());

        let result = executor.execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        let file = fs::File::open(result.output_files.first().unwrap()).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        assert_eq!(
            archive
                .by_name("link.txt")
                .unwrap()
                .bytes()
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            b"archive bytes"
        );
        assert_eq!(
            archive
                .by_name("real.txt")
                .unwrap()
                .bytes()
                .collect::<Result<Vec<_>, _>>()
                .unwrap(),
            b"archive bytes"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_zip_directory_symlink_fails_closed() {
        let tmp = TempDir::new().unwrap();
        let src_dir = tmp.path().join("src");
        let target_dir = tmp.path().join("target");
        let out_dir = tmp.path().join("out");

        fs::create_dir_all(&src_dir).unwrap();
        fs::create_dir_all(&target_dir).unwrap();
        fs::write(target_dir.join("nested.txt"), b"nested").unwrap();
        std::os::unix::fs::symlink(&target_dir, src_dir.join("linked-dir")).unwrap();

        let executor = JarTaskExecutor::new();
        let mut input = TaskInput::new("Zip");
        input.source_files.push(src_dir);
        input.target_dir = out_dir;
        input.options.insert(
            "archive_file_name".to_string(),
            "symlink-dir.zip".to_string(),
        );

        let result = executor.execute(&input).await;

        assert!(!result.success);
        assert!(result.error_message.contains("Directory symlink inputs"));
    }

    #[tokio::test]
    async fn test_jar_uses_reproducible_zip_timestamp() {
        let tmp = TempDir::new().unwrap();
        let src_dir = tmp.path().join("src");
        let out_dir = tmp.path().join("out");

        fs::create_dir_all(&src_dir).unwrap();
        fs::write(src_dir.join("A.class"), b"a").unwrap();

        let executor = JarTaskExecutor::new();
        let mut input = TaskInput::new("Jar");
        input.source_files.push(src_dir);
        input.target_dir = out_dir;
        input
            .options
            .insert("jarName".to_string(), "timestamp.jar".to_string());

        let result = executor.execute(&input).await;

        assert!(result.success, "{}", result.error_message);
        let jar = fs::read(result.output_files.first().unwrap()).unwrap();
        assert_eq!(&jar[0..4], b"PK\x03\x04");
        assert_eq!(u16::from_le_bytes([jar[10], jar[11]]), 0);
        assert_eq!(u16::from_le_bytes([jar[12], jar[13]]), 33);
    }

    #[tokio::test]
    async fn test_jar_duplicate_strategy_fail_reports_duplicate_entry() {
        let tmp = TempDir::new().unwrap();
        let src_a = tmp.path().join("src-a");
        let src_b = tmp.path().join("src-b");
        let out_dir = tmp.path().join("out");

        fs::create_dir_all(&src_a).unwrap();
        fs::create_dir_all(&src_b).unwrap();
        fs::write(src_a.join("same.txt"), b"first").unwrap();
        fs::write(src_b.join("same.txt"), b"second").unwrap();

        let executor = JarTaskExecutor::new();
        let mut input = TaskInput::new("Jar");
        input.source_files.push(src_a);
        input.source_files.push(src_b);
        input.target_dir = out_dir;
        input
            .options
            .insert("jarName".to_string(), "dups.jar".to_string());
        input
            .options
            .insert("duplicates_strategy".to_string(), "FAIL".to_string());

        let result = executor.execute(&input).await;

        assert!(!result.success);
        assert!(result.error_message.contains("Duplicate archive entry"));
    }

    #[tokio::test]
    async fn test_jar_empty_sources() {
        let tmp = TempDir::new().unwrap();
        let out_dir = tmp.path().join("out");
        fs::create_dir_all(&out_dir).unwrap();

        let executor = JarTaskExecutor::new();
        let mut input = TaskInput::new("Jar");
        input.target_dir = out_dir.clone();
        input
            .options
            .insert("jarName".to_string(), "empty.jar".to_string());

        let result = executor.execute(&input).await;

        assert!(
            result.success,
            "Empty JAR creation failed: {}",
            result.error_message
        );
        assert!(out_dir.join("empty.jar").exists());

        // Empty JAR should still be a valid ZIP with just EOCD
        let jar_data = fs::read(out_dir.join("empty.jar")).unwrap();
        assert!(
            jar_data.starts_with(b"PK\x05\x06"),
            "Empty JAR must start with EOCD signature"
        );
    }
}
