//! Phase graph envelope types for configuration cache.
//!
//! The Rust side stores and validates phase graphs serialized by the JVM.
//! It does NOT interpret the graph semantics — it treats the inner bytes as
//! opaque data and only validates the surrounding invalidation metadata.

use serde::{Deserialize, Serialize};

/// Schema version for the phase graph envelope format.
pub const PHASE_GRAPH_SCHEMA_VERSION: u32 = 1;

/// Invalidation triggers that cause the configuration cache to be discarded.
/// Gradle checks these inputs to decide if a cached configuration is still valid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct InvalidationTriggers {
    /// Content hash of the root build script (build.gradle / build.gradle.kts).
    #[serde(default)]
    pub build_script_hash: String,
    /// Content hash of settings.gradle / settings.gradle.kts.
    #[serde(default)]
    pub settings_script_hash: String,
    /// Content hashes of init scripts, keyed by their path.
    #[serde(default)]
    pub init_script_hashes: Vec<(String, String)>,
    /// Gradle version string (e.g., "8.5").
    #[serde(default)]
    pub gradle_version: String,
    /// System properties that affect configuration (e.g., "-Dprofile=dev").
    #[serde(default)]
    pub relevant_system_properties: Vec<(String, String)>,
}

impl InvalidationTriggers {
    /// Compare this set of triggers against the current state and return
    /// the names of triggers that have changed.
    pub fn changed_triggers(&self, current: &InvalidationTriggers) -> Vec<String> {
        let mut changed = Vec::new();

        if self.build_script_hash != current.build_script_hash {
            changed.push("build_script_hash".to_string());
        }
        if self.settings_script_hash != current.settings_script_hash {
            changed.push("settings_script_hash".to_string());
        }
        if self.gradle_version != current.gradle_version {
            changed.push("gradle_version".to_string());
        }

        // Init scripts: check for added/removed/changed
        let mut init_map: std::collections::HashMap<&str, &str> =
            std::collections::HashMap::with_capacity(self.init_script_hashes.len());
        for (p, h) in &self.init_script_hashes {
            init_map.insert(p.as_str(), h.as_str());
        }
        let mut cur_init_map: std::collections::HashMap<&str, &str> =
            std::collections::HashMap::with_capacity(current.init_script_hashes.len());
        for (p, h) in &current.init_script_hashes {
            cur_init_map.insert(p.as_str(), h.as_str());
        }

        for (path, hash) in &init_map {
            match cur_init_map.get(path) {
                None => changed.push(format!("init_script_removed:{}", path)),
                Some(cur_hash) if hash != cur_hash => {
                    changed.push(format!("init_script_changed:{}", path));
                }
                _ => {}
            }
        }
        for path in cur_init_map.keys() {
            if !init_map.contains_key(path) {
                changed.push(format!("init_script_added:{}", path));
            }
        }

        // System properties: check for added/removed/changed
        let mut prop_map: std::collections::HashMap<&str, &str> =
            std::collections::HashMap::with_capacity(self.relevant_system_properties.len());
        for (k, v) in &self.relevant_system_properties {
            prop_map.insert(k.as_str(), v.as_str());
        }
        let mut cur_prop_map: std::collections::HashMap<&str, &str> =
            std::collections::HashMap::with_capacity(current.relevant_system_properties.len());
        for (k, v) in &current.relevant_system_properties {
            cur_prop_map.insert(k.as_str(), v.as_str());
        }

        for (key, val) in &prop_map {
            match cur_prop_map.get(key) {
                None => changed.push(format!("property_removed:{}", key)),
                Some(cur_val) if val != cur_val => {
                    changed.push(format!("property_changed:{}", key));
                }
                _ => {}
            }
        }
        for key in cur_prop_map.keys() {
            if !prop_map.contains_key(key) {
                changed.push(format!("property_added:{}", key));
            }
        }

        changed
    }
}

/// Phase graph envelope: wraps the JVM-serialized configuration with
/// metadata for invalidation and scope isolation.
///
/// This is stored as the `serialized_config` field in `ConfigCacheEntry`.
/// The JVM uses its own serialization; Rust validates the envelope and checks
/// invalidation triggers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseGraphEnvelope {
    pub schema_version: u32,
    /// The build ID that owns this cache entry, for scope isolation.
    #[serde(default)]
    pub build_id: String,
    /// Invalidation triggers checked on validate.
    #[serde(default)]
    pub triggers: InvalidationTriggers,
    /// Number of projects in the cached configuration.
    #[serde(default)]
    pub project_count: u32,
    /// Number of task definitions in the cached configuration.
    #[serde(default)]
    pub task_count: u32,
    /// Opaque serialized phase graph from the JVM.
    #[serde(default)]
    pub phase_graph_bytes: Vec<u8>,
    /// Timestamp when the JVM produced this graph.
    #[serde(default)]
    pub creation_time_ms: i64,
}

impl Default for PhaseGraphEnvelope {
    fn default() -> Self {
        Self {
            schema_version: PHASE_GRAPH_SCHEMA_VERSION,
            build_id: String::new(),
            triggers: InvalidationTriggers::default(),
            project_count: 0,
            task_count: 0,
            phase_graph_bytes: Vec::new(),
            creation_time_ms: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_triggers_all_match() {
        let t = InvalidationTriggers {
            build_script_hash: "abc".to_string(),
            settings_script_hash: "def".to_string(),
            gradle_version: "8.5".to_string(),
            ..Default::default()
        };
        assert!(t.changed_triggers(&t).is_empty());
    }

    #[test]
    fn test_build_script_hash_change() {
        let old = InvalidationTriggers {
            build_script_hash: "abc".to_string(),
            ..Default::default()
        };
        let cur = InvalidationTriggers {
            build_script_hash: "xyz".to_string(),
            ..Default::default()
        };
        let changed = old.changed_triggers(&cur);
        assert_eq!(changed, vec!["build_script_hash"]);
    }

    #[test]
    fn test_gradle_version_change() {
        let old = InvalidationTriggers {
            gradle_version: "8.5".to_string(),
            ..Default::default()
        };
        let cur = InvalidationTriggers {
            gradle_version: "8.6".to_string(),
            ..Default::default()
        };
        let changed = old.changed_triggers(&cur);
        assert_eq!(changed, vec!["gradle_version"]);
    }

    #[test]
    fn test_init_script_removed() {
        let old = InvalidationTriggers {
            init_script_hashes: vec![("init.gradle".to_string(), "h1".to_string())],
            ..Default::default()
        };
        let cur = InvalidationTriggers {
            init_script_hashes: vec![],
            ..Default::default()
        };
        let changed = old.changed_triggers(&cur);
        assert!(changed.iter().any(|s| s.contains("init_script_removed")));
    }

    #[test]
    fn test_init_script_added() {
        let old = InvalidationTriggers {
            init_script_hashes: vec![],
            ..Default::default()
        };
        let cur = InvalidationTriggers {
            init_script_hashes: vec![("init.gradle".to_string(), "h1".to_string())],
            ..Default::default()
        };
        let changed = old.changed_triggers(&cur);
        assert!(changed.iter().any(|s| s.contains("init_script_added")));
    }

    #[test]
    fn test_init_script_hash_changed() {
        let old = InvalidationTriggers {
            init_script_hashes: vec![("init.gradle".to_string(), "h1".to_string())],
            ..Default::default()
        };
        let cur = InvalidationTriggers {
            init_script_hashes: vec![("init.gradle".to_string(), "h2".to_string())],
            ..Default::default()
        };
        let changed = old.changed_triggers(&cur);
        assert!(changed.iter().any(|s| s.contains("init_script_changed")));
    }

    #[test]
    fn test_system_property_added() {
        let old = InvalidationTriggers {
            relevant_system_properties: vec![("foo".to_string(), "bar".to_string())],
            ..Default::default()
        };
        let cur = InvalidationTriggers {
            relevant_system_properties: vec![
                ("foo".to_string(), "bar".to_string()),
                ("new".to_string(), "val".to_string()),
            ],
            ..Default::default()
        };
        let changed = old.changed_triggers(&cur);
        assert!(changed.iter().any(|s| s.contains("property_added")));
    }

    #[test]
    fn test_system_property_changed() {
        let old = InvalidationTriggers {
            relevant_system_properties: vec![("foo".to_string(), "bar".to_string())],
            ..Default::default()
        };
        let cur = InvalidationTriggers {
            relevant_system_properties: vec![("foo".to_string(), "baz".to_string())],
            ..Default::default()
        };
        let changed = old.changed_triggers(&cur);
        assert!(changed.iter().any(|s| s.contains("property_changed")));
    }

    #[test]
    fn test_envelope_default() {
        let env = PhaseGraphEnvelope::default();
        assert_eq!(env.schema_version, PHASE_GRAPH_SCHEMA_VERSION);
        assert!(env.build_id.is_empty());
        assert!(env.phase_graph_bytes.is_empty());
    }

    #[test]
    fn test_envelope_roundtrip() {
        let env = PhaseGraphEnvelope {
            build_id: "build-123".to_string(),
            project_count: 5,
            task_count: 42,
            phase_graph_bytes: vec![1, 2, 3, 4, 5],
            creation_time_ms: 1234567890,
            ..Default::default()
        };
        let bytes = bincode::serialize(&env).unwrap();
        let restored: PhaseGraphEnvelope = bincode::deserialize(&bytes).unwrap();
        assert_eq!(restored.build_id, "build-123");
        assert_eq!(restored.project_count, 5);
        assert_eq!(restored.task_count, 42);
        assert_eq!(restored.phase_graph_bytes, vec![1, 2, 3, 4, 5]);
    }
}
