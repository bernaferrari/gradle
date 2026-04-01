use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use dashmap::DashMap;
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};

use crate::server::atomic_write::AtomicWriteError;

#[derive(Debug, thiserror::Error, Clone)]
pub enum CorruptionError {
    #[error("Checksum mismatch for key {key}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        key: String,
        expected: String,
        actual: String,
    },
    #[error("Deserialization error for key {key}: {reason}")]
    DeserializationError { key: String, reason: String },
    #[error(
        "Truncated entry for key {key}: expected {expected_size} bytes, got {actual_size} bytes"
    )]
    TruncatedEntry {
        key: String,
        expected_size: u64,
        actual_size: u64,
    },
    #[error("Missing metadata for key {key}")]
    MissingMetadata { key: String },
    #[error("Quarantine failed for key {key}: {reason}")]
    QuarantineFailed { key: String, reason: String },
}

impl From<AtomicWriteError> for CorruptionError {
    fn from(e: AtomicWriteError) -> Self {
        CorruptionError::QuarantineFailed {
            key: String::new(),
            reason: e.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CorruptionReport {
    pub key: String,
    pub error: CorruptionError,
    pub auto_quarantined: bool,
}

#[derive(Debug, Clone)]
pub struct ScanReport {
    pub total_entries: usize,
    pub valid_entries: usize,
    pub corrupted_entries: Vec<CorruptionReport>,
    pub quarantined_entries: Vec<String>,
    pub scan_time_ms: u64,
}

#[derive(Debug)]
pub enum EntryIntegrity {
    Valid { checksum: String, size: u64 },
    Corrupted(CorruptionError),
    Missing,
}

pub struct CorruptionScanner {
    cache_dir: PathBuf,
    quarantine_dir: PathBuf,
    checksum_index: DashMap<String, String>,
}

impl CorruptionScanner {
    pub fn new(cache_dir: PathBuf, quarantine_dir: PathBuf) -> Self {
        Self {
            cache_dir,
            quarantine_dir,
            checksum_index: DashMap::new(),
        }
    }

    pub fn register_expected_checksum(&self, key: String, checksum: String) {
        self.checksum_index.insert(key, checksum);
    }

    pub fn scan(&self) -> Result<ScanReport, CorruptionError> {
        let start = Instant::now();
        let mut total_entries = 0;
        let mut valid_entries = 0;
        let mut corrupted_entries = Vec::new();
        let mut quarantined_entries = Vec::new();

        if self.cache_dir.exists() {
            for entry in
                fs::read_dir(&self.cache_dir).map_err(|e| CorruptionError::QuarantineFailed {
                    key: String::new(),
                    reason: format!("Failed to read cache dir: {}", e),
                })?
            {
                let entry = entry.map_err(|e| CorruptionError::QuarantineFailed {
                    key: String::new(),
                    reason: format!("Failed to read entry: {}", e),
                })?;
                if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                    continue;
                }
                let shard_path = entry.path();
                for file_entry in
                    fs::read_dir(&shard_path).map_err(|e| CorruptionError::QuarantineFailed {
                        key: String::new(),
                        reason: format!("Failed to read shard: {}", e),
                    })?
                {
                    let file_entry = file_entry.map_err(|e| CorruptionError::QuarantineFailed {
                        key: String::new(),
                        reason: format!("Failed to read file entry: {}", e),
                    })?;
                    if !file_entry
                        .file_type()
                        .map(|ft| ft.is_file())
                        .unwrap_or(false)
                    {
                        continue;
                    }

                    let file_path = file_entry.path();
                    let key = file_path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if key.is_empty() {
                        continue;
                    }

                    let full_key = format!(
                        "{}{}",
                        shard_path
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default(),
                        key
                    );

                    total_entries += 1;
                    match self.scan_entry(&full_key) {
                        Ok(EntryIntegrity::Valid { .. }) => {
                            valid_entries += 1;
                        }
                        Ok(EntryIntegrity::Corrupted(err)) => {
                            let auto_quarantined = self.auto_quarantine(&full_key, &err).is_ok();
                            corrupted_entries.push(CorruptionReport {
                                key: full_key,
                                error: err,
                                auto_quarantined,
                            });
                        }
                        Ok(EntryIntegrity::Missing) => {
                            total_entries -= 1;
                        }
                        Err(err) => {
                            let auto_quarantined = self.auto_quarantine(&full_key, &err).is_ok();
                            corrupted_entries.push(CorruptionReport {
                                key: full_key,
                                error: err,
                                auto_quarantined,
                            });
                        }
                    }
                }
            }
        }

        if self.quarantine_dir.exists() {
            for entry in fs::read_dir(&self.quarantine_dir).map_err(|e| {
                CorruptionError::QuarantineFailed {
                    key: String::new(),
                    reason: format!("Failed to read quarantine dir: {}", e),
                }
            })? {
                let entry = entry.map_err(|e| CorruptionError::QuarantineFailed {
                    key: String::new(),
                    reason: format!("Failed to read quarantine entry: {}", e),
                })?;
                if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                    let key = entry.file_name().to_string_lossy().to_string();
                    quarantined_entries.push(key);
                }
            }
        }

        let scan_time_ms = start.elapsed().as_millis() as u64;

        Ok(ScanReport {
            total_entries,
            valid_entries,
            corrupted_entries,
            quarantined_entries,
            scan_time_ms,
        })
    }

    pub fn scan_entry(&self, key: &str) -> Result<EntryIntegrity, CorruptionError> {
        let key_path = self.key_to_path(key);

        if !key_path.exists() {
            return Ok(EntryIntegrity::Missing);
        }

        let metadata = fs::metadata(&key_path).map_err(|_e| CorruptionError::MissingMetadata {
            key: key.to_string(),
        })?;

        let actual_size = metadata.len();
        let data = fs::read(&key_path).map_err(|e| CorruptionError::DeserializationError {
            key: key.to_string(),
            reason: e.to_string(),
        })?;

        let mut hasher = Sha256::new();
        hasher.update(&data);
        let actual_checksum = format!("{:x}", hasher.finalize());

        if let Some(expected) = self.checksum_index.get(key) {
            if *expected != actual_checksum {
                return Ok(EntryIntegrity::Corrupted(
                    CorruptionError::ChecksumMismatch {
                        key: key.to_string(),
                        expected: expected.clone(),
                        actual: actual_checksum,
                    },
                ));
            }
        }

        Ok(EntryIntegrity::Valid {
            checksum: actual_checksum,
            size: actual_size,
        })
    }

    fn auto_quarantine(&self, key: &str, _err: &CorruptionError) -> Result<(), CorruptionError> {
        let key_path = self.key_to_path(key);
        if !key_path.exists() {
            return Ok(());
        }

        let quarantine_subdir = self.quarantine_dir.join(key);
        fs::create_dir_all(&quarantine_subdir).map_err(|e| CorruptionError::QuarantineFailed {
            key: key.to_string(),
            reason: format!("Failed to create quarantine subdir: {}", e),
        })?;

        let quarantine_path = quarantine_subdir.join("data");
        fs::rename(&key_path, &quarantine_path).map_err(|e| CorruptionError::QuarantineFailed {
            key: key.to_string(),
            reason: format!("Failed to move to quarantine: {}", e),
        })?;

        let quarantined_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let original_size = fs::metadata(&quarantine_path).map(|m| m.len()).unwrap_or(0);

        let meta_content = format!(
            "key={}\nreason=auto-quarantined\nquarantined_at_ms={}\noriginal_size={}\noriginal_path={}\n",
            key,
            quarantined_at_ms,
            original_size,
            key_path.display(),
        );
        let meta_path = quarantine_subdir.join("metadata.txt");
        fs::write(&meta_path, meta_content).map_err(|e| CorruptionError::QuarantineFailed {
            key: key.to_string(),
            reason: format!("Failed to write quarantine metadata: {}", e),
        })?;

        Ok(())
    }

    fn key_to_path(&self, key: &str) -> PathBuf {
        let trimmed = key.trim();
        if trimmed.len() > 2 {
            self.cache_dir.join(&trimmed[..2]).join(&trimmed[2..])
        } else {
            self.cache_dir.join(trimmed)
        }
    }
}

#[derive(Debug)]
pub struct ChecksummedCacheEntry<T> {
    pub data: T,
    pub checksum: String,
    pub serialized_size: u64,
    pub created_at_ms: i64,
}

impl<T: Serialize + DeserializeOwned> ChecksummedCacheEntry<T> {
    pub fn wrap(data: T) -> Result<Self, CorruptionError> {
        let serialized =
            bincode::serialize(&data).map_err(|e| CorruptionError::DeserializationError {
                key: String::new(),
                reason: format!("Failed to serialize: {}", e),
            })?;

        let serialized_size = serialized.len() as u64;

        let mut hasher = Sha256::new();
        hasher.update(&serialized);
        let checksum = format!("{:x}", hasher.finalize());

        let created_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        Ok(Self {
            data,
            checksum,
            serialized_size,
            created_at_ms,
        })
    }

    pub fn unwrap(self) -> Result<T, CorruptionError> {
        let serialized =
            bincode::serialize(&self.data).map_err(|e| CorruptionError::DeserializationError {
                key: String::new(),
                reason: format!("Failed to serialize for validation: {}", e),
            })?;

        let mut hasher = Sha256::new();
        hasher.update(&serialized);
        let actual_checksum = format!("{:x}", hasher.finalize());

        if actual_checksum != self.checksum {
            return Err(CorruptionError::ChecksumMismatch {
                key: String::new(),
                expected: self.checksum,
                actual: actual_checksum,
            });
        }

        Ok(self.data)
    }

    pub fn validate(&self) -> bool {
        let serialized = match bincode::serialize(&self.data) {
            Ok(s) => s,
            Err(_) => return false,
        };

        let mut hasher = Sha256::new();
        hasher.update(&serialized);
        let actual_checksum = format!("{:x}", hasher.finalize());

        actual_checksum == self.checksum
    }
}

pub fn load_with_integrity<T: DeserializeOwned>(
    path: &Path,
    expected_checksum: Option<&str>,
    quarantine_dir: &Path,
) -> Result<Option<T>, CorruptionError> {
    if !path.exists() {
        return Ok(None);
    }

    let data = fs::read(path).map_err(|e| CorruptionError::DeserializationError {
        key: path.to_string_lossy().to_string(),
        reason: e.to_string(),
    })?;

    let mut hasher = Sha256::new();
    hasher.update(&data);
    let actual_checksum = format!("{:x}", hasher.finalize());

    if let Some(expected) = expected_checksum {
        if actual_checksum != expected {
            let key = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            let quarantine_subdir = quarantine_dir.join(&key);
            if let Ok(()) = fs::create_dir_all(&quarantine_subdir) {
                let quarantine_path = quarantine_subdir.join("data");
                let _ = fs::rename(path, &quarantine_path);

                let quarantined_at_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as i64;

                let meta_content = format!(
                    "key={}\nreason=checksum_mismatch\nquarantined_at_ms={}\noriginal_size={}\noriginal_path={}\n",
                    key,
                    quarantined_at_ms,
                    data.len(),
                    path.display(),
                );
                let _ = fs::write(quarantine_subdir.join("metadata.txt"), meta_content);
            }

            return Err(CorruptionError::ChecksumMismatch {
                key: path.to_string_lossy().to_string(),
                expected: expected.to_string(),
                actual: actual_checksum,
            });
        }
    }

    let value: T =
        bincode::deserialize(&data).map_err(|e| CorruptionError::DeserializationError {
            key: path.to_string_lossy().to_string(),
            reason: e.to_string(),
        })?;

    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use tempfile::TempDir;

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct TestData {
        value: i32,
        label: String,
    }

    #[test]
    fn test_checksummed_cache_entry_roundtrip() {
        let data = TestData {
            value: 42,
            label: "hello".to_string(),
        };

        let entry = ChecksummedCacheEntry::wrap(data.clone()).unwrap();
        assert!(!entry.checksum.is_empty());
        assert!(entry.serialized_size > 0);
        assert!(entry.created_at_ms > 0);

        let unwrapped = entry.unwrap().unwrap();
        assert_eq!(unwrapped, data);
    }

    #[test]
    fn test_checksummed_cache_entry_detects_tampering() {
        let data = TestData {
            value: 42,
            label: "hello".to_string(),
        };

        let mut entry = ChecksummedCacheEntry::wrap(data).unwrap();
        entry.data.value = 99;

        assert!(!entry.validate());

        let result = entry.unwrap();
        assert!(result.is_err());
        match result.unwrap_err() {
            CorruptionError::ChecksumMismatch { .. } => {}
            other => panic!("Expected ChecksumMismatch, got {:?}", other),
        }
    }

    #[test]
    fn test_corruption_scanner_finds_corrupted_files() {
        let tmp = TempDir::new().unwrap();
        let cache_dir = tmp.path().join("cache");
        let quarantine_dir = tmp.path().join("quarantine");
        fs::create_dir_all(&cache_dir).unwrap();
        fs::create_dir_all(&quarantine_dir).unwrap();

        let shard = cache_dir.join("ab");
        fs::create_dir_all(&shard).unwrap();

        let good_data = b"good data";
        let mut hasher = Sha256::new();
        hasher.update(good_data);
        let good_checksum = format!("{:x}", hasher.finalize());

        fs::write(shard.join("1234567890"), good_data).unwrap();

        let scanner = CorruptionScanner::new(cache_dir.clone(), quarantine_dir.clone());
        scanner.register_expected_checksum("ab1234567890".to_string(), good_checksum.clone());

        let result = scanner.scan_entry("ab1234567890").unwrap();
        match result {
            EntryIntegrity::Valid { checksum, size } => {
                assert_eq!(checksum, good_checksum);
                assert_eq!(size, good_data.len() as u64);
            }
            other => panic!("Expected Valid, got {:?}", other),
        }

        fs::write(shard.join("1234567890"), b"tampered").unwrap();

        let result = scanner.scan_entry("ab1234567890").unwrap();
        match result {
            EntryIntegrity::Corrupted(CorruptionError::ChecksumMismatch { .. }) => {}
            other => panic!("Expected Corrupted, got {:?}", other),
        }
    }

    #[test]
    fn test_corruption_scanner_auto_quarantines() {
        let tmp = TempDir::new().unwrap();
        let cache_dir = tmp.path().join("cache");
        let quarantine_dir = tmp.path().join("quarantine");
        fs::create_dir_all(&cache_dir).unwrap();
        fs::create_dir_all(&quarantine_dir).unwrap();

        let shard = cache_dir.join("cd");
        fs::create_dir_all(&shard).unwrap();

        let key = "cd1234567890";
        let data_path = shard.join("1234567890");
        fs::write(&data_path, b"original").unwrap();

        let scanner = CorruptionScanner::new(cache_dir.clone(), quarantine_dir.clone());
        scanner.register_expected_checksum(key.to_string(), "wrong_checksum".to_string());

        let report = scanner.scan().unwrap();
        assert_eq!(report.corrupted_entries.len(), 1);
        assert!(report.corrupted_entries[0].auto_quarantined);

        assert!(!data_path.exists());
        assert!(quarantine_dir.join(key).join("data").exists());
    }

    #[test]
    fn test_scan_report_accuracy() {
        let tmp = TempDir::new().unwrap();
        let cache_dir = tmp.path().join("cache");
        let quarantine_dir = tmp.path().join("quarantine");
        fs::create_dir_all(&cache_dir).unwrap();
        fs::create_dir_all(&quarantine_dir).unwrap();

        let shard = cache_dir.join("ef");
        fs::create_dir_all(&shard).unwrap();

        let data1 = b"entry one";
        let data2 = b"entry two";

        fs::write(shard.join("1111111111"), data1).unwrap();
        fs::write(shard.join("2222222222"), data2).unwrap();

        let scanner = CorruptionScanner::new(cache_dir.clone(), quarantine_dir.clone());

        let mut hasher1 = Sha256::new();
        hasher1.update(data1);
        scanner.register_expected_checksum(
            "ef1111111111".to_string(),
            format!("{:x}", hasher1.finalize()),
        );

        let mut hasher2 = Sha256::new();
        hasher2.update(data2);
        scanner.register_expected_checksum(
            "ef2222222222".to_string(),
            format!("{:x}", hasher2.finalize()),
        );

        let report = scanner.scan().unwrap();
        assert_eq!(report.total_entries, 2);
        assert_eq!(report.valid_entries, 2);
        assert_eq!(report.corrupted_entries.len(), 0);
        assert!(report.scan_time_ms < 1000);
    }

    #[test]
    fn test_load_with_integrity_missing_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("nonexistent");
        let quarantine = tmp.path().join("quarantine");

        let result = load_with_integrity::<TestData>(&path, None, &quarantine).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_load_with_integrity_quarantines_corrupted() {
        let tmp = TempDir::new().unwrap();
        let cache_dir = tmp.path().join("cache");
        let quarantine_dir = tmp.path().join("quarantine");
        fs::create_dir_all(&cache_dir).unwrap();
        fs::create_dir_all(&quarantine_dir).unwrap();

        let data = TestData {
            value: 100,
            label: "test".to_string(),
        };
        let serialized = bincode::serialize(&data).unwrap();
        let file_path = cache_dir.join("myfile");
        fs::write(&file_path, &serialized).unwrap();

        let result =
            load_with_integrity::<TestData>(&file_path, Some("wrong_checksum"), &quarantine_dir);
        assert!(result.is_err());
        match result.unwrap_err() {
            CorruptionError::ChecksumMismatch { .. } => {}
            other => panic!("Expected ChecksumMismatch, got {:?}", other),
        }

        assert!(!file_path.exists());
        assert!(quarantine_dir.join("myfile").join("data").exists());
    }
}
