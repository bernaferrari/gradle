use std::collections::{HashMap, HashSet};

use crate::proto::{ResolutionStrategyConfig, ResolvedDependency};

use super::super::super::ivyresolve::strategy::compare_versions;

/// Gradle-shaped port target for `resolveengine.graph.conflicts`.
///
/// This is intentionally independent from `DependencyResolutionServiceImpl` so
/// conflict behavior can grow toward Gradle's `LatestModuleConflictResolver`
/// without deepening the gRPC service as the solver implementation.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ResolutionStrategy {
    /// Pick the highest version, matching Gradle's default newest-wins behavior.
    #[default]
    HighestVersion,
    /// Force specific versions for given "group:name" coordinates.
    Force(HashMap<String, String>),
    /// Prefer specific versions but don't force them.
    Prefer(HashMap<String, String>),
    /// Fail if any version conflict exists.
    FailOnConflict,
    /// Use the nearest definition in the dependency tree.
    NearestDefinition,
}

impl ResolutionStrategy {
    /// Parse strategy from proto config.
    pub fn from_proto(config: &ResolutionStrategyConfig) -> Self {
        match config.strategy.as_str() {
            "force" => {
                let mut map = HashMap::with_capacity(config.forced_versions.len());
                for entry in &config.forced_versions {
                    map.insert(entry.key.clone(), entry.value.clone());
                }
                ResolutionStrategy::Force(map)
            }
            "prefer" => {
                let mut map = HashMap::with_capacity(config.preferred_versions.len());
                for entry in &config.preferred_versions {
                    map.insert(entry.key.clone(), entry.value.clone());
                }
                ResolutionStrategy::Prefer(map)
            }
            "fail_on_conflict" => ResolutionStrategy::FailOnConflict,
            "nearest" => ResolutionStrategy::NearestDefinition,
            _ => ResolutionStrategy::HighestVersion,
        }
    }
}

/// Deduplicate resolved dependencies by (group, name), keeping the highest version.
pub fn resolve_conflicts(deps: &mut Vec<ResolvedDependency>) {
    resolve_conflicts_with_strategy(deps, &ResolutionStrategy::HighestVersion);
}

/// Deduplicate resolved dependencies using the given resolution strategy.
pub fn resolve_conflicts_with_strategy(
    deps: &mut Vec<ResolvedDependency>,
    strategy: &ResolutionStrategy,
) {
    if let Err(error) = try_resolve_conflicts_with_strategy(deps, strategy) {
        tracing::warn!(
            error,
            "Conflict resolution failed in legacy mutating API; falling back to highest-version resolution"
        );
        if !matches!(strategy, ResolutionStrategy::HighestVersion) {
            let _ = try_resolve_conflicts_with_strategy(deps, &ResolutionStrategy::HighestVersion);
        }
    }
}

/// Deduplicate resolved dependencies using the given resolution strategy.
///
/// This is the authoritative solver API. It fails closed for Gradle hard
/// constraints rather than silently approximating them.
pub fn try_resolve_conflicts_with_strategy(
    deps: &mut Vec<ResolvedDependency>,
    strategy: &ResolutionStrategy,
) -> Result<(), String> {
    match strategy {
        ResolutionStrategy::HighestVersion => {
            let mut best: HashMap<(String, String), usize> = HashMap::with_capacity(deps.len());

            for (idx, dep) in deps.iter().enumerate() {
                let key = (dep.group.clone(), dep.name.clone());
                if let Some(&prev_idx) = best.get(&key) {
                    if compare_versions(&dep.selected_version, &deps[prev_idx].selected_version)
                        == std::cmp::Ordering::Greater
                    {
                        best.insert(key, idx);
                    }
                } else {
                    best.insert(key, idx);
                }
            }

            retain_winners(deps, best, "highest_version", None);
            Ok(())
        }
        ResolutionStrategy::Force(forced) => {
            let mut best: HashMap<(String, String), usize> = HashMap::with_capacity(deps.len());

            for (idx, dep) in deps.iter().enumerate() {
                let key = (dep.group.clone(), dep.name.clone());
                let forced_key = module_key(dep);
                if let Some(forced_ver) = forced.get(&forced_key) {
                    if dep.selected_version == *forced_ver {
                        best.insert(key, idx);
                    }
                } else if let Some(&prev_idx) = best.get(&key) {
                    if compare_versions(&dep.selected_version, &deps[prev_idx].selected_version)
                        == std::cmp::Ordering::Greater
                    {
                        best.insert(key, idx);
                    }
                } else {
                    best.insert(key, idx);
                }
            }

            let mut missing_forced = forced
                .keys()
                .filter(|forced_key| {
                    deps.iter()
                        .any(|dep| module_key(dep).as_str() == forced_key.as_str())
                        && !best
                            .keys()
                            .any(|(group, name)| format!("{group}:{name}") == **forced_key)
                })
                .cloned()
                .collect::<Vec<_>>();
            if !missing_forced.is_empty() {
                missing_forced.sort_unstable();
                return Err(format!(
                    "Forced dependency versions were not present in resolved candidates: {}",
                    missing_forced.join(", ")
                ));
            }

            retain_winners(deps, best, "force", Some(forced.len()));
            Ok(())
        }
        ResolutionStrategy::FailOnConflict => {
            let mut versions: HashMap<(String, String), Vec<String>> =
                HashMap::with_capacity(deps.len());

            for dep in deps.iter() {
                let key = (dep.group.clone(), dep.name.clone());
                versions
                    .entry(key)
                    .or_default()
                    .push(dep.selected_version.clone());
            }

            let conflicts: Vec<((String, String), Vec<String>)> =
                versions.into_iter().filter(|(_, v)| v.len() > 1).collect();

            if !conflicts.is_empty() {
                let mut conflict_str: Vec<String> = conflicts
                    .iter()
                    .map(|((g, n), v)| {
                        let mut versions = v.clone();
                        versions.sort_unstable();
                        versions.dedup();
                        format!("{}:{} has versions {}", g, n, versions.join(", "))
                    })
                    .collect();
                conflict_str.sort_unstable();
                return Err(format!(
                    "Version conflict detected with fail_on_conflict: {}",
                    conflict_str.join("; ")
                ));
            }
            Ok(())
        }
        ResolutionStrategy::Prefer(preferred) => {
            let mut best: HashMap<(String, String), usize> = HashMap::with_capacity(deps.len());

            for (idx, dep) in deps.iter().enumerate() {
                let key = (dep.group.clone(), dep.name.clone());
                let is_preferred = preferred
                    .get(&module_key(dep))
                    .map(|v| dep.selected_version == *v)
                    .unwrap_or(false);

                if is_preferred {
                    best.insert(key, idx);
                } else if let Some(&prev_idx) = best.get(&key) {
                    let prev_preferred = preferred
                        .get(&module_key(&deps[prev_idx]))
                        .map(|v| deps[prev_idx].selected_version == *v)
                        .unwrap_or(false);
                    if !prev_preferred
                        && compare_versions(&dep.selected_version, &deps[prev_idx].selected_version)
                            == std::cmp::Ordering::Greater
                    {
                        best.insert(key, idx);
                    }
                } else {
                    best.insert(key, idx);
                }
            }

            retain_winners(deps, best, "prefer", None);
            Ok(())
        }
        ResolutionStrategy::NearestDefinition => {
            let mut seen: HashSet<(String, String)> = HashSet::with_capacity(deps.len());
            let mut kept = Vec::with_capacity(deps.len());

            for dep in deps.iter() {
                let key = (dep.group.clone(), dep.name.clone());
                if seen.insert(key) {
                    kept.push(dep.clone());
                }
            }

            let original_len = deps.len();
            *deps = kept;

            tracing::debug!(
                original_count = original_len,
                deduplicated_count = deps.len(),
                "Conflict resolution (nearest): deduplicated {} -> {}",
                original_len,
                deps.len()
            );
            Ok(())
        }
    }
}

fn module_key(dep: &ResolvedDependency) -> String {
    let mut key = String::with_capacity(dep.group.len() + dep.name.len() + 1);
    key.push_str(&dep.group);
    key.push(':');
    key.push_str(&dep.name);
    key
}

fn retain_winners(
    deps: &mut Vec<ResolvedDependency>,
    best: HashMap<(String, String), usize>,
    strategy_name: &str,
    forced_count: Option<usize>,
) {
    let mut winning_indices: Vec<usize> = best.values().copied().collect();
    winning_indices.sort_unstable();

    let original_len = deps.len();
    *deps = winning_indices
        .into_iter()
        .map(|idx| deps[idx].clone())
        .collect();

    if let Some(forced_count) = forced_count {
        tracing::debug!(
            original_count = original_len,
            deduplicated_count = deps.len(),
            forced_count = forced_count,
            "Conflict resolution ({}): deduplicated {} -> {}",
            strategy_name,
            original_len,
            deps.len()
        );
    } else {
        tracing::debug!(
            original_count = original_len,
            deduplicated_count = deps.len(),
            "Conflict resolution ({}): deduplicated {} -> {}",
            strategy_name,
            original_len,
            deps.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::proto::ResolvedDependency;

    use super::{
        resolve_conflicts, resolve_conflicts_with_strategy, try_resolve_conflicts_with_strategy,
        ResolutionStrategy,
    };

    fn dep(group: &str, name: &str, version: &str) -> ResolvedDependency {
        ResolvedDependency {
            group: group.to_string(),
            name: name.to_string(),
            version: version.to_string(),
            selected_version: version.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn latest_conflict_resolution_uses_gradle_version_comparator() {
        let mut deps = vec![
            dep("org.example", "lib", "1.0.0-beta"),
            dep("org.example", "lib", "1.0.0"),
            dep("org.example", "other", "1.0.0"),
        ];

        resolve_conflicts(&mut deps);

        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].selected_version, "1.0.0");
        assert_eq!(deps[1].name, "other");
    }

    #[test]
    fn forced_version_wins_when_present() {
        let mut forced = HashMap::new();
        forced.insert("org.example:lib".to_string(), "1.0.0".to_string());
        let mut deps = vec![
            dep("org.example", "lib", "2.0.0"),
            dep("org.example", "lib", "1.0.0"),
        ];

        resolve_conflicts_with_strategy(&mut deps, &ResolutionStrategy::Force(forced));

        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].selected_version, "1.0.0");
    }

    #[test]
    fn forced_version_missing_fails_closed_in_try_api() {
        let mut forced = HashMap::new();
        forced.insert("org.example:lib".to_string(), "3.0.0".to_string());
        let mut deps = vec![
            dep("org.example", "lib", "2.0.0"),
            dep("org.example", "lib", "1.0.0"),
        ];

        let error =
            try_resolve_conflicts_with_strategy(&mut deps, &ResolutionStrategy::Force(forced))
                .unwrap_err();

        assert!(error.contains("Forced dependency versions were not present"));
        assert!(error.contains("org.example:lib"));
        assert_eq!(deps.len(), 2);
    }

    #[test]
    fn fail_on_conflict_fails_closed_in_try_api() {
        let mut deps = vec![
            dep("org.example", "lib", "2.0.0"),
            dep("org.example", "lib", "1.0.0"),
        ];

        let error =
            try_resolve_conflicts_with_strategy(&mut deps, &ResolutionStrategy::FailOnConflict)
                .unwrap_err();

        assert!(error.contains("Version conflict detected with fail_on_conflict"));
        assert!(error.contains("org.example:lib has versions 1.0.0, 2.0.0"));
        assert_eq!(deps.len(), 2);
    }

    #[test]
    fn nearest_definition_keeps_first_seen_module() {
        let mut deps = vec![
            dep("org.example", "lib", "1.0.0"),
            dep("org.example", "other", "1.0.0"),
            dep("org.example", "lib", "2.0.0"),
        ];

        resolve_conflicts_with_strategy(&mut deps, &ResolutionStrategy::NearestDefinition);

        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].selected_version, "1.0.0");
        assert_eq!(deps[1].name, "other");
    }
}
