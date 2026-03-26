use std::sync::RwLock;

/// Per-subsystem authoritative mode flags.
#[derive(Debug, Clone, Default)]
pub struct SubsystemModes {
    pub hashing: bool,
    pub cache_keys: bool,
    pub value_snapshots: bool,
    pub execution_history: bool,
    pub task_graph: bool,
    pub file_fingerprinting: bool,
    pub execution_plan: bool,
    pub config_cache: bool,
}

impl SubsystemModes {
    /// Returns the names of all subsystems.
    pub fn subsystem_names() -> &'static [&'static str] {
        &[
            "hashing",
            "cache_keys",
            "value_snapshots",
            "execution_history",
            "task_graph",
            "file_fingerprinting",
            "execution_plan",
            "config_cache",
        ]
    }

    /// Returns a vector of (name, authoritative) pairs for every subsystem.
    pub fn as_pairs(&self) -> Vec<(&'static str, bool)> {
        Self::subsystem_names()
            .iter()
            .map(|&name| (name, self.get(name)))
            .collect()
    }

    /// Get the authoritative flag for a subsystem by name.
    pub fn get(&self, subsystem: &str) -> bool {
        match subsystem {
            "hashing" => self.hashing,
            "cache_keys" => self.cache_keys,
            "value_snapshots" => self.value_snapshots,
            "execution_history" => self.execution_history,
            "task_graph" => self.task_graph,
            "file_fingerprinting" => self.file_fingerprinting,
            "execution_plan" => self.execution_plan,
            "config_cache" => self.config_cache,
            _ => false,
        }
    }

    /// Set the authoritative flag for a subsystem by name. Returns the previous value.
    pub fn set(&mut self, subsystem: &str, authoritative: bool) -> Option<bool> {
        match subsystem {
            "hashing" => Some(std::mem::replace(&mut self.hashing, authoritative)),
            "cache_keys" => Some(std::mem::replace(&mut self.cache_keys, authoritative)),
            "value_snapshots" => Some(std::mem::replace(&mut self.value_snapshots, authoritative)),
            "execution_history" => Some(std::mem::replace(
                &mut self.execution_history,
                authoritative,
            )),
            "task_graph" => Some(std::mem::replace(&mut self.task_graph, authoritative)),
            "file_fingerprinting" => Some(std::mem::replace(
                &mut self.file_fingerprinting,
                authoritative,
            )),
            "execution_plan" => Some(std::mem::replace(&mut self.execution_plan, authoritative)),
            "config_cache" => Some(std::mem::replace(&mut self.config_cache, authoritative)),
            _ => None,
        }
    }
}

/// Thread-safe authoritative mode configuration shared across services.
#[derive(Default)]
pub struct AuthoritativeConfig {
    modes: RwLock<SubsystemModes>,
}

impl AuthoritativeConfig {
    /// Create a new config with all subsystems in shadow mode.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a single subsystem's authoritative mode. Returns the previous value, or `None` if the
    /// subsystem name is not recognised.
    pub fn set_subsystem(&self, subsystem: &str, authoritative: bool) -> Option<bool> {
        let mut modes = self.modes.write().unwrap();
        modes.set(subsystem, authoritative)
    }

    /// Set all subsystems to the given authoritative mode.
    pub fn set_all(&self, authoritative: bool) {
        let mut modes = self.modes.write().unwrap();
        modes.hashing = authoritative;
        modes.cache_keys = authoritative;
        modes.value_snapshots = authoritative;
        modes.execution_history = authoritative;
        modes.task_graph = authoritative;
        modes.file_fingerprinting = authoritative;
        modes.execution_plan = authoritative;
        modes.config_cache = authoritative;
    }

    /// Check whether a specific subsystem is in authoritative mode.
    pub fn is_authoritative(&self, subsystem: &str) -> bool {
        let modes = self.modes.read().unwrap();
        modes.get(subsystem)
    }

    /// Get a snapshot of all current subsystem modes.
    pub fn get_modes(&self) -> SubsystemModes {
        let modes = self.modes.read().unwrap();
        modes.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_all_shadow() {
        let config = AuthoritativeConfig::new();
        for &name in SubsystemModes::subsystem_names() {
            assert!(!config.is_authoritative(name), "{} should be shadow", name);
        }
    }

    #[test]
    fn set_single_subsystem() {
        let config = AuthoritativeConfig::new();
        let prev = config.set_subsystem("hashing", true).unwrap();
        assert!(!prev);
        assert!(config.is_authoritative("hashing"));
        assert!(!config.is_authoritative("cache_keys"));
    }

    #[test]
    fn set_unknown_subsystem_returns_none() {
        let config = AuthoritativeConfig::new();
        assert!(config.set_subsystem("nonexistent", true).is_none());
    }

    #[test]
    fn set_all() {
        let config = AuthoritativeConfig::new();
        config.set_all(true);
        let modes = config.get_modes();
        assert!(modes.hashing);
        assert!(modes.cache_keys);
        assert!(modes.value_snapshots);
        assert!(modes.execution_history);
        assert!(modes.task_graph);
        assert!(modes.file_fingerprinting);
        assert!(modes.execution_plan);
        assert!(modes.config_cache);
    }

    #[test]
    fn set_all_then_set_subsystem_false() {
        let config = AuthoritativeConfig::new();
        config.set_all(true);
        let prev = config.set_subsystem("hashing", false).unwrap();
        assert!(prev);
        assert!(!config.is_authoritative("hashing"));
        assert!(config.is_authoritative("cache_keys"));
    }

    #[test]
    fn subsystem_modes_as_pairs() {
        let config = AuthoritativeConfig::new();
        config.set_subsystem("hashing", true);
        let modes = config.get_modes();
        let pairs = modes.as_pairs();
        assert_eq!(pairs.len(), 8);
        assert!(pairs.iter().any(|(name, auth)| *name == "hashing" && *auth));
        assert!(pairs
            .iter()
            .any(|(name, auth)| *name == "cache_keys" && !*auth));
    }
}
