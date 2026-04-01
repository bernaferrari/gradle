//! Secret hygiene utilities for gradle-substrate.
//!
//! Provides zeroizing secret buffers, redaction policies for logs,
//! XOR-obfuscated cache fields, and a credential store.
//!
//! # Security Note
//!
//! The `EncryptedCacheField` uses XOR-based obfuscation as a **placeholder
//! pattern only**. Production code MUST use AES-GCM or XChaCha20-Poly1305
//! from a proper cryptographic library (e.g. `ring` or `aes-gcm`).

use std::fmt;
use std::hint::black_box;
use std::ptr;
use std::sync::LazyLock;

use dashmap::DashMap;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Secret — zeroizing wrapper
// ---------------------------------------------------------------------------

/// A wrapper type that zeroizes its buffer on drop and redacts in debug/log output.
pub struct Secret {
    inner: Vec<u8>,
}

impl Secret {
    /// Create a new `Secret` from any data that can become a `Vec<u8>`.
    pub fn new(data: impl Into<Vec<u8>>) -> Self {
        Self { inner: data.into() }
    }

    /// Temporarily expose the raw bytes. The caller is responsible for
    /// zeroizing any copies after use.
    pub fn expose(&self) -> &[u8] {
        &self.inner
    }

    /// Expose the secret as a `String`. The returned `String` is **not**
    /// zeroized — use only for text secrets where the caller will handle it.
    pub fn expose_string(&self) -> String {
        String::from_utf8_lossy(&self.inner).into_owned()
    }
}

impl Clone for Secret {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        // Zeroize the buffer before deallocation.
        // Use volatile writes via black_box to prevent compiler optimization.
        for byte in self.inner.iter_mut() {
            unsafe {
                ptr::write_volatile(byte, 0u8);
            }
            black_box(byte);
        }
        self.inner.clear();
        self.inner.shrink_to(0);
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secret(*****)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "*****")
    }
}

// ---------------------------------------------------------------------------
// RedactionLevel
// ---------------------------------------------------------------------------

/// How aggressively to redact a secret value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedactionLevel {
    /// Replace the entire value with `*****`.
    Full,
    /// Show first 4 and last 4 characters: `sk-****-xyz1`.
    Partial,
    /// Show the SHA-256 hex digest of the value.
    Hash,
}

// ---------------------------------------------------------------------------
// RedactedValue
// ---------------------------------------------------------------------------

/// A value with redaction applied.
pub struct RedactedValue(pub String);

impl fmt::Display for RedactedValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// redact
// ---------------------------------------------------------------------------

/// Apply the given redaction level to a string value.
pub fn redact(value: &str, level: RedactionLevel) -> RedactedValue {
    match level {
        RedactionLevel::Full => RedactedValue("*****".to_string()),
        RedactionLevel::Partial => {
            if value.len() <= 8 {
                RedactedValue("*****".to_string())
            } else {
                let first4: String = value.chars().take(4).collect();
                let last4: String = value
                    .chars()
                    .rev()
                    .take(4)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                RedactedValue(format!("{}****{}", first4, last4))
            }
        }
        RedactionLevel::Hash => {
            let mut hasher = Sha256::new();
            hasher.update(value.as_bytes());
            let hex: String = hasher
                .finalize()
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect();
            RedactedValue(hex)
        }
    }
}

// ---------------------------------------------------------------------------
// SecretField
// ---------------------------------------------------------------------------

/// A named secret field that can be redacted in structured data.
pub struct SecretField {
    pub name: String,
    pub value: Secret,
}

impl SecretField {
    /// Create a new secret field.
    pub fn new(name: impl Into<String>, value: Secret) -> Self {
        Self {
            name: name.into(),
            value,
        }
    }

    /// Return a redacted string representation of the field value.
    pub fn redacted_display(&self, level: RedactionLevel) -> String {
        redact(&self.value.expose_string(), level).0
    }
}

// ---------------------------------------------------------------------------
// RedactionPolicy
// ---------------------------------------------------------------------------

/// A policy that defines which field names should be redacted.
///
/// Field names are matched against glob-like patterns (case-insensitive,
/// `*` matches any sequence of characters).
#[derive(Clone)]
pub struct RedactionPolicy {
    field_patterns: Vec<String>,
    default_level: RedactionLevel,
}

impl RedactionPolicy {
    /// Create a new policy.
    pub fn new(field_patterns: Vec<String>, default_level: RedactionLevel) -> Self {
        Self {
            field_patterns,
            default_level,
        }
    }

    /// Check whether a field name matches any pattern in this policy.
    pub fn should_redact(&self, field_name: &str) -> bool {
        let lower = field_name.to_lowercase();
        self.field_patterns
            .iter()
            .any(|pattern| glob_match(pattern, &lower))
    }

    /// Redact a field value if the field name matches the policy.
    pub fn redact_field(&self, name: &str, value: &str) -> String {
        if self.should_redact(name) {
            redact(value, self.default_level).0
        } else {
            value.to_string()
        }
    }
}

/// Simple glob matcher supporting only `*` wildcards (case-insensitive input expected).
fn glob_match(pattern: &str, text: &str) -> bool {
    let pat_bytes = pattern.as_bytes();
    let txt_bytes = text.as_bytes();
    let mut pi = 0usize;
    let mut ti = 0usize;
    let mut star_pi: Option<usize> = None;
    let mut star_ti: Option<usize> = None;

    while ti < txt_bytes.len() {
        if pi < pat_bytes.len() && pat_bytes[pi] == b'*' {
            star_pi = Some(pi);
            star_ti = Some(ti);
            pi += 1;
        } else if pi < pat_bytes.len() && pat_bytes[pi] == txt_bytes[ti] {
            pi += 1;
            ti += 1;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            star_ti = star_ti.map(|st| st + 1);
            ti = star_ti.unwrap();
        } else {
            return false;
        }
    }

    while pi < pat_bytes.len() && pat_bytes[pi] == b'*' {
        pi += 1;
    }

    pi == pat_bytes.len()
}

/// Default redaction policy matching common secret field names.
pub static DEFAULT_REDACTION_POLICY: LazyLock<RedactionPolicy> =
    LazyLock::new(|| RedactionPolicy {
        field_patterns: vec![
            "*password*".to_string(),
            "*secret*".to_string(),
            "*token*".to_string(),
            "*key*".to_string(),
            "*credential*".to_string(),
            "*auth*".to_string(),
        ],
        default_level: RedactionLevel::Full,
    });

// ---------------------------------------------------------------------------
// RedactingFormatter
// ---------------------------------------------------------------------------

/// A log formatter that applies a redaction policy to field values.
pub struct RedactingFormatter {
    policy: RedactionPolicy,
}

impl RedactingFormatter {
    /// Create a new formatter with the given policy.
    pub fn new(policy: RedactionPolicy) -> Self {
        Self { policy }
    }

    /// Format a single field/value pair, redacting if the policy matches.
    pub fn format_value(&self, field: &str, value: &str) -> String {
        let redacted = self.policy.redact_field(field, value);
        format!("{}={}", field, redacted)
    }
}

// ---------------------------------------------------------------------------
// EncryptedCacheField
// ---------------------------------------------------------------------------

/// A wrapper for cache fields that should be encrypted at rest.
///
/// # Security Warning
///
/// This implementation uses **XOR-based obfuscation** which is NOT
/// cryptographically secure. It demonstrates the pattern for
/// encrypt/decrypt lifecycle. Production code MUST use AES-GCM or
/// XChaCha20-Poly1305 (e.g. from the `ring` or `aes-gcm` crate).
#[derive(Debug, Clone)]
pub struct EncryptedCacheField {
    ciphertext: Vec<u8>,
    nonce: [u8; 12],
}

impl EncryptedCacheField {
    /// Encrypt a serializable value with the given key.
    ///
    /// The value is serialized with bincode, then XOR'd with the key
    /// (repeating the key as needed). A nonce is generated from the
    /// ciphertext length for demonstration purposes.
    pub fn encrypt<T: Serialize>(value: &T, key: &[u8]) -> Result<Self, SecretHygieneError> {
        if key.is_empty() {
            return Err(SecretHygieneError::EmptyKey);
        }
        let plaintext = bincode::serialize(value)
            .map_err(|e| SecretHygieneError::Serialization(e.to_string()))?;
        let ciphertext: Vec<u8> = plaintext
            .iter()
            .enumerate()
            .map(|(i, b)| b ^ key[i % key.len()])
            .collect();
        let mut nonce = [0u8; 12];
        let len_bytes = ciphertext.len().to_le_bytes();
        nonce[..8].copy_from_slice(&len_bytes[..8]);
        nonce[8..12].copy_from_slice(&key[..4.min(key.len())]);
        Ok(Self { ciphertext, nonce })
    }

    /// Decrypt the field back to the original type.
    pub fn decrypt<T: DeserializeOwned>(&self, key: &[u8]) -> Result<T, SecretHygieneError> {
        if key.is_empty() {
            return Err(SecretHygieneError::EmptyKey);
        }
        let plaintext: Vec<u8> = self
            .ciphertext
            .iter()
            .enumerate()
            .map(|(i, b)| b ^ key[i % key.len()])
            .collect();
        bincode::deserialize(&plaintext)
            .map_err(|e| SecretHygieneError::Deserialization(e.to_string()))
    }

    /// Return the raw ciphertext (for inspection / persistence).
    pub fn ciphertext(&self) -> &[u8] {
        &self.ciphertext
    }

    /// Return the nonce.
    pub fn nonce(&self) -> &[u8; 12] {
        &self.nonce
    }
}

// ---------------------------------------------------------------------------
// CredentialStore
// ---------------------------------------------------------------------------

/// An in-memory store for build credentials backed by `DashMap`.
pub struct CredentialStore {
    credentials: DashMap<String, Secret>,
}

impl CredentialStore {
    /// Create an empty credential store.
    pub fn new() -> Self {
        Self {
            credentials: DashMap::new(),
        }
    }

    /// Store a secret under the given name.
    pub fn store(&self, name: &str, secret: Secret) {
        self.credentials.insert(name.to_string(), secret);
    }

    /// Retrieve a clone of the secret. The clone has its own zeroizing
    /// lifecycle independent of the stored value.
    pub fn get(&self, name: &str) -> Option<Secret> {
        self.credentials
            .get(name)
            .map(|entry| entry.value().clone())
    }

    /// Remove and return the secret, zeroizing the stored copy.
    pub fn remove(&self, name: &str) -> Option<Secret> {
        self.credentials.remove(name).map(|(_, v)| v)
    }

    /// Return all credential names.
    pub fn names(&self) -> Vec<String> {
        self.credentials
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// Clear all credentials, zeroizing each one first.
    pub fn clear(&self) {
        self.credentials.iter_mut().for_each(|mut entry| {
            let secret = entry.value_mut();
            for byte in secret.inner.iter_mut() {
                unsafe {
                    ptr::write_volatile(byte, 0u8);
                }
                black_box(byte);
            }
            secret.inner.clear();
            secret.inner.shrink_to(0);
        });
        self.credentials.clear();
    }
}

impl Default for CredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// URL credential redaction
// ---------------------------------------------------------------------------

/// Redact credentials from a URL string.
///
/// `https://user:password@host.com/path` → `https://user:*****@host.com/path`
pub fn redact_url_credentials(url: &str) -> String {
    // Find the scheme separator "://"
    let scheme_end = match url.find("://") {
        Some(pos) => pos + 3,
        None => return url.to_string(),
    };
    let after_scheme = &url[scheme_end..];

    // Find the @ separator (credentials end before host)
    let at_pos = match after_scheme.find('@') {
        Some(pos) => pos,
        None => return url.to_string(),
    };

    let credentials = &after_scheme[..at_pos];
    let rest = &after_scheme[at_pos..];

    // Split credentials into user:password
    if let Some(colon_pos) = credentials.find(':') {
        let user = &credentials[..colon_pos];
        format!("{}{}:*****{}", &url[..scheme_end], user, rest)
    } else {
        // No password, just a username — still redact
        format!("{}*****{}", &url[..scheme_end], rest)
    }
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur in secret hygiene operations.
#[derive(Debug, thiserror::Error)]
pub enum SecretHygieneError {
    #[error("encryption key must not be empty")]
    EmptyKey,
    #[error("serialization failed: {0}")]
    Serialization(String),
    #[error("deserialization failed: {0}")]
    Deserialization(String),
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(non_snake_case)]
mod tests {
    use super::*;
    use serde::Deserialize;

    // --- Secret tests ---

    #[test]
    fn test_secret_new_and_expose() {
        let secret = Secret::new(b"my-password");
        assert_eq!(secret.expose(), b"my-password");
    }

    #[test]
    fn test_secret_expose_string() {
        let secret = Secret::new("hello-world");
        assert_eq!(secret.expose_string(), "hello-world");
    }

    #[test]
    fn test_secret_debug_is_redacted() {
        let secret = Secret::new("super-secret-value");
        let debug_str = format!("{:?}", secret);
        assert_eq!(debug_str, "Secret(*****)");
    }

    #[test]
    fn test_secret_display_is_redacted() {
        let secret = Secret::new("super-secret-value");
        let display_str = format!("{}", secret);
        assert_eq!(display_str, "*****");
    }

    #[test]
    fn test_secret_clone_is_independent() {
        let original = Secret::new(b"clone-test");
        let cloned = original.clone();
        assert_eq!(original.expose(), b"clone-test");
        assert_eq!(cloned.expose(), b"clone-test");
        // Both exist and have the same value
        drop(cloned);
        // Original should still be valid
        assert_eq!(original.expose(), b"clone-test");
    }

    #[test]
    fn test_secret_zeroizes_on_drop() {
        // We can't directly inspect memory after drop, but we can verify
        // the Drop impl compiles and runs without panicking.
        let secret = Secret::new(vec![0xAB; 64]);
        assert_eq!(secret.expose().len(), 64);
        assert!(secret.expose().iter().all(|&b| b == 0xAB));
        drop(secret);
        // After drop the buffer should be zeroized — we trust the Drop impl.
    }

    // --- RedactionLevel tests ---

    #[test]
    fn test_redact_full() {
        let result = redact("my-api-key-12345", RedactionLevel::Full);
        assert_eq!(result.0, "*****");
    }

    #[test]
    fn test_redact_partial_long_value() {
        let result = redact("sk-abc-12345-xyz1", RedactionLevel::Partial);
        assert_eq!(result.0, "sk-a****xyz1");
    }

    #[test]
    fn test_redact_partial_short_value() {
        let result = redact("short", RedactionLevel::Partial);
        assert_eq!(result.0, "*****");
    }

    #[test]
    fn test_redact_hash() {
        let result = redact("test-value", RedactionLevel::Hash);
        // SHA-256 hex is 64 characters
        assert_eq!(result.0.len(), 64);
        // All hex characters
        assert!(result.0.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_redact_hash_is_deterministic() {
        let r1 = redact("same-value", RedactionLevel::Hash);
        let r2 = redact("same-value", RedactionLevel::Hash);
        assert_eq!(r1.0, r2.0);
    }

    // --- RedactionPolicy tests ---

    #[test]
    fn test_default_policy_matches_password() {
        assert!(DEFAULT_REDACTION_POLICY.should_redact("password"));
        assert!(DEFAULT_REDACTION_POLICY.should_redact("db_password"));
        assert!(DEFAULT_REDACTION_POLICY.should_redact("PASSWORD"));
        assert!(DEFAULT_REDACTION_POLICY.should_redact("MyPassword123"));
    }

    #[test]
    fn test_default_policy_matches_secret() {
        assert!(DEFAULT_REDACTION_POLICY.should_redact("secret"));
        assert!(DEFAULT_REDACTION_POLICY.should_redact("api_secret"));
        assert!(DEFAULT_REDACTION_POLICY.should_redact("SECRET_KEY"));
    }

    #[test]
    fn test_default_policy_matches_token() {
        assert!(DEFAULT_REDACTION_POLICY.should_redact("token"));
        assert!(DEFAULT_REDACTION_POLICY.should_redact("auth_token"));
        assert!(DEFAULT_REDACTION_POLICY.should_redact("TOKEN_VALUE"));
    }

    #[test]
    fn test_default_policy_matches_key() {
        assert!(DEFAULT_REDACTION_POLICY.should_redact("key"));
        assert!(DEFAULT_REDACTION_POLICY.should_redact("api_key"));
        assert!(DEFAULT_REDACTION_POLICY.should_redact("KEY_ID"));
    }

    #[test]
    fn test_default_policy_matches_credential() {
        assert!(DEFAULT_REDACTION_POLICY.should_redact("credential"));
        assert!(DEFAULT_REDACTION_POLICY.should_redact("user_credential"));
    }

    #[test]
    fn test_default_policy_matches_auth() {
        assert!(DEFAULT_REDACTION_POLICY.should_redact("auth"));
        assert!(DEFAULT_REDACTION_POLICY.should_redact("authorization"));
        assert!(DEFAULT_REDACTION_POLICY.should_redact("AUTH_HEADER"));
    }

    #[test]
    fn test_default_policy_does_not_match_safe_fields() {
        assert!(!DEFAULT_REDACTION_POLICY.should_redact("username"));
        assert!(!DEFAULT_REDACTION_POLICY.should_redact("host"));
        assert!(!DEFAULT_REDACTION_POLICY.should_redact("port"));
        assert!(!DEFAULT_REDACTION_POLICY.should_redact("path"));
        assert!(!DEFAULT_REDACTION_POLICY.should_redact("name"));
    }

    #[test]
    fn test_redact_field_applies_policy() {
        let redacted = DEFAULT_REDACTION_POLICY.redact_field("password", "hunter2");
        assert_eq!(redacted, "*****");
        let not_redacted = DEFAULT_REDACTION_POLICY.redact_field("username", "admin");
        assert_eq!(not_redacted, "admin");
    }

    // --- SecretField tests ---

    #[test]
    fn test_secret_field_redacted_display() {
        let field = SecretField::new("api_key", Secret::new("sk-abc-12345-xyz1"));
        let full = field.redacted_display(RedactionLevel::Full);
        assert_eq!(full, "*****");
        let partial = field.redacted_display(RedactionLevel::Partial);
        assert_eq!(partial, "sk-a****xyz1");
    }

    // --- RedactingFormatter tests ---

    #[test]
    fn test_redacting_formatter_redacts_secret_fields() {
        let formatter = RedactingFormatter::new((*DEFAULT_REDACTION_POLICY).clone());
        let output = formatter.format_value("password", "hunter2");
        assert_eq!(output, "password=*****");
    }

    #[test]
    fn test_redacting_formatter_preserves_safe_fields() {
        let formatter = RedactingFormatter::new((*DEFAULT_REDACTION_POLICY).clone());
        let output = formatter.format_value("username", "admin");
        assert_eq!(output, "username=admin");
    }

    #[test]
    fn test_redacting_formatter_mixed_fields() {
        let formatter = RedactingFormatter::new((*DEFAULT_REDACTION_POLICY).clone());
        let fields = vec![
            ("username", "admin"),
            ("password", "hunter2"),
            ("host", "localhost"),
            ("api_token", "tok-12345"),
        ];
        let formatted: Vec<String> = fields
            .iter()
            .map(|(k, v)| formatter.format_value(k, v))
            .collect();
        assert_eq!(formatted[0], "username=admin");
        assert_eq!(formatted[1], "password=*****");
        assert_eq!(formatted[2], "host=localhost");
        assert_eq!(formatted[3], "api_token=*****");
    }

    // --- EncryptedCacheField tests ---

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct TestCacheData {
        url: String,
        etag: String,
    }

    #[test]
    fn test_encrypted_cache_field_round_trip() {
        let data = TestCacheData {
            url: "https://example.com/artifact.jar".to_string(),
            etag: "\"abc123\"".to_string(),
        };
        let key = b"test-key-1234567890";
        let encrypted = EncryptedCacheField::encrypt(&data, key).unwrap();
        let decrypted: TestCacheData = encrypted.decrypt(key).unwrap();
        assert_eq!(decrypted, data);
    }

    #[test]
    fn test_encrypted_cache_field_wrong_key_fails() {
        let data = TestCacheData {
            url: "https://example.com/artifact.jar".to_string(),
            etag: "\"abc123\"".to_string(),
        };
        let key = b"test-key-1234567890";
        let encrypted = EncryptedCacheField::encrypt(&data, key).unwrap();
        let wrong_key = b"wrong-key-1234567890";
        let result: Result<TestCacheData, _> = encrypted.decrypt(wrong_key);
        assert!(result.is_err());
    }

    #[test]
    fn test_encrypted_cache_field_empty_key_errors() {
        let data = TestCacheData {
            url: "test".to_string(),
            etag: "test".to_string(),
        };
        let result = EncryptedCacheField::encrypt(&data, &[]);
        assert!(matches!(result, Err(SecretHygieneError::EmptyKey)));
    }

    #[test]
    fn test_encrypted_cache_field_ciphertext_differs_from_plaintext() {
        let data = TestCacheData {
            url: "https://example.com".to_string(),
            etag: "etag".to_string(),
        };
        let key = b"some-key";
        let encrypted = EncryptedCacheField::encrypt(&data, key).unwrap();
        let plaintext = bincode::serialize(&data).unwrap();
        // XOR with non-zero key should produce different bytes
        assert_ne!(encrypted.ciphertext(), plaintext.as_slice());
    }

    #[test]
    fn test_encrypted_cache_field_nonce_is_set() {
        let data = TestCacheData {
            url: "test".to_string(),
            etag: "test".to_string(),
        };
        let key = b"nonce-test-key";
        let encrypted = EncryptedCacheField::encrypt(&data, key).unwrap();
        assert_eq!(encrypted.nonce().len(), 12);
        // Nonce should not be all zeros
        assert!(encrypted.nonce().iter().any(|&b| b != 0));
    }

    // --- CredentialStore tests ---

    #[test]
    fn test_credential_store_store_and_get() {
        let store = CredentialStore::new();
        store.store("db_pass", Secret::new("hunter2"));
        let retrieved = store.get("db_pass").unwrap();
        assert_eq!(retrieved.expose_string(), "hunter2");
    }

    #[test]
    fn test_credential_store_get_returns_clone() {
        let store = CredentialStore::new();
        store.store("api_key", Secret::new("sk-12345"));
        let clone1 = store.get("api_key").unwrap();
        let clone2 = store.get("api_key").unwrap();
        assert_eq!(clone1.expose_string(), "sk-12345");
        assert_eq!(clone2.expose_string(), "sk-12345");
        // Dropping clones should not affect the stored value
        drop(clone1);
        let still_there = store.get("api_key").unwrap();
        assert_eq!(still_there.expose_string(), "sk-12345");
    }

    #[test]
    fn test_credential_store_remove() {
        let store = CredentialStore::new();
        store.store("token", Secret::new("tok-abc"));
        let removed = store.remove("token").unwrap();
        assert_eq!(removed.expose_string(), "tok-abc");
        assert!(store.get("token").is_none());
    }

    #[test]
    fn test_credential_store_names() {
        let store = CredentialStore::new();
        store.store("a", Secret::new("1"));
        store.store("b", Secret::new("2"));
        store.store("c", Secret::new("3"));
        let mut names = store.names();
        names.sort();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn test_credential_store_clear() {
        let store = CredentialStore::new();
        store.store("x", Secret::new("secret-x"));
        store.store("y", Secret::new("secret-y"));
        assert_eq!(store.names().len(), 2);
        store.clear();
        assert_eq!(store.names().len(), 0);
        assert!(store.get("x").is_none());
        assert!(store.get("y").is_none());
    }

    #[test]
    fn test_credential_store_get_missing() {
        let store = CredentialStore::new();
        assert!(store.get("nonexistent").is_none());
    }

    // --- URL credential redaction tests ---

    #[test]
    fn test_redact_url_credentials_with_password() {
        let url = "https://user:password@host.com/path";
        let redacted = redact_url_credentials(url);
        assert_eq!(redacted, "https://user:*****@host.com/path");
    }

    #[test]
    fn test_redact_url_credentials_no_credentials() {
        let url = "https://host.com/path";
        let redacted = redact_url_credentials(url);
        assert_eq!(redacted, "https://host.com/path");
    }

    #[test]
    fn test_redact_url_credentials_complex_url() {
        let url = "https://deployer:ghp_abc123@github.com/org/repo.git";
        let redacted = redact_url_credentials(url);
        assert_eq!(redacted, "https://deployer:*****@github.com/org/repo.git");
    }

    #[test]
    fn test_redact_url_credentials_maven_repo() {
        let url = "https://admin:s3cret@maven.internal.com/releases/";
        let redacted = redact_url_credentials(url);
        assert_eq!(redacted, "https://admin:*****@maven.internal.com/releases/");
    }

    #[test]
    fn test_redact_url_credentials_http() {
        let url = "http://user:pass@example.com";
        let redacted = redact_url_credentials(url);
        assert_eq!(redacted, "http://user:*****@example.com");
    }

    #[test]
    fn test_redact_url_credentials_no_scheme() {
        let url = "user:pass@host.com";
        let redacted = redact_url_credentials(url);
        // No "://" found, returns unchanged
        assert_eq!(redacted, "user:pass@host.com");
    }

    // --- RedactedValue Display test ---

    #[test]
    fn test_redacted_value_display() {
        let rv = RedactedValue("redacted".to_string());
        assert_eq!(format!("{}", rv), "redacted");
    }

    // --- glob_match edge cases ---

    #[test]
    fn test_glob_match_exact() {
        assert!(glob_match("password", "password"));
        assert!(!glob_match("password", "pass"));
    }

    #[test]
    fn test_glob_match_leading_wildcard() {
        assert!(glob_match("*password", "db_password"));
        assert!(glob_match("*password", "password"));
        assert!(!glob_match("*password", "passwords_extra"));
    }

    #[test]
    fn test_glob_match_trailing_wildcard() {
        assert!(glob_match("password*", "password"));
        assert!(glob_match("password*", "passwords"));
        assert!(!glob_match("password*", "my_password"));
    }

    #[test]
    fn test_glob_match_both_wildcards() {
        assert!(glob_match("*password*", "my_password_hash"));
        assert!(glob_match("*password*", "password"));
        assert!(glob_match("*password*", "passwords"));
        assert!(!glob_match("*password*", "pass"));
    }
}
