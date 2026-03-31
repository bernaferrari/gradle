use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Error, Debug)]
pub enum SchemaVersionedError {
    #[error("version mismatch: got {got}, expected {expected}")]
    VersionMismatch { got: u32, expected: u32 },

    #[error("checksum mismatch: expected {expected}, got {got}")]
    ChecksumMismatch { expected: String, got: String },

    #[error("unknown checksum algorithm: {0}")]
    UnknownChecksumAlgorithm(u8),

    #[error("serialization error: {0}")]
    SerializationError(String),

    #[error("migration error: from {from} to {to}: {reason}")]
    MigrationError { from: u32, to: u32, reason: String },

    #[error("IO error: {0}")]
    IoError(String),
}

// ---------------------------------------------------------------------------
// SchemaVersion
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SchemaVersion(pub u32);

impl SchemaVersion {
    pub const V1: SchemaVersion = SchemaVersion(1);
    pub const CURRENT: SchemaVersion = Self::V1;

    pub fn is_compatible_with(&self, expected: SchemaVersion) -> bool {
        self.major() == expected.major()
    }

    pub fn major(&self) -> u32 {
        self.0 / 1000
    }

    pub fn minor(&self) -> u32 {
        self.0 % 1000
    }
}

impl fmt::Display for SchemaVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major(), self.minor())
    }
}

// ---------------------------------------------------------------------------
// ChecksumAlgorithm
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChecksumAlgorithm {
    Sha256,
    Sha1,
    Crc32,
}

impl ChecksumAlgorithm {
    pub fn tag(&self) -> u8 {
        match self {
            ChecksumAlgorithm::Sha256 => 0,
            ChecksumAlgorithm::Sha1 => 1,
            ChecksumAlgorithm::Crc32 => 2,
        }
    }

    pub fn from_tag(tag: u8) -> Result<Self, SchemaVersionedError> {
        match tag {
            0 => Ok(ChecksumAlgorithm::Sha256),
            1 => Ok(ChecksumAlgorithm::Sha1),
            2 => Ok(ChecksumAlgorithm::Crc32),
            other => Err(SchemaVersionedError::UnknownChecksumAlgorithm(other)),
        }
    }

    pub fn compute(&self, data: &[u8]) -> Vec<u8> {
        match self {
            ChecksumAlgorithm::Sha256 => {
                let mut hasher = Sha256::new();
                hasher.update(data);
                hasher.finalize().to_vec()
            }
            ChecksumAlgorithm::Sha1 => {
                let mut hasher = sha1::Sha1::new();
                hasher.update(data);
                hasher.finalize().to_vec()
            }
            ChecksumAlgorithm::Crc32 => {
                let crc = crc32fast::hash(data);
                crc.to_le_bytes().to_vec()
            }
        }
    }

    pub fn checksum_len(&self) -> usize {
        match self {
            ChecksumAlgorithm::Sha256 => 32,
            ChecksumAlgorithm::Sha1 => 20,
            ChecksumAlgorithm::Crc32 => 4,
        }
    }
}

// ---------------------------------------------------------------------------
// VersionedPayload
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionedPayload<T> {
    pub schema_version: u32,
    pub checksum: Vec<u8>,
    pub checksum_algorithm: u8,
    pub data: Vec<u8>,
    #[serde(skip)]
    _phantom: std::marker::PhantomData<T>,
}

impl<T: Serialize + DeserializeOwned> VersionedPayload<T> {
    pub fn encode(
        value: &T,
        version: SchemaVersion,
        algo: ChecksumAlgorithm,
    ) -> Result<Self, SchemaVersionedError> {
        let data = bincode::serialize(value)
            .map_err(|e| SchemaVersionedError::SerializationError(e.to_string()))?;
        let checksum = algo.compute(&data);
        Ok(VersionedPayload {
            schema_version: version.0,
            checksum,
            checksum_algorithm: algo.tag(),
            data,
            _phantom: std::marker::PhantomData,
        })
    }

    pub fn decode(&self, expected_version: SchemaVersion) -> Result<T, SchemaVersionedError> {
        let algo = ChecksumAlgorithm::from_tag(self.checksum_algorithm)?;

        if !SchemaVersion(self.schema_version).is_compatible_with(expected_version) {
            return Err(SchemaVersionedError::VersionMismatch {
                got: self.schema_version,
                expected: expected_version.0,
            });
        }

        if !self.validate_checksum() {
            let expected_hex = hex::encode(&algo.compute(&self.data));
            let got_hex = hex::encode(&self.checksum);
            return Err(SchemaVersionedError::ChecksumMismatch {
                expected: expected_hex,
                got: got_hex,
            });
        }

        bincode::deserialize(&self.data)
            .map_err(|e| SchemaVersionedError::SerializationError(e.to_string()))
    }

    pub fn validate_checksum(&self) -> bool {
        if let Ok(algo) = ChecksumAlgorithm::from_tag(self.checksum_algorithm) {
            let expected = algo.compute(&self.data);
            expected == self.checksum
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// VersionedFileStore
// ---------------------------------------------------------------------------

pub struct VersionedFileStore {
    base_dir: PathBuf,
    default_version: SchemaVersion,
    default_checksum: ChecksumAlgorithm,
}

impl VersionedFileStore {
    pub fn new(
        base_dir: PathBuf,
        default_version: SchemaVersion,
        default_checksum: ChecksumAlgorithm,
    ) -> Self {
        Self {
            base_dir,
            default_version,
            default_checksum,
        }
    }

    fn key_to_path(&self, key: &str) -> PathBuf {
        self.base_dir.join(key)
    }

    pub fn write<T: Serialize>(&self, key: &str, value: &T) -> Result<(), SchemaVersionedError> {
        let data = bincode::serialize(value)
            .map_err(|e| SchemaVersionedError::SerializationError(e.to_string()))?;
        let checksum = self.default_checksum.compute(&data);

        let path = self.key_to_path(key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| SchemaVersionedError::IoError(e.to_string()))?;
        }

        let mut file_bytes = Vec::new();
        file_bytes.extend_from_slice(&self.default_version.0.to_le_bytes());
        file_bytes.push(self.default_checksum.tag());
        file_bytes.extend_from_slice(&checksum);
        file_bytes.extend_from_slice(&data);

        std::fs::write(&path, &file_bytes)
            .map_err(|e| SchemaVersionedError::IoError(e.to_string()))?;

        Ok(())
    }

    pub fn read<T: DeserializeOwned>(
        &self,
        key: &str,
        compatible_from: SchemaVersion,
        compatible_to: SchemaVersion,
    ) -> Result<T, SchemaVersionedError> {
        let path = self.key_to_path(key);
        let file_bytes =
            std::fs::read(&path).map_err(|e| SchemaVersionedError::IoError(e.to_string()))?;

        if file_bytes.len() < 5 {
            return Err(SchemaVersionedError::SerializationError(
                "file too short to contain header".into(),
            ));
        }

        let version_bytes: [u8; 4] = file_bytes[..4].try_into().map_err(|_| {
            SchemaVersionedError::SerializationError("invalid version bytes".into())
        })?;
        let version = SchemaVersion(u32::from_le_bytes(version_bytes));

        let algo_tag = file_bytes[4];
        let algo = ChecksumAlgorithm::from_tag(algo_tag)?;

        if !version.is_compatible_with(compatible_from)
            || !version.is_compatible_with(compatible_to)
        {
            return Err(SchemaVersionedError::VersionMismatch {
                got: version.0,
                expected: compatible_to.0,
            });
        }

        let checksum_len = algo.checksum_len();
        let header_size = 5 + checksum_len;

        if file_bytes.len() < header_size {
            return Err(SchemaVersionedError::SerializationError(
                "file too short to contain checksum".into(),
            ));
        }

        let stored_checksum = &file_bytes[5..header_size];
        let data = &file_bytes[header_size..];

        let computed_checksum = algo.compute(data);
        if stored_checksum != computed_checksum {
            return Err(SchemaVersionedError::ChecksumMismatch {
                expected: hex::encode(&computed_checksum),
                got: hex::encode(stored_checksum),
            });
        }

        bincode::deserialize(data)
            .map_err(|e| SchemaVersionedError::SerializationError(e.to_string()))
    }

    pub fn exists(&self, key: &str) -> bool {
        self.key_to_path(key).exists()
    }

    pub fn remove(&self, key: &str) -> Result<bool, SchemaVersionedError> {
        let path = self.key_to_path(key);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(SchemaVersionedError::IoError(e.to_string())),
        }
    }
}

// ---------------------------------------------------------------------------
// MigrationRegistry
// ---------------------------------------------------------------------------

type MigrationFn = dyn Fn(Vec<u8>) -> Result<Vec<u8>, SchemaVersionedError> + Send + Sync;

#[derive(Default)]
pub struct MigrationRegistry {
    migrations: BTreeMap<(u32, u32), Box<MigrationFn>>,
}

impl MigrationRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        from: SchemaVersion,
        to: SchemaVersion,
        f: impl Fn(Vec<u8>) -> Result<Vec<u8>, SchemaVersionedError> + Send + Sync + 'static,
    ) {
        self.migrations.insert((from.0, to.0), Box::new(f));
    }

    pub fn migrate(
        &self,
        data: Vec<u8>,
        from: SchemaVersion,
        to: SchemaVersion,
    ) -> Result<Vec<u8>, SchemaVersionedError> {
        if from == to {
            return Ok(data);
        }

        if !self.has_path(from, to) {
            return Err(SchemaVersionedError::MigrationError {
                from: from.0,
                to: to.0,
                reason: "no migration path".into(),
            });
        }

        let mut current = data;
        let mut current_version = from.0;

        while current_version < to.0 {
            let next_version = current_version + 1;
            let key = (current_version, next_version);

            let migration_fn =
                self.migrations
                    .get(&key)
                    .ok_or_else(|| SchemaVersionedError::MigrationError {
                        from: current_version,
                        to: next_version,
                        reason: format!(
                            "missing migration step {} -> {}",
                            current_version, next_version
                        ),
                    })?;

            current = migration_fn(current).map_err(|e| SchemaVersionedError::MigrationError {
                from: current_version,
                to: next_version,
                reason: e.to_string(),
            })?;

            current_version = next_version;
        }

        Ok(current)
    }

    pub fn has_path(&self, from: SchemaVersion, to: SchemaVersion) -> bool {
        if from == to {
            return true;
        }
        if from.0 > to.0 {
            return false;
        }
        let mut current = from.0;
        while current < to.0 {
            let next = current + 1;
            if !self.migrations.contains_key(&(current, next)) {
                return false;
            }
            current = next;
        }
        true
    }
}

// ---------------------------------------------------------------------------
// hex helper (matches existing cache.rs style)
// ---------------------------------------------------------------------------

mod hex {
    const HEX_TABLE: &[u8; 16] = b"0123456789abcdef";

    pub fn encode(bytes: &[u8]) -> String {
        let mut s = String::with_capacity(bytes.len() * 2);
        for &b in bytes {
            s.push(HEX_TABLE[(b >> 4) as usize] as char);
            s.push(HEX_TABLE[(b & 0x0F) as usize] as char);
        }
        s
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // -----------------------------------------------------------------------
    // SchemaVersion
    // -----------------------------------------------------------------------

    #[test]
    fn test_schema_version_constants() {
        assert_eq!(SchemaVersion::V1.0, 1);
        assert_eq!(SchemaVersion::CURRENT.0, 1);
    }

    #[test]
    fn test_schema_version_major_minor() {
        let v = SchemaVersion(1042);
        assert_eq!(v.major(), 1);
        assert_eq!(v.minor(), 42);

        let v2 = SchemaVersion(0);
        assert_eq!(v2.major(), 0);
        assert_eq!(v2.minor(), 0);

        let v3 = SchemaVersion(2000);
        assert_eq!(v3.major(), 2);
        assert_eq!(v3.minor(), 0);
    }

    #[test]
    fn test_schema_version_compatible_same_major() {
        let a = SchemaVersion(1);
        let b = SchemaVersion(500);
        assert!(a.is_compatible_with(b));
        assert!(b.is_compatible_with(a));
    }

    #[test]
    fn test_schema_version_incompatible_different_major() {
        let a = SchemaVersion(1);
        let b = SchemaVersion(1001);
        assert!(!a.is_compatible_with(b));
        assert!(!b.is_compatible_with(a));
    }

    #[test]
    fn test_schema_version_display() {
        let v = SchemaVersion(1042);
        assert_eq!(format!("{}", v), "1.42");
    }

    // -----------------------------------------------------------------------
    // ChecksumAlgorithm
    // -----------------------------------------------------------------------

    #[test]
    fn test_checksum_algorithm_tags() {
        assert_eq!(ChecksumAlgorithm::Sha256.tag(), 0);
        assert_eq!(ChecksumAlgorithm::Sha1.tag(), 1);
        assert_eq!(ChecksumAlgorithm::Crc32.tag(), 2);
    }

    #[test]
    fn test_checksum_algorithm_from_tag() {
        assert_eq!(
            ChecksumAlgorithm::from_tag(0).unwrap(),
            ChecksumAlgorithm::Sha256
        );
        assert_eq!(
            ChecksumAlgorithm::from_tag(1).unwrap(),
            ChecksumAlgorithm::Sha1
        );
        assert_eq!(
            ChecksumAlgorithm::from_tag(2).unwrap(),
            ChecksumAlgorithm::Crc32
        );
        assert!(ChecksumAlgorithm::from_tag(99).is_err());
    }

    #[test]
    fn test_checksum_sha256_deterministic() {
        let data = b"hello world";
        let c1 = ChecksumAlgorithm::Sha256.compute(data);
        let c2 = ChecksumAlgorithm::Sha256.compute(data);
        assert_eq!(c1, c2);
        assert_eq!(c1.len(), 32);
    }

    #[test]
    fn test_checksum_sha1_deterministic() {
        let data = b"hello world";
        let c1 = ChecksumAlgorithm::Sha1.compute(data);
        let c2 = ChecksumAlgorithm::Sha1.compute(data);
        assert_eq!(c1, c2);
        assert_eq!(c1.len(), 20);
    }

    #[test]
    fn test_checksum_crc32_deterministic() {
        let data = b"hello world";
        let c1 = ChecksumAlgorithm::Crc32.compute(data);
        let c2 = ChecksumAlgorithm::Crc32.compute(data);
        assert_eq!(c1, c2);
        assert_eq!(c1.len(), 4);
    }

    #[test]
    fn test_checksum_algorithms_produce_different_results() {
        let data = b"hello world";
        let sha = ChecksumAlgorithm::Sha256.compute(data);
        let crc = ChecksumAlgorithm::Crc32.compute(data);
        assert_ne!(sha, crc);
    }

    #[test]
    fn test_checksum_length() {
        assert_eq!(ChecksumAlgorithm::Sha256.checksum_len(), 32);
        assert_eq!(ChecksumAlgorithm::Sha1.checksum_len(), 20);
        assert_eq!(ChecksumAlgorithm::Crc32.checksum_len(), 4);
    }

    // -----------------------------------------------------------------------
    // VersionedPayload encode/decode round-trip
    // -----------------------------------------------------------------------

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct TestPayload {
        name: String,
        value: i64,
    }

    #[test]
    fn test_versioned_payload_roundtrip_sha256() {
        let original = TestPayload {
            name: "test".into(),
            value: 42,
        };
        let payload =
            VersionedPayload::encode(&original, SchemaVersion::V1, ChecksumAlgorithm::Sha256)
                .unwrap();

        assert_eq!(payload.schema_version, 1);
        assert_eq!(payload.checksum_algorithm, 0);
        assert!(payload.validate_checksum());

        let decoded = payload.decode(SchemaVersion::V1).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_versioned_payload_roundtrip_sha1() {
        let original = TestPayload {
            name: "sha1-test".into(),
            value: 99,
        };
        let payload =
            VersionedPayload::encode(&original, SchemaVersion::V1, ChecksumAlgorithm::Sha1)
                .unwrap();

        assert_eq!(payload.checksum_algorithm, 1);
        assert!(payload.validate_checksum());

        let decoded = payload.decode(SchemaVersion::V1).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_versioned_payload_roundtrip_crc32() {
        let original = TestPayload {
            name: "crc-test".into(),
            value: 777,
        };
        let payload =
            VersionedPayload::encode(&original, SchemaVersion::V1, ChecksumAlgorithm::Crc32)
                .unwrap();

        assert_eq!(payload.checksum_algorithm, 2);
        assert!(payload.validate_checksum());

        let decoded = payload.decode(SchemaVersion::V1).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_versioned_payload_version_mismatch() {
        let original = TestPayload {
            name: "test".into(),
            value: 1,
        };
        let payload =
            VersionedPayload::encode(&original, SchemaVersion(1001), ChecksumAlgorithm::Sha256)
                .unwrap();

        let result = payload.decode(SchemaVersion::V1);
        assert!(matches!(
            result.unwrap_err(),
            SchemaVersionedError::VersionMismatch { .. }
        ));
    }

    #[test]
    fn test_versioned_payload_checksum_corruption() {
        let original = TestPayload {
            name: "test".into(),
            value: 1,
        };
        let mut payload =
            VersionedPayload::encode(&original, SchemaVersion::V1, ChecksumAlgorithm::Sha256)
                .unwrap();

        payload.checksum[0] ^= 0xFF;

        assert!(!payload.validate_checksum());

        let result = payload.decode(SchemaVersion::V1);
        assert!(matches!(
            result.unwrap_err(),
            SchemaVersionedError::ChecksumMismatch { .. }
        ));
    }

    #[test]
    fn test_versioned_payload_data_corruption() {
        let original = TestPayload {
            name: "test".into(),
            value: 1,
        };
        let mut payload =
            VersionedPayload::encode(&original, SchemaVersion::V1, ChecksumAlgorithm::Sha256)
                .unwrap();

        payload.data[0] ^= 0xFF;

        assert!(!payload.validate_checksum());

        let result = payload.decode(SchemaVersion::V1);
        assert!(matches!(
            result.unwrap_err(),
            SchemaVersionedError::ChecksumMismatch { .. }
        ));
    }

    #[test]
    fn test_versioned_payload_unknown_algorithm() {
        let original = TestPayload {
            name: "test".into(),
            value: 1,
        };
        let mut payload =
            VersionedPayload::encode(&original, SchemaVersion::V1, ChecksumAlgorithm::Sha256)
                .unwrap();

        payload.checksum_algorithm = 99;

        let result = payload.decode(SchemaVersion::V1);
        assert!(matches!(
            result.unwrap_err(),
            SchemaVersionedError::UnknownChecksumAlgorithm(99)
        ));
    }

    // -----------------------------------------------------------------------
    // MigrationRegistry
    // -----------------------------------------------------------------------

    #[test]
    fn test_migration_registry_register_and_has_path() {
        let mut registry = MigrationRegistry::new();
        registry.register(SchemaVersion(1), SchemaVersion(2), |data| Ok(data));
        registry.register(SchemaVersion(2), SchemaVersion(3), |data| Ok(data));

        assert!(registry.has_path(SchemaVersion(1), SchemaVersion(3)));
        assert!(registry.has_path(SchemaVersion(1), SchemaVersion(2)));
        assert!(registry.has_path(SchemaVersion(2), SchemaVersion(3)));
        assert!(registry.has_path(SchemaVersion(1), SchemaVersion(1)));
        assert!(!registry.has_path(SchemaVersion(1), SchemaVersion(4)));
    }

    #[test]
    fn test_migration_no_op_when_same_version() {
        let registry = MigrationRegistry::new();
        let data = vec![1, 2, 3];
        let result = registry
            .migrate(data.clone(), SchemaVersion(1), SchemaVersion(1))
            .unwrap();
        assert_eq!(result, data);
    }

    #[test]
    fn test_migration_chain_execution() {
        let mut registry = MigrationRegistry::new();

        registry.register(SchemaVersion(1), SchemaVersion(2), |data| {
            let mut out = data;
            out.push(0xAA);
            Ok(out)
        });

        registry.register(SchemaVersion(2), SchemaVersion(3), |data| {
            let mut out = data;
            out.push(0xBB);
            Ok(out)
        });

        let input = vec![1, 2, 3];
        let result = registry
            .migrate(input.clone(), SchemaVersion(1), SchemaVersion(3))
            .unwrap();

        assert_eq!(result, vec![1, 2, 3, 0xAA, 0xBB]);
    }

    #[test]
    fn test_migration_error_propagates() {
        let mut registry = MigrationRegistry::new();

        registry.register(SchemaVersion(1), SchemaVersion(2), |_data| {
            Err(SchemaVersionedError::SerializationError("boom".into()))
        });

        let result = registry.migrate(vec![1], SchemaVersion(1), SchemaVersion(2));
        assert!(matches!(
            result.unwrap_err(),
            SchemaVersionedError::MigrationError { .. }
        ));
    }

    #[test]
    fn test_migration_no_path_error() {
        let registry = MigrationRegistry::new();
        let result = registry.migrate(vec![1], SchemaVersion(1), SchemaVersion(5));
        assert!(matches!(
            result.unwrap_err(),
            SchemaVersionedError::MigrationError { .. }
        ));
    }

    #[test]
    fn test_migration_backward_not_supported() {
        let mut registry = MigrationRegistry::new();
        registry.register(SchemaVersion(1), SchemaVersion(2), |data| Ok(data));

        assert!(!registry.has_path(SchemaVersion(2), SchemaVersion(1)));
    }

    // -----------------------------------------------------------------------
    // VersionedFileStore
    // -----------------------------------------------------------------------

    #[test]
    fn test_file_store_write_read_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let store = VersionedFileStore::new(
            tmp.path().to_path_buf(),
            SchemaVersion::V1,
            ChecksumAlgorithm::Sha256,
        );

        let original = TestPayload {
            name: "file-test".into(),
            value: 123,
        };

        store.write("mykey", &original).unwrap();
        assert!(store.exists("mykey"));

        let loaded: TestPayload = store
            .read("mykey", SchemaVersion::V1, SchemaVersion::V1)
            .unwrap();
        assert_eq!(loaded, original);
    }

    #[test]
    fn test_file_store_exists() {
        let tmp = TempDir::new().unwrap();
        let store = VersionedFileStore::new(
            tmp.path().to_path_buf(),
            SchemaVersion::V1,
            ChecksumAlgorithm::Sha256,
        );

        assert!(!store.exists("nonexistent"));

        store.write("exists-key", &42i64).unwrap();
        assert!(store.exists("exists-key"));
    }

    #[test]
    fn test_file_store_remove() {
        let tmp = TempDir::new().unwrap();
        let store = VersionedFileStore::new(
            tmp.path().to_path_buf(),
            SchemaVersion::V1,
            ChecksumAlgorithm::Sha256,
        );

        store.write("remove-key", &99i64).unwrap();
        assert!(store.exists("remove-key"));

        let removed = store.remove("remove-key").unwrap();
        assert!(removed);
        assert!(!store.exists("remove-key"));

        let removed_again = store.remove("remove-key").unwrap();
        assert!(!removed_again);
    }

    #[test]
    fn test_file_store_version_mismatch_on_read() {
        let tmp = TempDir::new().unwrap();
        let store = VersionedFileStore::new(
            tmp.path().to_path_buf(),
            SchemaVersion(1001),
            ChecksumAlgorithm::Sha256,
        );

        store
            .write(
                "v2-key",
                &TestPayload {
                    name: "v2".into(),
                    value: 1,
                },
            )
            .unwrap();

        let result: Result<TestPayload, _> =
            store.read("v2-key", SchemaVersion::V1, SchemaVersion::V1);
        assert!(matches!(
            result.unwrap_err(),
            SchemaVersionedError::VersionMismatch { .. }
        ));
    }

    #[test]
    fn test_file_store_checksum_corruption() {
        let tmp = TempDir::new().unwrap();
        let store = VersionedFileStore::new(
            tmp.path().to_path_buf(),
            SchemaVersion::V1,
            ChecksumAlgorithm::Sha256,
        );

        store
            .write(
                "corrupt-key",
                &TestPayload {
                    name: "corrupt".into(),
                    value: 1,
                },
            )
            .unwrap();

        let path = store.key_to_path("corrupt-key");
        let mut bytes = std::fs::read(&path).unwrap();
        let header_size = 5 + 32;
        bytes[header_size] ^= 0xFF;
        std::fs::write(&path, bytes).unwrap();

        let result: Result<TestPayload, _> =
            store.read("corrupt-key", SchemaVersion::V1, SchemaVersion::V1);
        assert!(matches!(
            result.unwrap_err(),
            SchemaVersionedError::ChecksumMismatch { .. }
        ));
    }

    #[test]
    fn test_file_store_backward_compatibility() {
        let tmp = TempDir::new().unwrap();

        let store_v1 = VersionedFileStore::new(
            tmp.path().to_path_buf(),
            SchemaVersion::V1,
            ChecksumAlgorithm::Sha256,
        );

        let original = TestPayload {
            name: "old-data".into(),
            value: 42,
        };
        store_v1.write("old-key", &original).unwrap();

        let store_v1_compat = VersionedFileStore::new(
            tmp.path().to_path_buf(),
            SchemaVersion::V1,
            ChecksumAlgorithm::Sha256,
        );

        let loaded: TestPayload = store_v1_compat
            .read("old-key", SchemaVersion::V1, SchemaVersion::V1)
            .unwrap();
        assert_eq!(loaded, original);
    }

    #[test]
    fn test_file_store_crc32() {
        let tmp = TempDir::new().unwrap();
        let store = VersionedFileStore::new(
            tmp.path().to_path_buf(),
            SchemaVersion::V1,
            ChecksumAlgorithm::Crc32,
        );

        let original = TestPayload {
            name: "crc-file".into(),
            value: 555,
        };
        store.write("crc-key", &original).unwrap();

        let loaded: TestPayload = store
            .read("crc-key", SchemaVersion::V1, SchemaVersion::V1)
            .unwrap();
        assert_eq!(loaded, original);
    }

    #[test]
    fn test_file_store_large_entry() {
        let tmp = TempDir::new().unwrap();
        let store = VersionedFileStore::new(
            tmp.path().to_path_buf(),
            SchemaVersion::V1,
            ChecksumAlgorithm::Sha256,
        );

        let large_data = vec![0xABu8; 100_000];
        store.write("large-key", &large_data).unwrap();

        let loaded: Vec<u8> = store
            .read("large-key", SchemaVersion::V1, SchemaVersion::V1)
            .unwrap();
        assert_eq!(loaded, large_data);
    }

    #[test]
    fn test_file_store_file_format_header() {
        let tmp = TempDir::new().unwrap();
        let store = VersionedFileStore::new(
            tmp.path().to_path_buf(),
            SchemaVersion(1042),
            ChecksumAlgorithm::Sha256,
        );

        store.write("format-key", &42i64).unwrap();

        let path = store.key_to_path("format-key");
        let bytes = std::fs::read(&path).unwrap();

        let version_bytes: [u8; 4] = bytes[..4].try_into().unwrap();
        let version = u32::from_le_bytes(version_bytes);
        assert_eq!(version, 1042);

        assert_eq!(bytes[4], 0);
    }

    #[test]
    fn test_file_store_read_nonexistent() {
        let tmp = TempDir::new().unwrap();
        let store = VersionedFileStore::new(
            tmp.path().to_path_buf(),
            SchemaVersion::V1,
            ChecksumAlgorithm::Sha256,
        );

        let result: Result<i64, _> =
            store.read("no-such-key", SchemaVersion::V1, SchemaVersion::V1);
        assert!(matches!(
            result.unwrap_err(),
            SchemaVersionedError::IoError(_)
        ));
    }

    #[test]
    fn test_file_store_corrupted_truncated_file() {
        let tmp = TempDir::new().unwrap();
        let store = VersionedFileStore::new(
            tmp.path().to_path_buf(),
            SchemaVersion::V1,
            ChecksumAlgorithm::Sha256,
        );

        let path = store.key_to_path("truncated-key");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, [0u8; 3]).unwrap();

        let result: Result<i64, _> =
            store.read("truncated-key", SchemaVersion::V1, SchemaVersion::V1);
        assert!(matches!(
            result.unwrap_err(),
            SchemaVersionedError::SerializationError(_)
        ));
    }

    // -----------------------------------------------------------------------
    // Integration: encode -> write -> read -> decode
    // -----------------------------------------------------------------------

    #[test]
    fn test_full_lifecycle_encode_write_read_decode() {
        let tmp = TempDir::new().unwrap();

        let original = TestPayload {
            name: "full-lifecycle".into(),
            value: 9999,
        };

        let payload =
            VersionedPayload::encode(&original, SchemaVersion::V1, ChecksumAlgorithm::Sha256)
                .unwrap();

        let store = VersionedFileStore::new(
            tmp.path().to_path_buf(),
            SchemaVersion::V1,
            ChecksumAlgorithm::Sha256,
        );

        std::fs::write(
            store.key_to_path("lifecycle-key"),
            [
                payload.schema_version.to_le_bytes().as_slice(),
                &[payload.checksum_algorithm][..],
                &payload.checksum,
                &payload.data,
            ]
            .concat(),
        )
        .unwrap();

        let read_payload: TestPayload = store
            .read("lifecycle-key", SchemaVersion::V1, SchemaVersion::V1)
            .unwrap();
        assert_eq!(read_payload, original);
    }
}
