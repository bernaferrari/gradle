//! Resolved dependency graph utilities.

use crate::proto::ResolvedDependency;

pub(crate) struct TransitiveResolution {
    pub(crate) dependencies: Vec<ResolvedDependency>,
    pub(crate) source_repo_url: Option<String>,
}

pub(crate) fn first_unresolved_reason(dependencies: &[ResolvedDependency]) -> Option<String> {
    for dep in dependencies {
        if !dep.resolved {
            return Some(dep.failure_reason.clone());
        }
        if let Some(reason) = first_unresolved_reason(&dep.dependencies) {
            return Some(reason);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dep(name: &str, resolved: bool, reason: &str) -> ResolvedDependency {
        ResolvedDependency {
            group: "org.example".to_string(),
            name: name.to_string(),
            version: "1.0".to_string(),
            selected_version: "1.0".to_string(),
            dependencies: Vec::new(),
            resolved,
            failure_reason: reason.to_string(),
            artifact_url: String::new(),
            artifact_size: 0,
            artifact_sha256: String::new(),
            scope: "compile".to_string(),
        }
    }

    #[test]
    fn finds_first_unresolved_reason_recursively() {
        let mut root = dep("root", true, "");
        root.dependencies = vec![dep("child", false, "missing metadata")];

        assert_eq!(
            first_unresolved_reason(&[root]).as_deref(),
            Some("missing metadata")
        );
    }

    #[test]
    fn returns_none_when_all_dependencies_resolved() {
        let mut root = dep("root", true, "");
        root.dependencies = vec![dep("child", true, "")];

        assert_eq!(first_unresolved_reason(&[root]), None);
    }
}
