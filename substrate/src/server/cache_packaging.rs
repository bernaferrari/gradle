use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{Cursor, Read, Write};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::{Compression, GzBuilder};
use tonic::{Request, Response, Status};

use crate::proto::{
    build_cache_packaging_service_server::BuildCachePackagingService, BuildCachePackFile,
    PackCacheEntryRequest, PackCacheEntryResponse, UnpackCacheEntryRequest,
    UnpackCacheEntryResponse,
};

const METADATA_ENTRY: &str = "METADATA";
const DEFAULT_TREE_NAME: &str = "output";
const LEGACY_TREE_PREFIX: &str = "tree/";

#[derive(Default)]
pub struct BuildCachePackagingServiceImpl;

fn normalize_relative_path(path: &str) -> Result<String, String> {
    let path = path.replace('\\', "/");
    let mut parts = Vec::new();
    for part in path.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(format!("unsafe relative path: {path}"));
        }
        if part.contains('\0') {
            return Err("path contains NUL byte".to_string());
        }
        parts.push(part);
    }
    if parts.is_empty() {
        return Err("path is empty".to_string());
    }
    Ok(parts.join("/"))
}

fn encode_metadata(metadata: &HashMap<String, String>) -> Vec<u8> {
    let sorted: BTreeMap<_, _> = metadata.iter().collect();
    let mut encoded = String::new();
    for (key, value) in sorted {
        encoded.push_str(key);
        encoded.push('=');
        encoded.push_str(value);
        encoded.push('\n');
    }
    encoded.into_bytes()
}

fn decode_metadata(bytes: &[u8]) -> HashMap<String, String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

fn append_file(
    builder: &mut tar::Builder<&mut Vec<u8>>,
    path: &str,
    content: &[u8],
    mode: u32,
) -> Result<(), std::io::Error> {
    let mut header = tar::Header::new_gnu();
    header.set_size(content.len() as u64);
    header.set_mode(mode);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_cksum();
    builder.append_data(&mut header, path, Cursor::new(content))
}

fn append_directory(
    builder: &mut tar::Builder<&mut Vec<u8>>,
    path: &str,
    mode: u32,
) -> Result<(), std::io::Error> {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Directory);
    header.set_size(0);
    header.set_mode(mode);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_cksum();
    builder.append_data(&mut header, path, Cursor::new(Vec::<u8>::new()))
}

fn escape_tree_name(name: &str) -> String {
    let mut encoded = String::new();
    for byte in name.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-' | b'*' | b'_' => {
                encoded.push(*byte as char)
            }
            b' ' => encoded.push('+'),
            _ => {
                encoded.push('%');
                encoded.push_str(&format!("{:02X}", byte));
            }
        }
    }
    encoded
}

fn tree_root_entry() -> String {
    format!("tree-{}/", escape_tree_name(DEFAULT_TREE_NAME))
}

fn tree_child_entry(path: &str) -> String {
    format!("{}{}", tree_root_entry(), path)
}

fn parent_directories(path: &str) -> Vec<String> {
    let mut parents = Vec::new();
    let mut parts = path.split('/').collect::<Vec<_>>();
    parts.pop();
    let mut current = String::new();
    for part in parts {
        if current.is_empty() {
            current.push_str(part);
        } else {
            current.push('/');
            current.push_str(part);
        }
        parents.push(current.clone());
    }
    parents
}

fn gzip_bytes(bytes: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    let mut encoder: GzEncoder<Vec<u8>> = GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), Compression::default());
    encoder.write_all(bytes)?;
    encoder.finish()
}

fn gunzip_bytes(bytes: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    let mut decoder = GzDecoder::new(Cursor::new(bytes));
    let mut decoded = Vec::new();
    decoder.read_to_end(&mut decoded)?;
    Ok(decoded)
}

impl BuildCachePackagingServiceImpl {
    fn pack(req: PackCacheEntryRequest) -> Result<(Vec<u8>, i64), String> {
        let mut files = Vec::with_capacity(req.files.len());
        for file in req.files {
            let path = normalize_relative_path(&file.path)?;
            files.push(BuildCachePackFile { path, ..file });
        }
        files.sort_unstable_by(|left, right| left.path.cmp(&right.path));

        let mut tar_bytes = Vec::new();
        let mut entry_count = 0_i64;
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            append_file(
                &mut builder,
                METADATA_ENTRY,
                &encode_metadata(&req.origin_metadata),
                0o644,
            )
            .map_err(|e| format!("append metadata: {e}"))?;
            entry_count += 1;

            let tree_root = tree_root_entry();
            append_directory(&mut builder, &tree_root, 0o755)
                .map_err(|e| format!("append {tree_root}: {e}"))?;
            entry_count += 1;

            let mut parent_dirs = BTreeSet::new();
            for file in &files {
                parent_dirs.extend(parent_directories(&file.path));
            }
            for dir in parent_dirs {
                let entry_path = tree_child_entry(&format!("{dir}/"));
                append_directory(&mut builder, &entry_path, 0o755)
                    .map_err(|e| format!("append {entry_path}: {e}"))?;
                entry_count += 1;
            }

            for file in &files {
                let entry_path = tree_child_entry(&file.path);
                let mode = if file.executable { 0o755 } else { 0o644 };
                append_file(&mut builder, &entry_path, &file.content, mode)
                    .map_err(|e| format!("append {}: {e}", file.path))?;
                entry_count += 1;
            }
            builder.finish().map_err(|e| format!("finish tar: {e}"))?;
        }

        let packaged = if req.gzip {
            gzip_bytes(&tar_bytes).map_err(|e| format!("gzip: {e}"))?
        } else {
            tar_bytes
        };
        Ok((packaged, entry_count))
    }

    fn unpack(
        req: UnpackCacheEntryRequest,
    ) -> Result<(Vec<BuildCachePackFile>, HashMap<String, String>, i64), String> {
        let tar_bytes = if req.gzip {
            gunzip_bytes(&req.packaged_bytes).map_err(|e| format!("gunzip: {e}"))?
        } else {
            req.packaged_bytes
        };

        let mut archive = tar::Archive::new(Cursor::new(tar_bytes));
        let mut files = Vec::new();
        let mut metadata = HashMap::new();
        let mut entry_count = 0_i64;
        let entries = archive.entries().map_err(|e| format!("read tar: {e}"))?;
        for entry in entries {
            let mut entry = entry.map_err(|e| format!("read tar entry: {e}"))?;
            entry_count += 1;
            let entry_path = entry
                .path()
                .map_err(|e| format!("read tar path: {e}"))?
                .to_string_lossy()
                .replace('\\', "/");
            if !entry.header().entry_type().is_file() {
                continue;
            }
            let mode = entry.header().mode().unwrap_or(0);
            let mut content = Vec::new();
            entry
                .read_to_end(&mut content)
                .map_err(|e| format!("read {entry_path}: {e}"))?;

            if entry_path == METADATA_ENTRY {
                metadata = decode_metadata(&content);
                continue;
            }
            if let Some(relative) = entry_path.strip_prefix(LEGACY_TREE_PREFIX) {
                files.push(BuildCachePackFile {
                    path: normalize_relative_path(relative)?,
                    content,
                    executable: mode & 0o111 != 0,
                });
                continue;
            }

            if entry_path.starts_with("missing-tree-") {
                continue;
            }

            if let Some((tree_root, relative)) = entry_path.split_once('/') {
                if tree_root.starts_with("tree-") && !relative.is_empty() {
                    files.push(BuildCachePackFile {
                        path: normalize_relative_path(relative)?,
                        content,
                        executable: mode & 0o111 != 0,
                    });
                }
            }
        }
        files.sort_unstable_by(|left, right| left.path.cmp(&right.path));
        Ok((files, metadata, entry_count))
    }
}

#[tonic::async_trait]
impl BuildCachePackagingService for BuildCachePackagingServiceImpl {
    async fn pack_cache_entry(
        &self,
        request: Request<PackCacheEntryRequest>,
    ) -> Result<Response<PackCacheEntryResponse>, Status> {
        let req = request.into_inner();
        let build_id = req.build_id.clone();
        match Self::pack(req) {
            Ok((packaged_bytes, entry_count)) => {
                tracing::debug!(
                    target: "gradle_substrate::cache_packaging",
                    build_id = %build_id,
                    bytes = packaged_bytes.len(),
                    entry_count,
                    "Packed build cache entry"
                );
                Ok(Response::new(PackCacheEntryResponse {
                    success: true,
                    packaged_bytes,
                    error: String::new(),
                    entry_count,
                }))
            }
            Err(error) => Ok(Response::new(PackCacheEntryResponse {
                success: false,
                packaged_bytes: Vec::new(),
                error,
                entry_count: 0,
            })),
        }
    }

    async fn unpack_cache_entry(
        &self,
        request: Request<UnpackCacheEntryRequest>,
    ) -> Result<Response<UnpackCacheEntryResponse>, Status> {
        let req = request.into_inner();
        let build_id = req.build_id.clone();
        match Self::unpack(req) {
            Ok((files, origin_metadata, entry_count)) => {
                tracing::debug!(
                    target: "gradle_substrate::cache_packaging",
                    build_id = %build_id,
                    files = files.len(),
                    entry_count,
                    "Unpacked build cache entry"
                );
                Ok(Response::new(UnpackCacheEntryResponse {
                    success: true,
                    files,
                    origin_metadata,
                    error: String::new(),
                    entry_count,
                }))
            }
            Err(error) => Ok(Response::new(UnpackCacheEntryResponse {
                success: false,
                files: Vec::new(),
                origin_metadata: HashMap::new(),
                error,
                entry_count: 0,
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tar_entry_names(packaged_bytes: &[u8], gzip: bool) -> Vec<String> {
        let tar_bytes = if gzip {
            gunzip_bytes(packaged_bytes).unwrap()
        } else {
            packaged_bytes.to_vec()
        };
        let mut archive = tar::Archive::new(Cursor::new(tar_bytes));
        archive
            .entries()
            .unwrap()
            .map(|entry| {
                entry
                    .unwrap()
                    .path()
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect()
    }

    fn pack_request(files: Vec<BuildCachePackFile>, gzip: bool) -> PackCacheEntryRequest {
        PackCacheEntryRequest {
            build_id: "build-1".to_string(),
            files,
            origin_metadata: [
                ("identity".to_string(), ":compileJava".to_string()),
                ("buildInvocationId".to_string(), "invocation-1".to_string()),
            ]
            .into_iter()
            .collect(),
            gzip,
        }
    }

    #[tokio::test]
    async fn pack_is_deterministic() {
        let service = BuildCachePackagingServiceImpl;
        let files_a = vec![
            BuildCachePackFile {
                path: "b.txt".to_string(),
                content: b"b".to_vec(),
                executable: false,
            },
            BuildCachePackFile {
                path: "a.txt".to_string(),
                content: b"a".to_vec(),
                executable: true,
            },
        ];
        let files_b = vec![files_a[1].clone(), files_a[0].clone()];

        let first = service
            .pack_cache_entry(Request::new(pack_request(files_a, true)))
            .await
            .unwrap()
            .into_inner();
        let second = service
            .pack_cache_entry(Request::new(pack_request(files_b, true)))
            .await
            .unwrap()
            .into_inner();

        assert!(first.success, "pack failed: {}", first.error);
        assert_eq!(first.packaged_bytes, second.packaged_bytes);
        assert_eq!(first.entry_count, 4);
    }

    #[tokio::test]
    async fn pack_uses_gradle_tree_entry_layout() {
        let service = BuildCachePackagingServiceImpl;
        let packed = service
            .pack_cache_entry(Request::new(pack_request(
                vec![BuildCachePackFile {
                    path: "classes/App.class".to_string(),
                    content: vec![0xca, 0xfe],
                    executable: false,
                }],
                true,
            )))
            .await
            .unwrap()
            .into_inner();

        assert!(packed.success, "pack failed: {}", packed.error);
        assert_eq!(
            tar_entry_names(&packed.packaged_bytes, true),
            vec![
                "METADATA".to_string(),
                "tree-output/".to_string(),
                "tree-output/classes/".to_string(),
                "tree-output/classes/App.class".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn unpack_round_trips_files_metadata_and_modes() {
        let service = BuildCachePackagingServiceImpl;
        let packed = service
            .pack_cache_entry(Request::new(pack_request(
                vec![
                    BuildCachePackFile {
                        path: "dir/run.sh".to_string(),
                        content: b"#!/bin/sh\n".to_vec(),
                        executable: true,
                    },
                    BuildCachePackFile {
                        path: "classes/App.class".to_string(),
                        content: vec![0xca, 0xfe],
                        executable: false,
                    },
                ],
                true,
            )))
            .await
            .unwrap()
            .into_inner();
        assert!(packed.success, "pack failed: {}", packed.error);

        let unpacked = service
            .unpack_cache_entry(Request::new(UnpackCacheEntryRequest {
                build_id: "build-1".to_string(),
                packaged_bytes: packed.packaged_bytes,
                gzip: true,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(unpacked.success, "unpack failed: {}", unpacked.error);
        assert_eq!(unpacked.entry_count, 6);
        assert_eq!(unpacked.origin_metadata["identity"], ":compileJava");
        assert_eq!(unpacked.files.len(), 2);
        assert_eq!(unpacked.files[0].path, "classes/App.class");
        assert_eq!(unpacked.files[0].content, vec![0xca, 0xfe]);
        assert!(!unpacked.files[0].executable);
        assert_eq!(unpacked.files[1].path, "dir/run.sh");
        assert!(unpacked.files[1].executable);
    }

    #[tokio::test]
    async fn pack_rejects_parent_directory_paths() {
        let service = BuildCachePackagingServiceImpl;
        let response = service
            .pack_cache_entry(Request::new(pack_request(
                vec![BuildCachePackFile {
                    path: "../outside.txt".to_string(),
                    content: b"bad".to_vec(),
                    executable: false,
                }],
                false,
            )))
            .await
            .unwrap()
            .into_inner();

        assert!(!response.success);
        assert!(response.error.contains("unsafe relative path"));
    }
}
