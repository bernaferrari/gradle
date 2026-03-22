use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::fs;
use tonic::{Request, Response, Status};

use crate::proto::{
    garbage_collection_service_server::GarbageCollectionService, GcBuildCacheRequest,
    GcBuildCacheResponse, GcConfigCacheRequest, GcConfigCacheResponse,
    GcExecutionHistoryRequest, GcExecutionHistoryResponse, GetStorageStatsRequest,
    GetStorageStatsResponse, StorageStats,
};

/// Provides garbage collection for substrate-managed stores.
/// Evicts stale entries based on age, size, or access patterns.
pub struct GarbageCollectionServiceImpl {
    cache_dir: PathBuf,
    history_dir: PathBuf,
    config_cache_dir: PathBuf,
}

impl Default for GarbageCollectionServiceImpl {
    fn default() -> Self {
        Self::new(
            std::path::PathBuf::new(),
            std::path::PathBuf::new(),
            std::path::PathBuf::new(),
        )
    }
}

impl GarbageCollectionServiceImpl {
    pub fn new(
        cache_dir: PathBuf,
        history_dir: PathBuf,
        config_cache_dir: PathBuf,
    ) -> Self {
        Self {
            cache_dir,
            history_dir,
            config_cache_dir,
        }
    }

    fn now_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    async fn gc_directory(
        &self,
        dir: &Path,
        max_age_ms: i64,
        max_entries: Option<i32>,
        dry_run: bool,
        extension: &str,
    ) -> Result<(i32, i64, i32), Status> {
        if !dir.exists() {
            return Ok((0, 0, 0));
        }

        let now = Self::now_ms();
        let mut entries: Vec<(String, i64, i64)> = Vec::new(); // (path, mtime_ms, size_bytes)

        // Walk subdirectories too (build cache uses shard dirs)
        let mut dirs_to_scan = vec![dir.to_path_buf()];
        while let Some(scan_dir) = dirs_to_scan.pop() {
            let mut dir_entries = fs::read_dir(&scan_dir).await.map_err(|e| {
                Status::internal(format!("Failed to read directory {}: {}", scan_dir.display(), e))
            })?;

            while let Some(entry) = dir_entries.next_entry().await.map_err(|e| {
                Status::internal(format!("Failed to read directory entry: {}", e))
            })? {
                let path = entry.path();
                if path.is_dir() {
                    dirs_to_scan.push(path);
                    continue;
                }
                if let Some(name) = entry.file_name().to_str() {
                    if extension.is_empty() || name.ends_with(extension) {
                        let metadata = match fs::metadata(&path).await {
                            Ok(m) => m,
                            Err(_) => continue,
                        };
                        let mtime_ms = metadata
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                            .map(|d| d.as_millis() as i64)
                            .unwrap_or(0);
                        let size = metadata.len() as i64;
                        entries.push((path.to_string_lossy().to_string(), mtime_ms, size));
                    }
                }
            }
        }

        // Sort by modification time (oldest first) for LRU-style eviction
        entries.sort_by_key(|e| e.1);

        let mut to_remove = Vec::new();

        // Remove entries older than max_age_ms (max_age_ms == 0 means evict all)
        if max_age_ms >= 0 {
            for (path, mtime, _) in &entries {
                if max_age_ms == 0 || now - mtime > max_age_ms {
                    to_remove.push(path.clone());
                }
            }
        }

        // If we have too many entries, remove oldest ones beyond the limit
        if let Some(limit) = max_entries {
            let remaining = entries.len() as i32 - to_remove.len() as i32;
            if remaining > limit {
                let excess = (remaining - limit) as usize;
                for (path, _, _) in &entries {
                    if !to_remove.contains(path) && to_remove.len() < excess {
                        to_remove.push(path.clone());
                    }
                }
            }
        }

        let mut removed = 0i32;
        let mut bytes_recovered = 0i64;

        for (path, _, size) in &entries {
            if to_remove.contains(path) {
                if !dry_run {
                    fs::remove_file(path).await.ok();
                }
                removed += 1;
                bytes_recovered += size;
            }
        }

        let remaining = entries.len() as i32 - removed;
        Ok((removed, bytes_recovered, remaining))
    }

    async fn dir_total_bytes(&self, dir: &Path, extension: &str) -> Result<i64, Status> {
        if !dir.exists() {
            return Ok(0);
        }

        let mut total = 0i64;

        // Walk subdirectories too (build cache uses shard dirs)
        let mut dirs_to_scan = vec![dir.to_path_buf()];
        while let Some(scan_dir) = dirs_to_scan.pop() {
            let mut entries = match fs::read_dir(&scan_dir).await {
                Ok(e) => e,
                Err(_) => continue,
            };

            while let Some(entry) = entries.next_entry().await.ok().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    dirs_to_scan.push(path);
                } else if let Some(name) = entry.file_name().to_str() {
                    if extension.is_empty() || name.ends_with(extension) {
                        if let Ok(metadata) = fs::metadata(&path).await {
                            total += metadata.len() as i64;
                        }
                    }
                }
            }
        }

        Ok(total)
    }

    async fn dir_stats(&self, dir: &Path, extension: &str) -> Result<StorageStats, Status> {
        if !dir.exists() {
            return Ok(StorageStats {
                store_name: dir
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default(),
                entries: 0,
                total_bytes: 0,
                oldest_entry_ms: 0,
                newest_entry_ms: 0,
            });
        }

        let mut entries = 0i64;
        let mut total_bytes = 0i64;
        let mut oldest = i64::MAX;
        let mut newest = 0i64;

        // Walk subdirectories too (build cache uses shard dirs)
        let mut dirs_to_scan = vec![dir.to_path_buf()];
        while let Some(scan_dir) = dirs_to_scan.pop() {
            let mut dir_entries = fs::read_dir(&scan_dir).await.map_err(|e| {
                Status::internal(format!("Failed to read directory {}: {}", scan_dir.display(), e))
            })?;

            while let Some(entry) = dir_entries.next_entry().await.map_err(|e| {
                Status::internal(format!("Failed to read directory entry: {}", e))
            })? {
                let path = entry.path();
                if path.is_dir() {
                    dirs_to_scan.push(path);
                    continue;
                }
                if let Some(name) = entry.file_name().to_str() {
                    if extension.is_empty() || name.ends_with(extension) {
                        entries += 1;
                        let metadata = match fs::metadata(&path).await {
                            Ok(m) => m,
                            Err(_) => continue,
                        };
                        total_bytes += metadata.len() as i64;
                        if let Ok(mtime) = metadata.modified() {
                            if let Ok(dur) = mtime.duration_since(UNIX_EPOCH) {
                                let ms = dur.as_millis() as i64;
                                oldest = oldest.min(ms);
                                newest = newest.max(ms);
                            }
                        }
                    }
                }
            }
        }

        if oldest == i64::MAX {
            oldest = 0;
        }

        Ok(StorageStats {
            store_name: dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            entries,
            total_bytes,
            oldest_entry_ms: oldest,
            newest_entry_ms: newest,
        })
    }
}

#[tonic::async_trait]
impl GarbageCollectionService for GarbageCollectionServiceImpl {
    async fn gc_build_cache(
        &self,
        request: Request<GcBuildCacheRequest>,
    ) -> Result<Response<GcBuildCacheResponse>, Status> {
        let req = request.into_inner();
        let (removed, bytes_recovered, remaining) = self
            .gc_directory(
                &self.cache_dir,
                req.max_age_ms,
                None,
                req.dry_run,
                "",
            )
            .await?;

        let bytes_remaining = self.dir_total_bytes(&self.cache_dir, "").await?;

        tracing::info!(
            removed,
            bytes_recovered,
            bytes_remaining,
            remaining,
            dry_run = req.dry_run,
            "Build cache GC complete"
        );

        Ok(Response::new(GcBuildCacheResponse {
            entries_removed: removed,
            bytes_recovered,
            entries_remaining: remaining,
            bytes_remaining,
        }))
    }

    async fn gc_execution_history(
        &self,
        request: Request<GcExecutionHistoryRequest>,
    ) -> Result<Response<GcExecutionHistoryResponse>, Status> {
        let req = request.into_inner();
        let max_entries = if req.max_entries > 0 {
            Some(req.max_entries)
        } else {
            None
        };

        let (removed, bytes, remaining) = self
            .gc_directory(
                &self.history_dir,
                req.max_age_ms,
                max_entries,
                req.dry_run,
                ".bin",
            )
            .await?;

        tracing::info!(
            removed,
            bytes_recovered = bytes,
            remaining,
            dry_run = req.dry_run,
            "Execution history GC complete"
        );

        Ok(Response::new(GcExecutionHistoryResponse {
            entries_removed: removed,
            bytes_recovered: bytes,
            entries_remaining: remaining,
        }))
    }

    async fn gc_config_cache(
        &self,
        request: Request<GcConfigCacheRequest>,
    ) -> Result<Response<GcConfigCacheResponse>, Status> {
        let req = request.into_inner();
        let max_entries = if req.max_entries > 0 {
            Some(req.max_entries)
        } else {
            None
        };

        let (removed, bytes, remaining) = self
            .gc_directory(
                &self.config_cache_dir,
                req.max_age_ms,
                max_entries,
                req.dry_run,
                ".bin",
            )
            .await?;

        tracing::info!(
            removed,
            bytes_recovered = bytes,
            remaining,
            dry_run = req.dry_run,
            "Configuration cache GC complete"
        );

        Ok(Response::new(GcConfigCacheResponse {
            entries_removed: removed,
            bytes_recovered: bytes,
            entries_remaining: remaining,
        }))
    }

    async fn get_storage_stats(
        &self,
        request: Request<GetStorageStatsRequest>,
    ) -> Result<Response<GetStorageStatsResponse>, Status> {
        let _req = request.into_inner();

        let cache_stats = self.dir_stats(&self.cache_dir, "").await?;
        let history_stats = self.dir_stats(&self.history_dir, ".bin").await?;
        let config_stats = self.dir_stats(&self.config_cache_dir, ".bin").await?;

        Ok(Response::new(GetStorageStatsResponse {
            stats: vec![cache_stats, history_stats, config_stats],
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_gc_by_age() {
        let dir = tempdir().unwrap();

        // Create some files
        for i in 0..5 {
            let path = dir.path().join(format!("entry{}.bin", i));
            tokio::fs::write(&path, vec![0u8; 100])
                .await
                .unwrap();
        }

        let svc = GarbageCollectionServiceImpl::new(
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
        );

        // GC with max_age=0 should remove all entries
        let resp = svc
            .gc_execution_history(Request::new(GcExecutionHistoryRequest {
                max_age_ms: 0,
                max_entries: 0,
                dry_run: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.entries_removed, 5);
        assert_eq!(resp.bytes_recovered, 500);
        assert_eq!(resp.entries_remaining, 0);
    }

    #[tokio::test]
    async fn test_gc_dry_run() {
        let dir = tempdir().unwrap();

        for i in 0..3 {
            let path = dir.path().join(format!("entry{}.bin", i));
            tokio::fs::write(&path, vec![0u8; 50])
                .await
                .unwrap();
        }

        let svc = GarbageCollectionServiceImpl::new(
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
        );

        let resp = svc
            .gc_execution_history(Request::new(GcExecutionHistoryRequest {
                max_age_ms: 0,
                max_entries: 0,
                dry_run: true,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.entries_removed, 3);
        assert_eq!(resp.bytes_recovered, 150);
        // But files still exist
        assert_eq!(resp.entries_remaining, 0);
        assert!(dir.path().join("entry0.bin").exists());
    }

    #[tokio::test]
    async fn test_gc_by_max_entries() {
        let dir = tempdir().unwrap();

        for i in 0..5 {
            let path = dir.path().join(format!("entry{}.bin", i));
            tokio::fs::write(&path, vec![0u8; 100])
                .await
                .unwrap();
        }

        let svc = GarbageCollectionServiceImpl::new(
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
        );

        // Use max_age_ms=-1 (don't evict by age) and max_entries=2
        let resp = svc
            .gc_execution_history(Request::new(GcExecutionHistoryRequest {
                max_age_ms: -1, // don't evict by age
                max_entries: 2, // keep only 2
                dry_run: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.entries_removed, 3);
        assert_eq!(resp.entries_remaining, 2);
    }

    #[tokio::test]
    async fn test_gc_nonexistent_dir() {
        let svc = GarbageCollectionServiceImpl::new(
            std::path::PathBuf::from("/nonexistent/path/that/does/not/exist"),
            std::path::PathBuf::from("/nonexistent/path/that/does/not/exist"),
            std::path::PathBuf::from("/nonexistent/path/that/does/not/exist"),
        );

        let resp = svc
            .gc_build_cache(Request::new(GcBuildCacheRequest {
                max_age_ms: 0,
                max_total_bytes: 0,
                dry_run: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.entries_removed, 0);
        assert_eq!(resp.entries_remaining, 0);
    }

    #[tokio::test]
    async fn test_gc_build_cache_all_extensions() {
        let dir = tempdir().unwrap();

        // Build cache uses no extension filter (empty string)
        tokio::fs::write(dir.path().join("entry1.bin"), vec![0u8; 50])
            .await.unwrap();
        tokio::fs::write(dir.path().join("entry2.dat"), vec![0u8; 75])
            .await.unwrap();
        tokio::fs::write(dir.path().join("entry3.txt"), vec![0u8; 25])
            .await.unwrap();

        let svc = GarbageCollectionServiceImpl::new(
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
        );

        let resp = svc
            .gc_build_cache(Request::new(GcBuildCacheRequest {
                max_age_ms: 0,
                max_total_bytes: 0,
                dry_run: false,
            }))
            .await
            .unwrap()
            .into_inner();

        // All files should be removed (no extension filter)
        assert_eq!(resp.entries_removed, 3);
        assert_eq!(resp.bytes_recovered, 150);
    }

    #[tokio::test]
    async fn test_gc_subdirectory_scanning() {
        let dir = tempdir().unwrap();

        // Create subdirectory structure (simulates build cache shards)
        let shard_dir = dir.path().join("ab");
        tokio::fs::create_dir_all(&shard_dir).await.unwrap();
        tokio::fs::write(shard_dir.join("entry1.bin"), vec![0u8; 100])
            .await.unwrap();
        tokio::fs::write(dir.path().join("entry2.bin"), vec![0u8; 200])
            .await.unwrap();

        let svc = GarbageCollectionServiceImpl::new(
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
        );

        let resp = svc
            .gc_execution_history(Request::new(GcExecutionHistoryRequest {
                max_age_ms: 0,
                max_entries: 0,
                dry_run: false,
            }))
            .await
            .unwrap()
            .into_inner();

        // Should find both files (root + subdirectory)
        assert_eq!(resp.entries_removed, 2);
        assert_eq!(resp.bytes_recovered, 300);
    }

    #[tokio::test]
    async fn test_storage_stats() {
        let dir = tempdir().unwrap();

        tokio::fs::write(dir.path().join("a.bin"), vec![0u8; 100])
            .await
            .unwrap();
        tokio::fs::write(dir.path().join("b.bin"), vec![0u8; 200])
            .await
            .unwrap();

        let svc = GarbageCollectionServiceImpl::new(
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
        );

        let resp = svc
            .get_storage_stats(Request::new(GetStorageStatsRequest {
                store_names: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.stats.len(), 3); // cache, history, config_cache

        // History stats should show 2 entries, 300 bytes
        let history = &resp.stats[1];
        assert_eq!(history.entries, 2);
        assert_eq!(history.total_bytes, 300);
    }

    #[tokio::test]
    async fn test_gc_config_cache() {
        let dir = tempdir().unwrap();

        for i in 0..4 {
            let path = dir.path().join(format!("config{}.bin", i));
            tokio::fs::write(&path, vec![0u8; 80])
                .await
                .unwrap();
        }
        // Also a non-.bin file that should be ignored
        tokio::fs::write(dir.path().join("readme.txt"), vec![0u8; 50])
            .await
            .unwrap();

        let svc = GarbageCollectionServiceImpl::new(
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
            dir.path().to_path_buf(),
        );

        // Use max_age_ms=-1 (don't evict by age) and max_entries=2
        let resp = svc
            .gc_config_cache(Request::new(GcConfigCacheRequest {
                max_age_ms: -1,
                max_entries: 2,
                dry_run: false,
            }))
            .await
            .unwrap()
            .into_inner();

        // Should only evict .bin files, keep at most 2
        assert_eq!(resp.entries_removed, 2);
        assert_eq!(resp.entries_remaining, 2);
    }
}
