use std::collections::HashMap;

use crate::proto::{DependencyDescriptor, RepositoryDescriptor};

use super::ivyresolve::strategy::compare_versions;

/// Gradle-shaped dependency graph input builder.
///
/// This module owns normalization of request-level dependency graph inputs before
/// repository traversal starts. `DependencyResolutionServiceImpl` should stay a
/// transport/API shim; selector, constraint, repository, and conflict semantics
/// belong under `dependency_solver`.
#[derive(Debug, Clone, PartialEq)]
pub struct DependencyGraphRequest {
    pub repositories: Vec<RepositoryDescriptor>,
    pub dependencies: Vec<DependencyDescriptor>,
}

pub fn build_dependency_graph_request(
    dependencies: &[DependencyDescriptor],
    constraints: &[DependencyDescriptor],
    repositories: &[RepositoryDescriptor],
    target_scope: &str,
) -> Result<DependencyGraphRequest, String> {
    let constraint_versions = constraint_versions(constraints)?;
    Ok(DependencyGraphRequest {
        repositories: normalized_repositories(repositories),
        dependencies: dependencies
            .iter()
            .map(|dep| effective_dependency(dep, &constraint_versions, target_scope))
            .collect(),
    })
}

pub fn normalized_repositories(repositories: &[RepositoryDescriptor]) -> Vec<RepositoryDescriptor> {
    if repositories.is_empty() {
        return vec![RepositoryDescriptor {
            id: "central".to_string(),
            url: "https://repo.maven.apache.org/maven2/".to_string(),
            m2compatible: true,
            allow_insecure_protocol: false,
            credentials: Default::default(),
            layout: String::new(),
            ivy_pattern: String::new(),
        }];
    }
    repositories
        .iter()
        .map(|repo| RepositoryDescriptor {
            id: repo.id.clone(),
            url: repo.url.clone(),
            m2compatible: repo.m2compatible,
            allow_insecure_protocol: repo.allow_insecure_protocol,
            credentials: repo.credentials.clone(),
            layout: repo.layout.clone(),
            ivy_pattern: repo.ivy_pattern.clone(),
        })
        .collect()
}

pub fn unsupported_version_selector_reason(version: &str) -> Option<String> {
    let trimmed = version.trim();
    if trimmed.is_empty() {
        return Some("Empty Maven version selector is not supported".to_string());
    }
    if trimmed.contains('+') {
        return Some(format!(
            "Unsupported Maven/Ivy version selector '{trimmed}': wildcard '+' selectors are not native-ready"
        ));
    }
    if (trimmed.starts_with('[') || trimmed.starts_with('('))
        && !(trimmed.ends_with(']') || trimmed.ends_with(')'))
    {
        return Some(format!(
            "Unsupported Maven version range '{trimmed}': range must end with ']' or ')'"
        ));
    }
    None
}

fn constraint_versions(
    constraints: &[DependencyDescriptor],
) -> Result<HashMap<(String, String), String>, String> {
    let mut versions = HashMap::<(String, String), String>::with_capacity(constraints.len());
    for constraint in constraints {
        if constraint.group.trim().is_empty() || constraint.name.trim().is_empty() {
            return Err("Dependency constraint contains empty group or name".to_string());
        }
        if let Some(reason) = unsupported_version_selector_reason(&constraint.version) {
            return Err(format!(
                "Unsupported dependency constraint {}:{}:{}: {}",
                constraint.group, constraint.name, constraint.version, reason
            ));
        }
        let key = (constraint.group.clone(), constraint.name.clone());
        match versions.get(&key) {
            Some(existing)
                if compare_versions(&constraint.version, existing) != std::cmp::Ordering::Greater => {}
            _ => {
                versions.insert(key, constraint.version.clone());
            }
        }
    }
    Ok(versions)
}

fn effective_dependency(
    dep: &DependencyDescriptor,
    constraints: &HashMap<(String, String), String>,
    target_scope: &str,
) -> DependencyDescriptor {
    let constrained_version = constraints.get(&(dep.group.clone(), dep.name.clone()));
    let version = select_version_with_constraint(&dep.version, constrained_version)
        .unwrap_or_else(|| dep.version.clone());
    let scope = if dep.scope.is_empty() && !target_scope.is_empty() {
        target_scope.to_string()
    } else {
        dep.scope.clone()
    };
    DependencyDescriptor {
        version,
        scope,
        ..dep.clone()
    }
}

fn select_version_with_constraint(
    requested_version: &str,
    constrained_version: Option<&String>,
) -> Option<String> {
    let Some(constrained_version) = constrained_version else {
        return Some(requested_version.to_string());
    };
    if requested_version.trim().is_empty() {
        return Some(constrained_version.clone());
    }
    if unsupported_version_selector_reason(requested_version).is_some() {
        return Some(requested_version.to_string());
    }
    if compare_versions(constrained_version, requested_version) == std::cmp::Ordering::Greater {
        Some(constrained_version.clone())
    } else {
        Some(requested_version.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dep(group: &str, name: &str, version: &str) -> DependencyDescriptor {
        DependencyDescriptor {
            group: group.to_string(),
            name: name.to_string(),
            version: version.to_string(),
            classifier: String::new(),
            extension: "jar".to_string(),
            transitive: true,
            scope: String::new(),
            changing: false,
            optional: false,
            ivy_conf: String::new(),
        }
    }

    #[test]
    fn graph_request_applies_static_constraints_and_target_scope() {
        let request = build_dependency_graph_request(
            &[dep("org.example", "demo", "1.0")],
            &[dep("org.example", "demo", "2.0")],
            &[],
            "runtime",
        )
        .unwrap();

        assert_eq!(request.repositories.len(), 1);
        assert_eq!(request.repositories[0].id, "central");
        assert_eq!(request.dependencies[0].version, "2.0");
        assert_eq!(request.dependencies[0].scope, "runtime");
    }

    #[test]
    fn graph_request_rejects_unsupported_constraint_selector() {
        let error = build_dependency_graph_request(
            &[dep("org.example", "demo", "1.0")],
            &[dep("org.example", "demo", "2.+")],
            &[],
            "",
        )
        .unwrap_err();

        assert!(error.contains("Unsupported dependency constraint"));
    }
}
