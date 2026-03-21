use md5::{Digest, Md5};
use tonic::{Request, Response, Status};

use crate::proto::{
    value_snapshot_service_server::ValueSnapshotService, PropertyValue, SnapshotValuesRequest,
    SnapshotValuesResponse, ValueSnapshotResult,
};

/// Rust-native value snapshotting service.
/// Computes fingerprints for input properties, replacing Java's DefaultValueSnapshotter.
#[derive(Default)]
pub struct ValueSnapshotServiceImpl;

impl ValueSnapshotServiceImpl {
    pub fn new() -> Self {
        Self
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
                hasher.update(s.as_bytes());
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
                hasher.update(s.as_bytes());
                hasher.update(b"]");
            }
            Some(crate::proto::property_value::Value::MapValue(s)) => {
                hasher.update(b"{");
                hasher.update(s.as_bytes());
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
}
