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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleSelector {
    pub group: String,
    pub name: String,
    pub version: String,
}

pub fn build_dependency_graph_request(
    dependencies: &[DependencyDescriptor],
    constraints: &[DependencyDescriptor],
    repositories: &[RepositoryDescriptor],
    target_scope: &str,
) -> Result<DependencyGraphRequest, String> {
    let constraint_versions = constraint_versions(constraints)?;
    validate_dependency_selectors(dependencies)?;
    validate_repositories(repositories)?;
    Ok(DependencyGraphRequest {
        repositories: normalized_repositories(repositories),
        dependencies: dependencies
            .iter()
            .map(|dep| effective_dependency(dep, &constraint_versions, target_scope))
            .collect(),
    })
}

fn validate_dependency_selectors(dependencies: &[DependencyDescriptor]) -> Result<(), String> {
    for dependency in dependencies {
        if dependency.changing {
            return Err(format!(
                "Unsupported dependency feature {}:{}:{}: changing modules are not native-ready",
                dependency.group, dependency.name, dependency.version
            ));
        }
        if dependency.optional {
            return Err(format!(
                "Unsupported dependency feature {}:{}:{}: direct optional dependencies are not native-ready",
                dependency.group, dependency.name, dependency.version
            ));
        }
        if !dependency.ivy_conf.trim().is_empty() {
            return Err(format!(
                "Unsupported dependency feature {}:{}:{}: Ivy conf mapping '{}' is not native-ready",
                dependency.group, dependency.name, dependency.version, dependency.ivy_conf
            ));
        }
        if let Some(reason) = unsupported_version_selector_reason(&dependency.version) {
            return Err(format!(
                "Unsupported dependency selector {}:{}:{}: {}",
                dependency.group, dependency.name, dependency.version, reason
            ));
        }
    }
    Ok(())
}

fn validate_repositories(repositories: &[RepositoryDescriptor]) -> Result<(), String> {
    for repository in repositories {
        if let Some(reason) = unsupported_repository_reason(repository) {
            return Err(reason);
        }
    }
    Ok(())
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

pub fn unsupported_native_version_selector_reason(version: &str) -> Option<String> {
    if let Some(reason) = unsupported_version_selector_reason(version) {
        return Some(reason);
    }
    let trimmed = version.trim();
    if trimmed.eq_ignore_ascii_case("latest.integration")
        || trimmed.eq_ignore_ascii_case("latest.release")
    {
        return Some(format!(
            "Unsupported native dependency selector '{trimmed}': latest selectors require Gradle metadata resolution"
        ));
    }
    if trimmed.ends_with("-SNAPSHOT") {
        return Some(format!(
            "Unsupported native dependency selector '{trimmed}': changing SNAPSHOT modules are not native-ready"
        ));
    }
    if looks_like_version_range(trimmed) {
        return Some(format!(
            "Unsupported native dependency selector '{trimmed}': Maven version ranges are not native-ready"
        ));
    }
    None
}

pub fn unsupported_repository_url_reason(repository_id: &str, url: &str) -> Option<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Some(format!("Repository '{repository_id}' has no URL"));
    }
    if trimmed.starts_with("file:")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("http://")
    {
        return None;
    }
    Some(format!(
        "Repository '{repository_id}' has unsupported URL '{url}'"
    ))
}

pub fn unsupported_repository_reason(repository: &RepositoryDescriptor) -> Option<String> {
    unsupported_repository_reason_parts(
        &repository.id,
        &repository.url,
        repository.allow_insecure_protocol,
    )
}

pub fn unsupported_repository_reason_parts(
    repository_id: &str,
    url: &str,
    allow_insecure_protocol: bool,
) -> Option<String> {
    if let Some(reason) = unsupported_repository_url_reason(repository_id, url) {
        return Some(reason);
    }
    if url.trim().starts_with("http://") && !allow_insecure_protocol {
        return Some(format!(
            "Repository '{repository_id}' uses insecure HTTP without allowInsecureProtocol"
        ));
    }
    None
}

pub fn parse_module_selector_notation(notation: &str) -> Option<ModuleSelector> {
    let notation = notation.split_once('@').map_or(notation, |(base, _)| base);
    let mut parts = notation.split(':');
    let group = parts.next()?.trim();
    let name = parts.next()?.trim();
    let version = parts.next()?.trim();
    if group.is_empty() || name.is_empty() || version.is_empty() {
        return None;
    }
    if let Some(classifier) = parts.next() {
        if classifier.trim().is_empty() || parts.next().is_some() {
            return None;
        }
    }
    if parts.next().is_some() {
        return None;
    }
    Some(ModuleSelector {
        group: group.to_string(),
        name: name.to_string(),
        version: version.to_string(),
    })
}

fn looks_like_version_range(version: &str) -> bool {
    (version.starts_with('[') || version.starts_with('('))
        && (version.ends_with(']') || version.ends_with(')'))
        && version.contains(',')
}

fn constraint_versions(
    constraints: &[DependencyDescriptor],
) -> Result<HashMap<(String, String), String>, String> {
    let mut versions = HashMap::<(String, String), String>::with_capacity(constraints.len());
    for constraint in constraints {
        if constraint.group.trim().is_empty() || constraint.name.trim().is_empty() {
            return Err("Dependency constraint contains empty group or name".to_string());
        }
        if constraint.changing {
            return Err(format!(
                "Unsupported dependency constraint {}:{}:{}: changing constraints are not native-ready",
                constraint.group, constraint.name, constraint.version
            ));
        }
        if constraint.optional {
            return Err(format!(
                "Unsupported dependency constraint {}:{}:{}: optional constraints are not native-ready",
                constraint.group, constraint.name, constraint.version
            ));
        }
        if !constraint.ivy_conf.trim().is_empty() {
            return Err(format!(
                "Unsupported dependency constraint {}:{}:{}: Ivy conf mapping '{}' is not native-ready",
                constraint.group, constraint.name, constraint.version, constraint.ivy_conf
            ));
        }
        if !constraint.classifier.trim().is_empty() {
            return Err(format!(
                "Unsupported dependency constraint {}:{}:{}: classifier '{}' is not native-ready",
                constraint.group, constraint.name, constraint.version, constraint.classifier
            ));
        }
        if !constraint.extension.trim().is_empty() && constraint.extension.trim() != "jar" {
            return Err(format!(
                "Unsupported dependency constraint {}:{}:{}: extension '{}' is not native-ready",
                constraint.group, constraint.name, constraint.version, constraint.extension
            ));
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
                if compare_versions(&constraint.version, existing)
                    != std::cmp::Ordering::Greater => {}
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

    fn repo(id: &str, url: &str) -> RepositoryDescriptor {
        RepositoryDescriptor {
            id: id.to_string(),
            url: url.to_string(),
            m2compatible: true,
            allow_insecure_protocol: false,
            credentials: Default::default(),
            layout: String::new(),
            ivy_pattern: String::new(),
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

    #[test]
    fn graph_request_rejects_unsupported_constraint_features() {
        let mut changing = dep("org.example", "changing", "1.0");
        changing.changing = true;
        let error =
            build_dependency_graph_request(&[dep("org.example", "demo", "1.0")], &[changing], &[], "")
                .unwrap_err();
        assert!(error.contains("changing constraints are not native-ready"));

        let mut classifier = dep("org.example", "classifier", "1.0");
        classifier.classifier = "sources".to_string();
        let error = build_dependency_graph_request(
            &[dep("org.example", "demo", "1.0")],
            &[classifier],
            &[],
            "",
        )
        .unwrap_err();
        assert!(error.contains("classifier 'sources'"));

        let mut extension = dep("org.example", "extension", "1.0");
        extension.extension = "pom".to_string();
        let error = build_dependency_graph_request(
            &[dep("org.example", "demo", "1.0")],
            &[extension],
            &[],
            "",
        )
        .unwrap_err();
        assert!(error.contains("extension 'pom'"));
    }

    #[test]
    fn graph_request_rejects_unsupported_dependency_selector_before_resolution() {
        let error =
            build_dependency_graph_request(&[dep("org.example", "demo", "1.+")], &[], &[], "")
                .unwrap_err();

        assert!(error.contains("Unsupported dependency selector"));
        assert!(error.contains("wildcard '+' selectors"));
    }

    #[test]
    fn graph_request_rejects_unsupported_dependency_features() {
        let mut changing = dep("org.example", "changing", "1.0");
        changing.changing = true;
        let error = build_dependency_graph_request(&[changing], &[], &[], "").unwrap_err();
        assert!(error.contains("changing modules are not native-ready"));

        let mut optional = dep("org.example", "optional", "1.0");
        optional.optional = true;
        let error = build_dependency_graph_request(&[optional], &[], &[], "").unwrap_err();
        assert!(error.contains("direct optional dependencies are not native-ready"));

        let mut ivy = dep("org.example", "ivy", "1.0");
        ivy.ivy_conf = "compile->default".to_string();
        let error = build_dependency_graph_request(&[ivy], &[], &[], "").unwrap_err();
        assert!(error.contains("Ivy conf mapping"));
    }

    #[test]
    fn graph_request_rejects_unsupported_repository_before_transport() {
        let error = build_dependency_graph_request(
            &[dep("org.example", "demo", "1.0")],
            &[],
            &[repo("legacy", "sftp://repo.example.test/maven")],
            "",
        )
        .unwrap_err();

        assert!(error.contains("unsupported URL"));
        assert!(error.contains("sftp://repo.example.test/maven"));
    }

    #[test]
    fn graph_request_rejects_insecure_http_without_explicit_allow() {
        let error = build_dependency_graph_request(
            &[dep("org.example", "demo", "1.0")],
            &[],
            &[repo("plain", "http://repo.example.test/maven")],
            "",
        )
        .unwrap_err();

        assert!(error.contains("insecure HTTP"));
        assert!(error.contains("allowInsecureProtocol"));
    }

    #[test]
    fn graph_request_accepts_insecure_http_with_explicit_allow() {
        let mut plain = repo("plain", "http://repo.example.test/maven");
        plain.allow_insecure_protocol = true;

        let request =
            build_dependency_graph_request(&[dep("org.example", "demo", "1.0")], &[], &[plain], "")
                .unwrap();

        assert_eq!(request.repositories[0].url, "http://repo.example.test/maven");
        assert!(request.repositories[0].allow_insecure_protocol);
    }

    #[test]
    fn native_selector_policy_rejects_gradle_dynamic_forms() {
        assert!(unsupported_native_version_selector_reason("latest.release")
            .unwrap()
            .contains("latest selectors"));
        assert!(unsupported_native_version_selector_reason("1.0-SNAPSHOT")
            .unwrap()
            .contains("SNAPSHOT"));
        assert!(unsupported_native_version_selector_reason("[1.0,2.0)")
            .unwrap()
            .contains("version ranges"));
        assert!(unsupported_native_version_selector_reason("1.2.3").is_none());
    }

    #[test]
    fn parses_gradle_module_selector_notation() {
        assert_eq!(
            parse_module_selector_notation("org.example:demo:1.2.3"),
            Some(ModuleSelector {
                group: "org.example".to_string(),
                name: "demo".to_string(),
                version: "1.2.3".to_string(),
            })
        );
        assert_eq!(
            parse_module_selector_notation("org.example:demo:1.2.3:tests@jar"),
            Some(ModuleSelector {
                group: "org.example".to_string(),
                name: "demo".to_string(),
                version: "1.2.3".to_string(),
            })
        );
        assert!(parse_module_selector_notation("org.example:demo").is_none());
        assert!(parse_module_selector_notation("org.example:demo:1.0::jar").is_none());
    }
}
