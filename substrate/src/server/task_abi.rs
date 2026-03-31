//! Pure-data task ABI.
//!
//! Enforces that task parameters are pure serializable data only — no live handles,
//! no rich proto messages. All inputs/outputs are `serde_json::Value` so they can
//! be safely serialized, cached, and replayed.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Marker traits
// ---------------------------------------------------------------------------

/// Marker trait for types that are pure serializable data.
/// Any type implementing this is guaranteed to contain no live handles.
pub trait TaskInput: Serialize + for<'de> Deserialize<'de> + Clone + Send + Sync + 'static {}

/// Marker trait for types that are pure serializable output data.
pub trait TaskOutput:
    Serialize + for<'de> Deserialize<'de> + Clone + Send + Sync + 'static
{
}

// Blanket implementations for common types
impl<T> TaskInput for T where
    T: Serialize + for<'de> Deserialize<'de> + Clone + Send + Sync + 'static
{
}
impl<T> TaskOutput for T where
    T: Serialize + for<'de> Deserialize<'de> + Clone + Send + Sync + 'static
{
}

// ---------------------------------------------------------------------------
// Core structs
// ---------------------------------------------------------------------------

/// A named input value for a task definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NamedInput {
    pub name: String,
    pub value: serde_json::Value,
}

/// An output file record produced by task execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutputFileRecord {
    pub path: String,
    pub size: u64,
    pub checksum: String,
}

/// Immutable definition of a task — the "contract" between the scheduler and executor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskDefinition {
    pub task_path: String,
    pub project_path: String,
    pub implementation_id: String,
    pub inputs: Vec<NamedInput>,
    pub outputs: Vec<String>,
    pub depends_on: Vec<String>,
    pub worker_isolation: String,
    pub declared_env_vars: Vec<String>,
    pub declared_system_properties: Vec<String>,
    pub declared_repository_urls: Vec<String>,
}

/// Request to execute a task with concrete input values.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskExecutionRequest {
    pub task: TaskDefinition,
    pub input_values: BTreeMap<String, serde_json::Value>,
    pub build_id: String,
}

/// Result returned after task execution completes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskExecutionResult {
    pub task_path: String,
    pub success: bool,
    pub output_files: Vec<OutputFileRecord>,
    pub duration_ms: u64,
    pub error_message: Option<String>,
}

// ---------------------------------------------------------------------------
// Cache key
// ---------------------------------------------------------------------------

/// Deterministic cache key computed from all declared inputs that affect task output.
///
/// Uses `BTreeMap` internally so that iteration order is always sorted,
/// guaranteeing the same key regardless of input ordering.
#[derive(Debug, Clone)]
pub struct CacheKey {
    pub implementation_version: String,
    pub inputs: BTreeMap<String, serde_json::Value>,
    pub outputs: BTreeMap<String, String>,
    pub env_vars: BTreeMap<String, String>,
    pub system_properties: BTreeMap<String, String>,
    pub repository_urls: BTreeMap<String, String>,
    pub implementation_id: String,
}

impl CacheKey {
    /// Build a `CacheKey` from a `TaskDefinition` and concrete input values.
    pub fn from_task(
        task: &TaskDefinition,
        input_values: &BTreeMap<String, serde_json::Value>,
    ) -> Self {
        let inputs = {
            let mut m = BTreeMap::new();
            for named in &task.inputs {
                m.insert(named.name.clone(), named.value.clone());
            }
            // Merge with explicit input_values (they take precedence).
            for (k, v) in input_values {
                m.insert(k.clone(), v.clone());
            }
            m
        };

        let outputs = task
            .outputs
            .iter()
            .map(|p| (p.clone(), String::new()))
            .collect();

        let env_vars = task
            .declared_env_vars
            .iter()
            .map(|v| (v.clone(), String::new()))
            .collect();

        let system_properties = task
            .declared_system_properties
            .iter()
            .map(|p| (p.clone(), String::new()))
            .collect();

        let repository_urls = task
            .declared_repository_urls
            .iter()
            .map(|u| (u.clone(), String::new()))
            .collect();

        Self {
            implementation_version: String::new(),
            inputs,
            outputs,
            env_vars,
            system_properties,
            repository_urls,
            implementation_id: task.implementation_id.clone(),
        }
    }

    /// Set the implementation version (e.g. "1.0.0").
    pub fn with_implementation_version(mut self, version: impl Into<String>) -> Self {
        self.implementation_version = version.into();
        self
    }
}

/// Compute a SHA-256 hex string from a `CacheKey`.
///
/// All components are serialized in a deterministic order (BTreeMap guarantees
/// sorted keys) and fed into a single SHA-256 digest.
pub fn compute_cache_key(key: &CacheKey) -> String {
    let mut hasher = Sha256::new();

    // Feed each component in a fixed order with a delimiter to prevent
    // cross-component collisions.
    fn feed_entry(hasher: &mut Sha256, k: &str, v: &str) {
        hasher.update(k.as_bytes());
        hasher.update(b"\x00");
        hasher.update(v.as_bytes());
        hasher.update(b"\x00");
    }

    feed_entry(&mut hasher, "impl_version", &key.implementation_version);
    feed_entry(&mut hasher, "impl_id", &key.implementation_id);

    for (k, v) in &key.inputs {
        let v_str = serde_json::to_string(v).unwrap_or_default();
        feed_entry(&mut hasher, &format!("input:{}", k), &v_str);
    }

    for (k, v) in &key.outputs {
        feed_entry(&mut hasher, &format!("output:{}", k), v);
    }

    for (k, v) in &key.env_vars {
        feed_entry(&mut hasher, &format!("env:{}", k), v);
    }

    for (k, v) in &key.system_properties {
        feed_entry(&mut hasher, &format!("sysprop:{}", k), v);
    }

    for (k, v) in &key.repository_urls {
        feed_entry(&mut hasher, &format!("repo:{}", k), v);
    }

    let result = hasher.finalize();
    result.iter().map(|b| format!("{:02x}", b)).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- Trait bound verification ---

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct TestInput {
        source: String,
        destination: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct TestOutput {
        files: Vec<String>,
    }

    fn assert_task_input<T: TaskInput>() {}
    fn assert_task_output<T: TaskOutput>() {}

    #[test]
    fn test_task_input_trait_bounds() {
        assert_task_input::<TestInput>();
        assert_task_input::<String>();
        assert_task_input::<BTreeMap<String, serde_json::Value>>();
        assert_task_input::<serde_json::Value>();
    }

    #[test]
    fn test_task_output_trait_bounds() {
        assert_task_output::<TestOutput>();
        assert_task_output::<String>();
        assert_task_output::<Vec<String>>();
        assert_task_output::<serde_json::Value>();
    }

    // --- Cache key stability across input order ---

    #[test]
    fn test_cache_key_stable_across_input_order() {
        fn make_task(inputs: Vec<NamedInput>) -> TaskDefinition {
            TaskDefinition {
                task_path: ":app:compile".to_string(),
                project_path: ":app".to_string(),
                implementation_id: "java-compile".to_string(),
                inputs,
                outputs: vec!["build/classes".to_string()],
                depends_on: vec![],
                worker_isolation: "process".to_string(),
                declared_env_vars: vec!["JAVA_HOME".to_string()],
                declared_system_properties: vec!["file.encoding".to_string()],
                declared_repository_urls: vec!["https://repo.maven.apache.org".to_string()],
            }
        }

        let inputs_a = vec![
            NamedInput {
                name: "source".to_string(),
                value: serde_json::json!("src/main/java"),
            },
            NamedInput {
                name: "target".to_string(),
                value: serde_json::json!("17"),
            },
        ];

        let inputs_b = vec![
            NamedInput {
                name: "target".to_string(),
                value: serde_json::json!("17"),
            },
            NamedInput {
                name: "source".to_string(),
                value: serde_json::json!("src/main/java"),
            },
        ];

        let task_a = make_task(inputs_a);
        let task_b = make_task(inputs_b);

        let input_values = BTreeMap::new();

        let key_a = CacheKey::from_task(&task_a, &input_values);
        let key_b = CacheKey::from_task(&task_b, &input_values);

        let hash_a = compute_cache_key(&key_a);
        let hash_b = compute_cache_key(&key_b);

        assert_eq!(
            hash_a, hash_b,
            "cache key must be stable regardless of input order"
        );
    }

    #[test]
    fn test_cache_key_stable_across_env_var_order() {
        let task_a = TaskDefinition {
            task_path: ":app:compile".to_string(),
            project_path: ":app".to_string(),
            implementation_id: "java-compile".to_string(),
            inputs: vec![],
            outputs: vec![],
            depends_on: vec![],
            worker_isolation: "process".to_string(),
            declared_env_vars: vec!["PATH".to_string(), "JAVA_HOME".to_string()],
            declared_system_properties: vec![],
            declared_repository_urls: vec![],
        };

        let task_b = TaskDefinition {
            task_path: ":app:compile".to_string(),
            project_path: ":app".to_string(),
            implementation_id: "java-compile".to_string(),
            inputs: vec![],
            outputs: vec![],
            depends_on: vec![],
            worker_isolation: "process".to_string(),
            declared_env_vars: vec!["JAVA_HOME".to_string(), "PATH".to_string()],
            declared_system_properties: vec![],
            declared_repository_urls: vec![],
        };

        let input_values = BTreeMap::new();

        let key_a = CacheKey::from_task(&task_a, &input_values);
        let key_b = CacheKey::from_task(&task_b, &input_values);

        assert_eq!(
            compute_cache_key(&key_a),
            compute_cache_key(&key_b),
            "env var order must not affect cache key"
        );
    }

    #[test]
    fn test_cache_key_stable_across_repo_url_order() {
        let task_a = TaskDefinition {
            task_path: ":app:compile".to_string(),
            project_path: ":app".to_string(),
            implementation_id: "java-compile".to_string(),
            inputs: vec![],
            outputs: vec![],
            depends_on: vec![],
            worker_isolation: "process".to_string(),
            declared_env_vars: vec![],
            declared_system_properties: vec![],
            declared_repository_urls: vec![
                "https://plugins.gradle.org".to_string(),
                "https://repo.maven.apache.org".to_string(),
            ],
        };

        let task_b = TaskDefinition {
            task_path: ":app:compile".to_string(),
            project_path: ":app".to_string(),
            implementation_id: "java-compile".to_string(),
            inputs: vec![],
            outputs: vec![],
            depends_on: vec![],
            worker_isolation: "process".to_string(),
            declared_env_vars: vec![],
            declared_system_properties: vec![],
            declared_repository_urls: vec![
                "https://repo.maven.apache.org".to_string(),
                "https://plugins.gradle.org".to_string(),
            ],
        };

        let input_values = BTreeMap::new();

        assert_eq!(
            compute_cache_key(&CacheKey::from_task(&task_a, &input_values)),
            compute_cache_key(&CacheKey::from_task(&task_b, &input_values)),
            "repository URL order must not affect cache key"
        );
    }

    // --- Cache key changes when any input changes ---

    #[test]
    fn test_cache_key_changes_when_input_value_changes() {
        let task = TaskDefinition {
            task_path: ":app:compile".to_string(),
            project_path: ":app".to_string(),
            implementation_id: "java-compile".to_string(),
            inputs: vec![NamedInput {
                name: "source".to_string(),
                value: serde_json::json!("src/main/java"),
            }],
            outputs: vec!["build/classes".to_string()],
            depends_on: vec![],
            worker_isolation: "process".to_string(),
            declared_env_vars: vec![],
            declared_system_properties: vec![],
            declared_repository_urls: vec![],
        };

        let mut input_values_a = BTreeMap::new();
        input_values_a.insert("target".to_string(), serde_json::json!("17"));

        let mut input_values_b = BTreeMap::new();
        input_values_b.insert("target".to_string(), serde_json::json!("21"));

        let key_a = CacheKey::from_task(&task, &input_values_a);
        let key_b = CacheKey::from_task(&task, &input_values_b);

        assert_ne!(
            compute_cache_key(&key_a),
            compute_cache_key(&key_b),
            "cache key must change when input value changes"
        );
    }

    #[test]
    fn test_cache_key_changes_when_env_var_changes() {
        let task_a = TaskDefinition {
            task_path: ":app:compile".to_string(),
            project_path: ":app".to_string(),
            implementation_id: "java-compile".to_string(),
            inputs: vec![],
            outputs: vec![],
            depends_on: vec![],
            worker_isolation: "process".to_string(),
            declared_env_vars: vec!["JAVA_HOME".to_string()],
            declared_system_properties: vec![],
            declared_repository_urls: vec![],
        };

        let task_b = TaskDefinition {
            task_path: ":app:compile".to_string(),
            project_path: ":app".to_string(),
            implementation_id: "java-compile".to_string(),
            inputs: vec![],
            outputs: vec![],
            depends_on: vec![],
            worker_isolation: "process".to_string(),
            declared_env_vars: vec!["JAVA_HOME".to_string(), "GRADLE_OPTS".to_string()],
            declared_system_properties: vec![],
            declared_repository_urls: vec![],
        };

        let input_values = BTreeMap::new();

        assert_ne!(
            compute_cache_key(&CacheKey::from_task(&task_a, &input_values)),
            compute_cache_key(&CacheKey::from_task(&task_b, &input_values)),
            "cache key must change when env vars differ"
        );
    }

    #[test]
    fn test_cache_key_changes_when_sysprop_changes() {
        let task_a = TaskDefinition {
            task_path: ":app:compile".to_string(),
            project_path: ":app".to_string(),
            implementation_id: "java-compile".to_string(),
            inputs: vec![],
            outputs: vec![],
            depends_on: vec![],
            worker_isolation: "process".to_string(),
            declared_env_vars: vec![],
            declared_system_properties: vec!["file.encoding".to_string()],
            declared_repository_urls: vec![],
        };

        let task_b = TaskDefinition {
            task_path: ":app:compile".to_string(),
            project_path: ":app".to_string(),
            implementation_id: "java-compile".to_string(),
            inputs: vec![],
            outputs: vec![],
            depends_on: vec![],
            worker_isolation: "process".to_string(),
            declared_env_vars: vec![],
            declared_system_properties: vec![
                "file.encoding".to_string(),
                "user.language".to_string(),
            ],
            declared_repository_urls: vec![],
        };

        let input_values = BTreeMap::new();

        assert_ne!(
            compute_cache_key(&CacheKey::from_task(&task_a, &input_values)),
            compute_cache_key(&CacheKey::from_task(&task_b, &input_values)),
            "cache key must change when system properties differ"
        );
    }

    #[test]
    fn test_cache_key_changes_when_repo_url_changes() {
        let task_a = TaskDefinition {
            task_path: ":app:compile".to_string(),
            project_path: ":app".to_string(),
            implementation_id: "java-compile".to_string(),
            inputs: vec![],
            outputs: vec![],
            depends_on: vec![],
            worker_isolation: "process".to_string(),
            declared_env_vars: vec![],
            declared_system_properties: vec![],
            declared_repository_urls: vec!["https://repo.maven.apache.org".to_string()],
        };

        let task_b = TaskDefinition {
            task_path: ":app:compile".to_string(),
            project_path: ":app".to_string(),
            implementation_id: "java-compile".to_string(),
            inputs: vec![],
            outputs: vec![],
            depends_on: vec![],
            worker_isolation: "process".to_string(),
            declared_env_vars: vec![],
            declared_system_properties: vec![],
            declared_repository_urls: vec!["https://repo.example.com".to_string()],
        };

        let input_values = BTreeMap::new();

        assert_ne!(
            compute_cache_key(&CacheKey::from_task(&task_a, &input_values)),
            compute_cache_key(&CacheKey::from_task(&task_b, &input_values)),
            "cache key must change when repository URLs differ"
        );
    }

    #[test]
    fn test_cache_key_changes_when_implementation_id_changes() {
        let task_a = TaskDefinition {
            task_path: ":app:compile".to_string(),
            project_path: ":app".to_string(),
            implementation_id: "java-compile".to_string(),
            inputs: vec![],
            outputs: vec![],
            depends_on: vec![],
            worker_isolation: "process".to_string(),
            declared_env_vars: vec![],
            declared_system_properties: vec![],
            declared_repository_urls: vec![],
        };

        let task_b = TaskDefinition {
            task_path: ":app:compile".to_string(),
            project_path: ":app".to_string(),
            implementation_id: "kotlin-compile".to_string(),
            inputs: vec![],
            outputs: vec![],
            depends_on: vec![],
            worker_isolation: "process".to_string(),
            declared_env_vars: vec![],
            declared_system_properties: vec![],
            declared_repository_urls: vec![],
        };

        let input_values = BTreeMap::new();

        assert_ne!(
            compute_cache_key(&CacheKey::from_task(&task_a, &input_values)),
            compute_cache_key(&CacheKey::from_task(&task_b, &input_values)),
            "cache key must change when implementation ID differs"
        );
    }

    // --- Cache key determinism ---

    #[test]
    fn test_cache_key_deterministic() {
        let task = TaskDefinition {
            task_path: ":app:compile".to_string(),
            project_path: ":app".to_string(),
            implementation_id: "java-compile".to_string(),
            inputs: vec![
                NamedInput {
                    name: "source".to_string(),
                    value: serde_json::json!("src/main/java"),
                },
                NamedInput {
                    name: "target".to_string(),
                    value: serde_json::json!("17"),
                },
            ],
            outputs: vec!["build/classes".to_string(), "build/resources".to_string()],
            depends_on: vec![":app:generate".to_string()],
            worker_isolation: "process".to_string(),
            declared_env_vars: vec!["JAVA_HOME".to_string(), "PATH".to_string()],
            declared_system_properties: vec!["file.encoding".to_string()],
            declared_repository_urls: vec!["https://repo.maven.apache.org".to_string()],
        };

        let mut input_values = BTreeMap::new();
        input_values.insert("debug".to_string(), serde_json::json!(true));

        let key = CacheKey::from_task(&task, &input_values);

        let hash1 = compute_cache_key(&key);
        let hash2 = compute_cache_key(&key);
        let hash3 = compute_cache_key(&key);

        assert_eq!(hash1, hash2);
        assert_eq!(hash2, hash3);
    }

    // --- Identical tasks produce identical keys ---

    #[test]
    fn test_identical_tasks_produce_identical_keys() {
        fn make_task() -> TaskDefinition {
            TaskDefinition {
                task_path: ":app:compile".to_string(),
                project_path: ":app".to_string(),
                implementation_id: "java-compile".to_string(),
                inputs: vec![NamedInput {
                    name: "source".to_string(),
                    value: serde_json::json!("src/main/java"),
                }],
                outputs: vec!["build/classes".to_string()],
                depends_on: vec![],
                worker_isolation: "process".to_string(),
                declared_env_vars: vec!["JAVA_HOME".to_string()],
                declared_system_properties: vec!["file.encoding".to_string()],
                declared_repository_urls: vec!["https://repo.maven.apache.org".to_string()],
            }
        }

        let task_a = make_task();
        let task_b = make_task();
        let input_values = BTreeMap::new();

        assert_eq!(
            compute_cache_key(&CacheKey::from_task(&task_a, &input_values)),
            compute_cache_key(&CacheKey::from_task(&task_b, &input_values))
        );
    }

    // --- Serialization round-trip ---

    #[test]
    fn test_task_definition_serialization_roundtrip() {
        let task = TaskDefinition {
            task_path: ":app:compile".to_string(),
            project_path: ":app".to_string(),
            implementation_id: "java-compile".to_string(),
            inputs: vec![NamedInput {
                name: "source".to_string(),
                value: serde_json::json!("src/main/java"),
            }],
            outputs: vec!["build/classes".to_string()],
            depends_on: vec![":app:generate".to_string()],
            worker_isolation: "process".to_string(),
            declared_env_vars: vec!["JAVA_HOME".to_string()],
            declared_system_properties: vec!["file.encoding".to_string()],
            declared_repository_urls: vec!["https://repo.maven.apache.org".to_string()],
        };

        let serialized = serde_json::to_string(&task).unwrap();
        let deserialized: TaskDefinition = serde_json::from_str(&serialized).unwrap();
        assert_eq!(task, deserialized);
    }

    #[test]
    fn test_execution_request_serialization_roundtrip() {
        let mut input_values = BTreeMap::new();
        input_values.insert("debug".to_string(), serde_json::json!(true));

        let req = TaskExecutionRequest {
            task: TaskDefinition {
                task_path: ":app:compile".to_string(),
                project_path: ":app".to_string(),
                implementation_id: "java-compile".to_string(),
                inputs: vec![],
                outputs: vec![],
                depends_on: vec![],
                worker_isolation: "process".to_string(),
                declared_env_vars: vec![],
                declared_system_properties: vec![],
                declared_repository_urls: vec![],
            },
            input_values,
            build_id: "build-123".to_string(),
        };

        let serialized = serde_json::to_string(&req).unwrap();
        let deserialized: TaskExecutionRequest = serde_json::from_str(&serialized).unwrap();
        assert_eq!(req, deserialized);
    }

    #[test]
    fn test_execution_result_serialization_roundtrip() {
        let result = TaskExecutionResult {
            task_path: ":app:compile".to_string(),
            success: true,
            output_files: vec![OutputFileRecord {
                path: "build/classes/Main.class".to_string(),
                size: 4096,
                checksum: "abc123".to_string(),
            }],
            duration_ms: 1234,
            error_message: None,
        };

        let serialized = serde_json::to_string(&result).unwrap();
        let deserialized: TaskExecutionResult = serde_json::from_str(&serialized).unwrap();
        assert_eq!(result, deserialized);
    }

    #[test]
    fn test_execution_result_with_error() {
        let result = TaskExecutionResult {
            task_path: ":app:compile".to_string(),
            success: false,
            output_files: vec![],
            duration_ms: 50,
            error_message: Some("Compilation failed: syntax error".to_string()),
        };

        let serialized = serde_json::to_string(&result).unwrap();
        let deserialized: TaskExecutionResult = serde_json::from_str(&serialized).unwrap();
        assert_eq!(result, deserialized);
        assert!(deserialized.error_message.is_some());
    }

    // --- Cache key format ---

    #[test]
    fn test_cache_key_is_valid_hex() {
        let task = TaskDefinition {
            task_path: ":app:compile".to_string(),
            project_path: ":app".to_string(),
            implementation_id: "java-compile".to_string(),
            inputs: vec![],
            outputs: vec![],
            depends_on: vec![],
            worker_isolation: "process".to_string(),
            declared_env_vars: vec![],
            declared_system_properties: vec![],
            declared_repository_urls: vec![],
        };

        let key = CacheKey::from_task(&task, &BTreeMap::new());
        let hex = compute_cache_key(&key);

        // SHA-256 produces 32 bytes = 64 hex chars
        assert_eq!(hex.len(), 64, "SHA-256 hex string must be 64 characters");
        assert!(
            hex.chars().all(|c| c.is_ascii_hexdigit()),
            "cache key must be valid hex"
        );
    }

    #[test]
    fn test_cache_key_with_implementation_version() {
        let task = TaskDefinition {
            task_path: ":app:compile".to_string(),
            project_path: ":app".to_string(),
            implementation_id: "java-compile".to_string(),
            inputs: vec![],
            outputs: vec![],
            depends_on: vec![],
            worker_isolation: "process".to_string(),
            declared_env_vars: vec![],
            declared_system_properties: vec![],
            declared_repository_urls: vec![],
        };

        let key_a = CacheKey::from_task(&task, &BTreeMap::new());
        let key_b = key_a.clone().with_implementation_version("1.0.0");

        assert_ne!(
            compute_cache_key(&key_a),
            compute_cache_key(&key_b),
            "cache key must change when implementation version is set"
        );
    }

    // --- Complex input values ---

    #[test]
    fn test_cache_key_with_complex_json_values() {
        let task = TaskDefinition {
            task_path: ":app:compile".to_string(),
            project_path: ":app".to_string(),
            implementation_id: "java-compile".to_string(),
            inputs: vec![
                NamedInput {
                    name: "options".to_string(),
                    value: serde_json::json!({
                        "debug": true,
                        "warnings": false,
                        "target": "17"
                    }),
                },
                NamedInput {
                    name: "sources".to_string(),
                    value: serde_json::json!(["src/main/java", "src/gen/java"]),
                },
            ],
            outputs: vec!["build/classes".to_string()],
            depends_on: vec![],
            worker_isolation: "process".to_string(),
            declared_env_vars: vec![],
            declared_system_properties: vec![],
            declared_repository_urls: vec![],
        };

        let key = CacheKey::from_task(&task, &BTreeMap::new());
        let hex = compute_cache_key(&key);

        assert_eq!(hex.len(), 64);

        // Same task should produce same key
        let key2 = CacheKey::from_task(&task, &BTreeMap::new());
        assert_eq!(hex, compute_cache_key(&key2));
    }

    // --- Output file record ---

    #[test]
    fn test_output_file_record() {
        let record = OutputFileRecord {
            path: "build/classes/Main.class".to_string(),
            size: 8192,
            checksum: "sha256:abc123".to_string(),
        };

        assert_eq!(record.path, "build/classes/Main.class");
        assert_eq!(record.size, 8192);
        assert_eq!(record.checksum, "sha256:abc123");
    }

    // --- Task execution request with input_values ---

    #[test]
    fn test_execution_request_input_values_override() {
        let mut input_values = BTreeMap::new();
        input_values.insert("source".to_string(), serde_json::json!("src/test/java"));
        input_values.insert("extra".to_string(), serde_json::json!(42));

        let req = TaskExecutionRequest {
            task: TaskDefinition {
                task_path: ":app:compile".to_string(),
                project_path: ":app".to_string(),
                implementation_id: "java-compile".to_string(),
                inputs: vec![NamedInput {
                    name: "source".to_string(),
                    value: serde_json::json!("src/main/java"),
                }],
                outputs: vec![],
                depends_on: vec![],
                worker_isolation: "process".to_string(),
                declared_env_vars: vec![],
                declared_system_properties: vec![],
                declared_repository_urls: vec![],
            },
            input_values: input_values.clone(),
            build_id: "build-456".to_string(),
        };

        let key = CacheKey::from_task(&req.task, &req.input_values);
        let hex = compute_cache_key(&key);

        assert_eq!(hex.len(), 64);
    }

    // --- Empty task produces valid key ---

    #[test]
    fn test_empty_task_produces_valid_key() {
        let task = TaskDefinition {
            task_path: String::new(),
            project_path: String::new(),
            implementation_id: String::new(),
            inputs: vec![],
            outputs: vec![],
            depends_on: vec![],
            worker_isolation: String::new(),
            declared_env_vars: vec![],
            declared_system_properties: vec![],
            declared_repository_urls: vec![],
        };

        let key = CacheKey::from_task(&task, &BTreeMap::new());
        let hex = compute_cache_key(&key);

        assert_eq!(hex.len(), 64);
    }

    // --- System property order stability ---

    #[test]
    fn test_cache_key_stable_across_sysprop_order() {
        let task_a = TaskDefinition {
            task_path: ":app:compile".to_string(),
            project_path: ":app".to_string(),
            implementation_id: "java-compile".to_string(),
            inputs: vec![],
            outputs: vec![],
            depends_on: vec![],
            worker_isolation: "process".to_string(),
            declared_env_vars: vec![],
            declared_system_properties: vec![
                "user.language".to_string(),
                "file.encoding".to_string(),
            ],
            declared_repository_urls: vec![],
        };

        let task_b = TaskDefinition {
            task_path: ":app:compile".to_string(),
            project_path: ":app".to_string(),
            implementation_id: "java-compile".to_string(),
            inputs: vec![],
            outputs: vec![],
            depends_on: vec![],
            worker_isolation: "process".to_string(),
            declared_env_vars: vec![],
            declared_system_properties: vec![
                "file.encoding".to_string(),
                "user.language".to_string(),
            ],
            declared_repository_urls: vec![],
        };

        let input_values = BTreeMap::new();

        assert_eq!(
            compute_cache_key(&CacheKey::from_task(&task_a, &input_values)),
            compute_cache_key(&CacheKey::from_task(&task_b, &input_values)),
            "system property order must not affect cache key"
        );
    }
}
