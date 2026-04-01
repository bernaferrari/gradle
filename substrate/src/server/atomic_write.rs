use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum AtomicWriteError {
    #[error("IO error: {0}")]
    IoError(String),
    #[error("Commit failed: {0}")]
    CommitFailed(String),
    #[error("Rollback failed: {0}")]
    RollbackFailed(String),
}

impl From<std::io::Error> for AtomicWriteError {
    fn from(e: std::io::Error) -> Self {
        AtomicWriteError::IoError(e.to_string())
    }
}

pub struct AtomicWriter {
    target_path: PathBuf,
    temp_path: PathBuf,
    committed: bool,
}

impl AtomicWriter {
    pub fn new(target: PathBuf) -> Self {
        let pid = std::process::id();
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros();
        let temp_path = target.with_extension(format!("tmp.{pid}.{timestamp}"));
        Self {
            target_path: target,
            temp_path,
            committed: false,
        }
    }

    pub fn write_all(&mut self, data: &[u8]) -> Result<(), AtomicWriteError> {
        if let Some(parent) = self.temp_path.parent() {
            fs::create_dir_all(parent).map_err(|e| AtomicWriteError::IoError(e.to_string()))?;
        }
        fs::write(&self.temp_path, data).map_err(|e| AtomicWriteError::IoError(e.to_string()))?;
        Ok(())
    }

    pub fn commit(&mut self) -> Result<(), AtomicWriteError> {
        fs::rename(&self.temp_path, &self.target_path)
            .map_err(|e| AtomicWriteError::CommitFailed(e.to_string()))?;
        self.committed = true;
        Ok(())
    }

    pub fn rollback(&mut self) -> Result<(), AtomicWriteError> {
        if self.temp_path.exists() {
            fs::remove_file(&self.temp_path)
                .map_err(|e| AtomicWriteError::RollbackFailed(e.to_string()))?;
        }
        self.committed = true;
        Ok(())
    }
}

impl Drop for AtomicWriter {
    fn drop(&mut self) {
        if !self.committed && self.temp_path.exists() {
            let _ = fs::remove_file(&self.temp_path);
        }
    }
}

#[derive(Debug, Clone)]
pub struct QuarantineEntry {
    pub key: String,
    pub original_path: String,
    pub quarantine_path: String,
    pub reason: String,
    pub quarantined_at_ms: i64,
    pub original_size: u64,
}

pub struct AtomicCacheStore {
    base: crate::server::cache::LocalCacheStore,
    base_dir: PathBuf,
    quarantine_dir: PathBuf,
}

impl AtomicCacheStore {
    pub fn new(base_dir: PathBuf, quarantine_dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&quarantine_dir);
        Self {
            base: crate::server::cache::LocalCacheStore::new(base_dir.clone()),
            base_dir,
            quarantine_dir,
        }
    }

    pub fn base_store(&self) -> &crate::server::cache::LocalCacheStore {
        &self.base
    }

    fn key_to_path(&self, key: &str) -> PathBuf {
        let trimmed = key.trim();
        if trimmed.len() > 2 {
            self.base_dir.join(&trimmed[..2]).join(&trimmed[2..])
        } else {
            self.base_dir.join(trimmed)
        }
    }

    pub async fn store(&self, key: &str, data: &[u8]) -> Result<(), AtomicWriteError> {
        let target = self.key_to_path(key);
        let mut writer = AtomicWriter::new(target.clone());
        writer
            .write_all(data)
            .map_err(|e| AtomicWriteError::IoError(e.to_string()))?;
        writer
            .commit()
            .map_err(|e| AtomicWriteError::CommitFailed(e.to_string()))?;
        Ok(())
    }

    pub fn quarantine(&self, key: &str, reason: &str) -> Result<(), AtomicWriteError> {
        let source = self.key_to_path(key);
        if !source.exists() {
            return Err(AtomicWriteError::IoError(format!(
                "Cannot quarantine: source file does not exist for key {}",
                key
            )));
        }

        let metadata = fs::metadata(&source).map_err(|e| AtomicWriteError::IoError(e.to_string()))?;
        let original_size = metadata.len();

        let quarantine_subdir = self.quarantine_dir.join(key);
        fs::create_dir_all(&quarantine_subdir)
            .map_err(|e| AtomicWriteError::IoError(e.to_string()))?;

        let quarantine_path = quarantine_subdir.join("data");
        fs::rename(&source, &quarantine_path)
            .map_err(|e| AtomicWriteError::CommitFailed(e.to_string()))?;

        let quarantined_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let meta_content = format!(
            "key={}\nreason={}\nquarantined_at_ms={}\noriginal_size={}\noriginal_path={}\n",
            key,
            reason,
            quarantined_at_ms,
            original_size,
            source.display(),
        );
        let meta_path = quarantine_subdir.join("metadata.txt");
        fs::write(&meta_path, meta_content)
            .map_err(|e| AtomicWriteError::IoError(e.to_string()))?;

        Ok(())
    }

    pub fn quarantined_entries(&self) -> Result<Vec<QuarantineEntry>, AtomicWriteError> {
        let mut entries = Vec::new();

        if !self.quarantine_dir.exists() {
            return Ok(entries);
        }

        for key_dir in fs::read_dir(&self.quarantine_dir)
            .map_err(|e| AtomicWriteError::IoError(e.to_string()))?
        {
            let key_dir = key_dir.map_err(|e| AtomicWriteError::IoError(e.to_string()))?;
            if !key_dir.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                continue;
            }

            let key = key_dir.file_name().to_string_lossy().to_string();
            let meta_path = key_dir.path().join("metadata.txt");
            let data_path = key_dir.path().join("data");

            if !meta_path.exists() {
                continue;
            }

            let meta_content =
                fs::read_to_string(&meta_path).map_err(|e| AtomicWriteError::IoError(e.to_string()))?;

            let mut original_path = String::new();
            let mut reason = String::new();
            let mut quarantined_at_ms: i64 = 0;
            let mut original_size: u64 = 0;

            for line in meta_content.lines() {
                if let Some(value) = line.strip_prefix("key=") {
                    let _ = value;
                } else if let Some(value) = line.strip_prefix("reason=") {
                    reason = value.to_string();
                } else if let Some(value) = line.strip_prefix("quarantined_at_ms=") {
                    quarantined_at_ms = value.parse().unwrap_or(0);
                } else if let Some(value) = line.strip_prefix("original_size=") {
                    original_size = value.parse().unwrap_or(0);
                } else if let Some(value) = line.strip_prefix("original_path=") {
                    original_path = value.to_string();
                }
            }

            let quarantine_path = data_path.to_string_lossy().to_string();

            entries.push(QuarantineEntry {
                key,
                original_path,
                quarantine_path,
                reason,
                quarantined_at_ms,
                original_size,
            });
        }

        Ok(entries)
    }

    pub fn restore_from_quarantine(&self, key: &str) -> Result<(), AtomicWriteError> {
        let quarantine_subdir = self.quarantine_dir.join(key);
        let data_path = quarantine_subdir.join("data");

        if !data_path.exists() {
            return Err(AtomicWriteError::IoError(format!(
                "Quarantined data file not found for key {}",
                key
            )));
        }

        let target = self.key_to_path(key);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| AtomicWriteError::IoError(e.to_string()))?;
        }

        fs::rename(&data_path, &target)
            .map_err(|e| AtomicWriteError::CommitFailed(e.to_string()))?;

        let _ = fs::remove_dir_all(&quarantine_subdir);

        Ok(())
    }
}

pub fn integrity_check(data: &[u8], expected_checksum: &[u8]) -> bool {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    result.as_slice() == expected_checksum
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_atomic_write_commit_succeeds() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("test_file");
        let mut writer = AtomicWriter::new(target.clone());

        writer.write_all(b"hello world").unwrap();
        assert!(!target.exists());
        assert!(writer.temp_path.exists());

        writer.commit().unwrap();
        assert!(target.exists());
        assert!(!writer.temp_path.exists());

        let content = fs::read_to_string(&target).unwrap();
        assert_eq!(content, "hello world");
    }

    #[test]
    fn test_atomic_write_rollback_on_drop() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("test_file");

        {
            let mut writer = AtomicWriter::new(target.clone());
            writer.write_all(b"should be rolled back").unwrap();
            assert!(writer.temp_path.exists());
            assert!(!target.exists());
        }

        assert!(!target.exists());
    }

    #[test]
    fn test_atomic_write_rollback_explicit() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("test_file");
        let mut writer = AtomicWriter::new(target.clone());

        writer.write_all(b"explicit rollback").unwrap();
        assert!(writer.temp_path.exists());

        writer.rollback().unwrap();
        assert!(!target.exists());
        assert!(!writer.temp_path.exists());
    }

    #[test]
    fn test_quarantine_entry_creation_and_listing() {
        let tmp = TempDir::new().unwrap();
        let base_dir = tmp.path().join("cache");
        let quarantine_dir = tmp.path().join("quarantine");
        fs::create_dir_all(&base_dir).unwrap();

        let store = AtomicCacheStore::new(base_dir, quarantine_dir.clone());

        let key = "ab1234567890";
        let data = b"corrupted data";
        let target = store.key_to_path(key);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, data).unwrap();

        store.quarantine(key, "checksum mismatch").unwrap();

        assert!(!target.exists());

        let entries = store.quarantined_entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, key);
        assert_eq!(entries[0].reason, "checksum mismatch");
        assert_eq!(entries[0].original_size, data.len() as u64);
        assert!(entries[0].quarantined_at_ms > 0);
    }

    #[test]
    fn test_restore_from_quarantine() {
        let tmp = TempDir::new().unwrap();
        let base_dir = tmp.path().join("cache");
        let quarantine_dir = tmp.path().join("quarantine");
        fs::create_dir_all(&base_dir).unwrap();

        let store = AtomicCacheStore::new(base_dir.clone(), quarantine_dir.clone());

        let key = "cd1234567890";
        let data = b"restore me";
        let target = store.key_to_path(key);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(&target, data).unwrap();

        store.quarantine(key, "test quarantine").unwrap();
        assert!(!target.exists());

        store.restore_from_quarantine(key).unwrap();
        assert!(target.exists());

        let restored = fs::read(&target).unwrap();
        assert_eq!(restored, data);

        let entries = store.quarantined_entries().unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_integrity_check_passes() {
        let data = b"test data for integrity";
        let mut hasher = Sha256::new();
        hasher.update(data);
        let expected = hasher.finalize();

        assert!(integrity_check(data, expected.as_slice()));
    }

    #[test]
    fn test_integrity_check_fails() {
        let data = b"test data for integrity";
        let mut hasher = Sha256::new();
        hasher.update(b"wrong data");
        let wrong_checksum = hasher.finalize();

        assert!(!integrity_check(data, wrong_checksum.as_slice()));
    }
}
