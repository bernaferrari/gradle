//! Compact serde storage codec for persistent Rust substrate state.

use serde::{de::DeserializeOwned, Serialize};

/// Serialize a serde value into compact postcard bytes.
pub fn serialize<T>(value: &T) -> postcard::Result<Vec<u8>>
where
    T: Serialize + ?Sized,
{
    postcard::to_allocvec(value)
}

/// Deserialize a serde value from compact postcard bytes.
pub fn deserialize<T>(bytes: &[u8]) -> postcard::Result<T>
where
    T: DeserializeOwned,
{
    postcard::from_bytes(bytes)
}
