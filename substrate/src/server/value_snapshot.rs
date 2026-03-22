use md5::{Digest, Md5};
use tonic::{Request, Response, Status};

use crate::proto::{
    value_snapshot_service_server::ValueSnapshotService, PropertyValue, SnapshotValuesRequest,
    SnapshotValuesResponse, ValueSnapshotResult,
};

/// Rust-native value snapshotting service.
/// Computes fingerprints for input properties, replacing Java's DefaultValueSnapshotter.
/// Supports Gradle-compatible normalization: sorted collections, path normalization,
/// and deterministic serialization.
#[derive(Default)]
pub struct ValueSnapshotServiceImpl;

impl ValueSnapshotServiceImpl {
    pub fn new() -> Self {
        Self
    }

    /// Check if a type name represents a Gradle FileCollection or similar ordered collection
    /// where the serialization order is not deterministic.
    fn is_ordered_collection_type(type_name: &str) -> bool {
        matches!(
            type_name,
            "org.gradle.api.file.FileCollection"
                | "org.gradle.api.file.ConfigurableFileCollection"
                | "org.gradle.api.file.FileTree"
                | "org.gradle.api.file.SourceDirectorySet"
                | "org.gradle.api.file.DirectorySet"
        )
    }

    /// Normalize a path for fingerprinting: forward slashes, no trailing slash, lowercase drive on Windows.
    fn normalize_path(p: &str) -> String {
        let p = p.replace('\\', "/");
        let p = p.trim_end_matches('/');
        p.to_string()
    }

    /// Normalize a semicolon-separated path list (FileCollection serialization).
    /// Sorts entries, deduplicates, normalizes each path.
    fn normalize_path_list(s: &str) -> String {
        if s.is_empty() {
            return String::new();
        }
        let mut paths: Vec<String> = s
            .split(';')
            .filter(|p| !p.is_empty())
            .map(Self::normalize_path)
            .collect();
        paths.sort();
        paths.dedup();
        paths.join(";")
    }

    /// Normalize a comma-separated value list.
    /// Sorts entries, deduplicates.
    fn normalize_value_list(s: &str) -> String {
        if s.is_empty() {
            return String::new();
        }
        let mut values: Vec<&str> = s.split(',').map(|v| v.trim()).filter(|v| !v.is_empty()).collect();
        values.sort();
        values.dedup();
        values.join(",")
    }

    /// Normalize a key=value map serialization (e.g., "k1=v1;k2=v2").
    /// Sorts by key for determinism.
    fn normalize_map(s: &str) -> String {
        if s.is_empty() {
            return String::new();
        }
        let mut entries: Vec<(&str, &str)> = s
            .split(';')
            .filter_map(|entry| {
                let mut parts = entry.splitn(2, '=');
                let key = parts.next()?.trim();
                let value = parts.next()?.trim();
                Some((key, value))
            })
            .collect();
        entries.sort_by_key(|(k, _)| *k);
        entries
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join(";")
    }

    fn fingerprint_value(prop: &PropertyValue) -> Vec<u8> {
        let mut hasher = Md5::new();

        // Include type name in fingerprint for type safety
        hasher.update(prop.type_name.as_bytes());
        hasher.update(b"|");

        // Include value content
        match &prop.value {
            Some(crate::proto::property_value::Value::StringValue(s)) => {
                hasher.update(b"S:");
                if Self::is_ordered_collection_type(&prop.type_name) {
                    // FileCollection: normalize as path list
                    let normalized = Self::normalize_path_list(s);
                    hasher.update(normalized.as_bytes());
                } else {
                    hasher.update(s.as_bytes());
                }
            }
            Some(crate::proto::property_value::Value::LongValue(l)) => {
                hasher.update(b"L:");
                hasher.update(l.to_le_bytes());
            }
            Some(crate::proto::property_value::Value::BoolValue(b)) => {
                hasher.update(b"B:");
                hasher.update([*b as u8]);
            }
            Some(crate::proto::property_value::Value::BinaryValue(bytes)) => {
                hasher.update(b"X:");
                hasher.update(bytes);
            }
            Some(crate::proto::property_value::Value::ListValue(s)) => {
                hasher.update(b"[");
                // Normalize list: sort entries for determinism
                let normalized = Self::normalize_value_list(s);
                hasher.update(normalized.as_bytes());
                hasher.update(b"]");
            }
            Some(crate::proto::property_value::Value::MapValue(s)) => {
                hasher.update(b"{");
                // Normalize map: sort by key for determinism
                let normalized = Self::normalize_map(s);
                hasher.update(normalized.as_bytes());
                hasher.update(b"}");
            }
            None => {
                hasher.update(b"null");
            }
        }

        hasher.finalize().to_vec()
    }
}

#[tonic::async_trait]
impl ValueSnapshotService for ValueSnapshotServiceImpl {
    async fn snapshot_values(
        &self,
        request: Request<SnapshotValuesRequest>,
    ) -> Result<Response<SnapshotValuesResponse>, Status> {
        let req = request.into_inner();
        let mut results = Vec::new();
        let mut composite_hasher = Md5::new();

        // Include implementation fingerprint in composite hash
        if !req.implementation_fingerprint.is_empty() {
            composite_hasher.update(req.implementation_fingerprint.as_bytes());
            composite_hasher.update(b"|");
        }

        // Sort by property name for deterministic ordering
        let mut sorted_values: Vec<_> = req.values.iter().collect();
        sorted_values.sort_by_key(|v| v.name.as_str());

        for prop in sorted_values {
            let fingerprint = Self::fingerprint_value(prop);
            composite_hasher.update(prop.name.as_bytes());
            composite_hasher.update(b"=");
            composite_hasher.update(&fingerprint);
            composite_hasher.update(b";");

            results.push(ValueSnapshotResult {
                name: prop.name.clone(),
                fingerprint,
            });
        }

        let composite_hash = composite_hasher.finalize().to_vec();

        Ok(Response::new(SnapshotValuesResponse {
            results,
            composite_hash,
            error_message: String::new(),
            success: true,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::property_value::Value;

    #[tokio::test]
    async fn test_snapshot_string_values() {
        let svc = ValueSnapshotServiceImpl::new();

        let resp = svc
            .snapshot_values(Request::new(SnapshotValuesRequest {
                values: vec![
                    PropertyValue {
                        name: "source".to_string(),
                        value: Some(Value::StringValue("src/main/java".to_string())),
                        type_name: "java.lang.String".to_string(),
                    },
                    PropertyValue {
                        name: "encoding".to_string(),
                        value: Some(Value::StringValue("UTF-8".to_string())),
                        type_name: "java.lang.String".to_string(),
                    },
                ],
                implementation_fingerprint: "impl-hash-123".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        assert_eq!(resp.results.len(), 2);
        assert!(!resp.composite_hash.is_empty());
        assert_eq!(resp.results[0].name, "encoding"); // sorted alphabetically
        assert_eq!(resp.results[1].name, "source");
    }

    #[tokio::test]
    async fn test_snapshot_mixed_types() {
        let svc = ValueSnapshotServiceImpl::new();

        let resp = svc
            .snapshot_values(Request::new(SnapshotValuesRequest {
                values: vec![
                    PropertyValue {
                        name: "debug".to_string(),
                        value: Some(Value::BoolValue(true)),
                        type_name: "boolean".to_string(),
                    },
                    PropertyValue {
                        name: "version".to_string(),
                        value: Some(Value::LongValue(42)),
                        type_name: "java.lang.Integer".to_string(),
                    },
                    PropertyValue {
                        name: "classpaths".to_string(),
                        value: Some(Value::ListValue("cp1.jar;cp2.jar".to_string())),
                        type_name: "java.util.List".to_string(),
                    },
                ],
                implementation_fingerprint: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        assert_eq!(resp.results.len(), 3);
        assert!(!resp.composite_hash.is_empty());
    }

    #[tokio::test]
    async fn test_snapshot_determinism() {
        let svc = ValueSnapshotServiceImpl::new();

        let make_request = || SnapshotValuesRequest {
            values: vec![
                PropertyValue {
                    name: "z".to_string(),
                    value: Some(Value::StringValue("last".to_string())),
                    type_name: "java.lang.String".to_string(),
                },
                PropertyValue {
                    name: "a".to_string(),
                    value: Some(Value::StringValue("first".to_string())),
                    type_name: "java.lang.String".to_string(),
                },
            ],
            implementation_fingerprint: "impl".to_string(),
        };

        let resp1 = svc.snapshot_values(Request::new(make_request())).await.unwrap().into_inner();
        let resp2 = svc.snapshot_values(Request::new(make_request())).await.unwrap().into_inner();

        assert_eq!(resp1.composite_hash, resp2.composite_hash);
        assert_eq!(resp1.results[0].fingerprint, resp2.results[0].fingerprint);
    }

    #[test]
    fn test_fingerprint_value_type_safety() {
        // Same value content but different types should produce different fingerprints
        let prop_str = PropertyValue {
            name: "test".to_string(),
            value: Some(Value::StringValue("42".to_string())),
            type_name: "java.lang.String".to_string(),
        };
        let prop_long = PropertyValue {
            name: "test".to_string(),
            value: Some(Value::LongValue(42)),
            type_name: "java.lang.Integer".to_string(),
        };

        let fp_str = ValueSnapshotServiceImpl::fingerprint_value(&prop_str);
        let fp_long = ValueSnapshotServiceImpl::fingerprint_value(&prop_long);

        assert_ne!(fp_str, fp_long, "Different types should produce different fingerprints");
    }

    #[test]
    fn test_file_collection_order_independence() {
        // FileCollection paths in different order should produce same fingerprint
        let prop_a = PropertyValue {
            name: "classpath".to_string(),
            value: Some(Value::StringValue("/a.jar;/b.jar;/c.jar".to_string())),
            type_name: "org.gradle.api.file.FileCollection".to_string(),
        };
        let prop_b = PropertyValue {
            name: "classpath".to_string(),
            value: Some(Value::StringValue("/c.jar;/a.jar;/b.jar".to_string())),
            type_name: "org.gradle.api.file.FileCollection".to_string(),
        };

        let fp_a = ValueSnapshotServiceImpl::fingerprint_value(&prop_a);
        let fp_b = ValueSnapshotServiceImpl::fingerprint_value(&prop_b);
        assert_eq!(fp_a, fp_b, "FileCollection paths in different order should have same fingerprint");
    }

    #[test]
    fn test_file_collection_dedup() {
        // Duplicate paths should not affect fingerprint
        let prop_unique = PropertyValue {
            name: "cp".to_string(),
            value: Some(Value::StringValue("/a.jar;/b.jar".to_string())),
            type_name: "org.gradle.api.file.FileCollection".to_string(),
        };
        let prop_dup = PropertyValue {
            name: "cp".to_string(),
            value: Some(Value::StringValue("/a.jar;/b.jar;/a.jar".to_string())),
            type_name: "org.gradle.api.file.FileCollection".to_string(),
        };

        let fp_unique = ValueSnapshotServiceImpl::fingerprint_value(&prop_unique);
        let fp_dup = ValueSnapshotServiceImpl::fingerprint_value(&prop_dup);
        assert_eq!(fp_unique, fp_dup, "Duplicate FileCollection paths should be deduplicated");
    }

    #[test]
    fn test_non_file_collection_string_unchanged() {
        // Regular strings should NOT be sorted
        let prop1 = PropertyValue {
            name: "s".to_string(),
            value: Some(Value::StringValue("hello world".to_string())),
            type_name: "java.lang.String".to_string(),
        };
        let prop2 = PropertyValue {
            name: "s".to_string(),
            value: Some(Value::StringValue("world hello".to_string())),
            type_name: "java.lang.String".to_string(),
        };

        let fp1 = ValueSnapshotServiceImpl::fingerprint_value(&prop1);
        let fp2 = ValueSnapshotServiceImpl::fingerprint_value(&prop2);
        assert_ne!(fp1, fp2, "Regular strings should not be normalized");
    }

    #[test]
    fn test_list_normalization() {
        // List values should be sorted for determinism
        let prop_a = PropertyValue {
            name: "opts".to_string(),
            value: Some(Value::ListValue("z,a,m".to_string())),
            type_name: "java.util.List".to_string(),
        };
        let prop_b = PropertyValue {
            name: "opts".to_string(),
            value: Some(Value::ListValue("a,m,z".to_string())),
            type_name: "java.util.List".to_string(),
        };

        let fp_a = ValueSnapshotServiceImpl::fingerprint_value(&prop_a);
        let fp_b = ValueSnapshotServiceImpl::fingerprint_value(&prop_b);
        assert_eq!(fp_a, fp_b, "List values should be sorted");
    }

    #[test]
    fn test_map_normalization() {
        // Map values should be sorted by key
        let prop_a = PropertyValue {
            name: "env".to_string(),
            value: Some(Value::MapValue("z=3;a=1;m=2".to_string())),
            type_name: "java.util.Map".to_string(),
        };
        let prop_b = PropertyValue {
            name: "env".to_string(),
            value: Some(Value::MapValue("a=1;m=2;z=3".to_string())),
            type_name: "java.util.Map".to_string(),
        };

        let fp_a = ValueSnapshotServiceImpl::fingerprint_value(&prop_a);
        let fp_b = ValueSnapshotServiceImpl::fingerprint_value(&prop_b);
        assert_eq!(fp_a, fp_b, "Map values should be sorted by key");
    }

    #[test]
    fn test_path_normalization() {
        assert_eq!(
            ValueSnapshotServiceImpl::normalize_path("/foo/bar/"),
            "/foo/bar"
        );
        assert_eq!(
            ValueSnapshotServiceImpl::normalize_path("/foo\\bar"),
            "/foo/bar"
        );
        assert_eq!(
            ValueSnapshotServiceImpl::normalize_path("/foo/bar"),
            "/foo/bar"
        );
    }

    #[tokio::test]
    async fn test_snapshot_empty_value() {
        let svc = ValueSnapshotServiceImpl::new();

        // Property with no value set (None)
        let resp = svc
            .snapshot_values(Request::new(SnapshotValuesRequest {
                values: vec![PropertyValue {
                    name: "empty-prop".to_string(),
                    value: None,
                    type_name: "java.lang.String".to_string(),
                }],
                implementation_fingerprint: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results[0].name, "empty-prop");
        assert!(!resp.results[0].fingerprint.is_empty(),
            "Even a null value should produce a non-empty fingerprint");
        assert!(resp.error_message.is_empty());

        // Also test empty string value — should differ from null
        let resp_empty_str = svc
            .snapshot_values(Request::new(SnapshotValuesRequest {
                values: vec![PropertyValue {
                    name: "empty-prop".to_string(),
                    value: Some(Value::StringValue(String::new())),
                    type_name: "java.lang.String".to_string(),
                }],
                implementation_fingerprint: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_ne!(
            resp.results[0].fingerprint,
            resp_empty_str.results[0].fingerprint,
            "Null and empty-string values must produce different fingerprints"
        );
    }

    #[tokio::test]
    async fn test_snapshot_same_value_twice_is_deterministic() {
        let svc = ValueSnapshotServiceImpl::new();

        let request = SnapshotValuesRequest {
            values: vec![
                PropertyValue {
                    name: "compiler".to_string(),
                    value: Some(Value::StringValue("javac".to_string())),
                    type_name: "java.lang.String".to_string(),
                },
                PropertyValue {
                    name: "debuggable".to_string(),
                    value: Some(Value::BoolValue(false)),
                    type_name: "boolean".to_string(),
                },
                PropertyValue {
                    name: "max-heap".to_string(),
                    value: Some(Value::LongValue(2048)),
                    type_name: "java.lang.Long".to_string(),
                },
            ],
            implementation_fingerprint: "stable-impl-fp".to_string(),
        };

        let resp1 = svc
            .snapshot_values(Request::new(request.clone()))
            .await
            .unwrap()
            .into_inner();
        let resp2 = svc
            .snapshot_values(Request::new(request.clone()))
            .await
            .unwrap()
            .into_inner();

        // Composite hash must be identical
        assert_eq!(resp1.composite_hash, resp2.composite_hash,
            "Composite hash must be deterministic across calls");

        // Each individual fingerprint must be identical
        assert_eq!(resp1.results.len(), resp2.results.len());
        for (r1, r2) in resp1.results.iter().zip(resp2.results.iter()) {
            assert_eq!(r1.name, r2.name);
            assert_eq!(r1.fingerprint, r2.fingerprint,
                "Fingerprint for '{}' must be deterministic", r1.name);
        }

        assert!(resp1.success);
        assert!(resp2.success);
    }

    #[tokio::test]
    async fn test_snapshot_very_long_value() {
        let svc = ValueSnapshotServiceImpl::new();

        let long_string = "x".repeat(1_000_000); // 1 MB string
        let resp = svc
            .snapshot_values(Request::new(SnapshotValuesRequest {
                values: vec![PropertyValue {
                    name: "large-input".to_string(),
                    value: Some(Value::StringValue(long_string.clone())),
                    type_name: "java.lang.String".to_string(),
                }],
                implementation_fingerprint: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.success);
        assert_eq!(resp.results.len(), 1);
        assert_eq!(resp.results[0].name, "large-input");
        // MD5 output is always 16 bytes regardless of input size
        assert_eq!(resp.results[0].fingerprint.len(), 16,
            "MD5 fingerprint must always be 16 bytes");
        assert_eq!(resp.composite_hash.len(), 16,
            "Composite MD5 hash must always be 16 bytes");
        assert!(resp.error_message.is_empty());
    }

    #[tokio::test]
    async fn test_snapshot_different_types_produce_different_hashes() {
        let svc = ValueSnapshotServiceImpl::new();

        // Build separate single-property requests for each type, all with the
        // same logical value "42" but different proto representations.
        let types_request = |value: Value, type_name: &str| SnapshotValuesRequest {
            values: vec![PropertyValue {
                name: "prop".to_string(),
                value: Some(value),
                type_name: type_name.to_string(),
            }],
            implementation_fingerprint: "same-impl".to_string(),
        };

        let string_resp = svc
            .snapshot_values(Request::new(types_request(
                Value::StringValue("42".to_string()),
                "java.lang.String",
            )))
            .await
            .unwrap()
            .into_inner();

        let bool_resp = svc
            .snapshot_values(Request::new(types_request(
                Value::BoolValue(true),
                "boolean",
            )))
            .await
            .unwrap()
            .into_inner();

        let long_resp = svc
            .snapshot_values(Request::new(types_request(
                Value::LongValue(42),
                "java.lang.Integer",
            )))
            .await
            .unwrap()
            .into_inner();

        let binary_resp = svc
            .snapshot_values(Request::new(types_request(
                Value::BinaryValue(b"42".to_vec()),
                "[B",
            )))
            .await
            .unwrap()
            .into_inner();

        // All four composite hashes must differ
        assert_ne!(string_resp.composite_hash, bool_resp.composite_hash,
            "String and bool composite hashes must differ");
        assert_ne!(string_resp.composite_hash, long_resp.composite_hash,
            "String and long composite hashes must differ");
        assert_ne!(string_resp.composite_hash, binary_resp.composite_hash,
            "String and binary composite hashes must differ");
        assert_ne!(bool_resp.composite_hash, long_resp.composite_hash,
            "Bool and long composite hashes must differ");
        assert_ne!(bool_resp.composite_hash, binary_resp.composite_hash,
            "Bool and binary composite hashes must differ");
        assert_ne!(long_resp.composite_hash, binary_resp.composite_hash,
            "Long and binary composite hashes must differ");

        // Verify all succeeded
        for resp in [&string_resp, &bool_resp, &long_resp, &binary_resp] {
            assert!(resp.success);
            assert_eq!(resp.results.len(), 1);
            assert_eq!(resp.results[0].fingerprint.len(), 16);
        }
    }

    #[test]
    fn test_file_collection_with_backslashes() {
        let prop_unix = PropertyValue {
            name: "src".to_string(),
            value: Some(Value::StringValue("/a/b;/c/d".to_string())),
            type_name: "org.gradle.api.file.FileCollection".to_string(),
        };
        let prop_mixed = PropertyValue {
            name: "src".to_string(),
            value: Some(Value::StringValue("\\a\\b;/c/d".to_string())),
            type_name: "org.gradle.api.file.FileCollection".to_string(),
        };

        let fp_unix = ValueSnapshotServiceImpl::fingerprint_value(&prop_unix);
        let fp_mixed = ValueSnapshotServiceImpl::fingerprint_value(&prop_mixed);
        assert_eq!(fp_unix, fp_mixed, "Backslash paths should normalize to forward slash");
    }
}
