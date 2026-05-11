use std::collections::{HashMap, HashSet};

use crate::proto::{DependencyDescriptor, RepositoryDescriptor, ResolvedDependency};

use super::artifact_selection::artifact_url_for_descriptor;
use super::ivyresolve::strategy::compare_versions;
use super::maven_pom;
pub use super::repository_chain::{
    normalized_repositories, unsupported_repository_reason, unsupported_repository_reason_parts,
    unsupported_repository_url_reason,
};
use super::repository_chain::{
    repositories_for_dependency, unsupported_repository_version_filter_reason,
};
use super::resolver_transport::DependencyResolverTransport;
use super::selector::{
    requires_version_metadata, resolution_strategy_label, resolve_version_range,
};
use super::selector::{selected_static_version, validate_rejected_versions};
pub use super::selector::{
    unsupported_native_version_selector_reason, unsupported_version_selector_reason,
};

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

#[derive(Debug, Clone, PartialEq, Eq)]
struct StaticVersionConstraint {
    version: String,
    rejected_versions: Vec<String>,
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
        let version = selected_static_version(dependency).ok_or_else(|| {
            format!(
                "Unsupported dependency selector {}:{}:{}: no static version selector is native-ready",
                dependency.group, dependency.name, dependency.version
            )
        })?;
        if let Some(reason) = unsupported_version_selector_reason(&version) {
            return Err(format!(
                "Unsupported dependency selector {}:{}:{}: {}",
                dependency.group, dependency.name, version, reason
            ));
        }
        validate_rejected_versions("dependency selector", dependency, &version)?;
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

#[allow(clippy::too_many_arguments)]
pub(crate) async fn resolve_dependency_node<T: DependencyResolverTransport + Sync>(
    dep: &DependencyDescriptor,
    repos: &[RepositoryDescriptor],
    transport: &T,
    visited: &mut HashSet<(String, String)>,
    depth: u32,
    inherited_exclusions: &[(String, String)],
    lenient: bool,
    max_depth: u32,
    max_parent_depth: u32,
) -> ResolvedDependency {
    let group = dep.group.clone();
    let name = dep.name.clone();
    let raw_version = dep.version.clone();
    let scope = if dep.scope.is_empty() {
        "compile".to_string()
    } else {
        dep.scope.clone()
    };

    if let Some(reason) = unsupported_repository_version_filter_reason(repos, &raw_version) {
        return unresolved_dependency(group, name, raw_version.clone(), raw_version, scope, reason);
    }

    let allowed_repos = repositories_for_dependency(repos, &group, &name, &raw_version);
    if allowed_repos.is_empty() {
        return unresolved_dependency(
            group,
            name,
            raw_version.clone(),
            raw_version,
            scope,
            "No repository admits dependency group through content filters".to_string(),
        );
    }

    if let Some(reason) = unsupported_version_selector_reason(&raw_version) {
        return unresolved_dependency(group, name, raw_version.clone(), raw_version, scope, reason);
    }

    let selected_version = if requires_version_metadata(&raw_version) {
        let (available, metadata) = transport
            .fetch_available_versions(&group, &name, &allowed_repos)
            .await;
        if !available.is_empty() {
            resolve_version_range(&raw_version, &available, metadata.as_ref())
                .unwrap_or(raw_version.clone())
        } else {
            raw_version.clone()
        }
    } else if raw_version.ends_with("-SNAPSHOT") {
        transport
            .resolve_snapshot_version(&group, &name, &raw_version, &allowed_repos)
            .await
    } else {
        raw_version.clone()
    };

    if selected_version != raw_version {
        let strategy = resolution_strategy_label(&raw_version);
        tracing::info!(
            group = %group,
            name = %name,
            resolved_version = %selected_version,
            strategy = strategy,
            "Version resolved"
        );
    }

    let coord = (group.clone(), name.clone());
    if !visited.insert(coord.clone()) {
        tracing::debug!(
            group = %group,
            name = %name,
            depth,
            "Cycle detected — skipping re-resolution"
        );
        let repo_base = allowed_repos
            .first()
            .map(|r| r.url.as_str())
            .unwrap_or("https://repo.maven.apache.org/maven2");
        let artifact_url = artifact_url_for_descriptor(
            repo_base,
            &group,
            &name,
            &selected_version,
            &dep.classifier,
            &dep.extension,
        );
        return ResolvedDependency {
            group,
            name,
            version: raw_version,
            selected_version,
            dependencies: Vec::new(),
            resolved: true,
            failure_reason: String::new(),
            artifact_url,
            artifact_size: 0,
            artifact_sha256: String::new(),
            scope,
        };
    }

    let transitive_resolution = if dep.transitive && depth < max_depth {
        match maven_pom::resolve_transitive_dependencies(
            &group,
            &name,
            &selected_version,
            &scope,
            repos,
            transport,
            visited,
            depth,
            inherited_exclusions,
            lenient,
            max_parent_depth,
        )
        .await
        {
            Ok(resolution) => resolution,
            Err(reason) => {
                visited.remove(&coord);
                return unresolved_dependency(
                    group,
                    name,
                    raw_version.clone(),
                    selected_version,
                    scope,
                    reason,
                );
            }
        }
    } else {
        super::resolved_graph::TransitiveResolution {
            dependencies: Vec::new(),
            source_repo_url: None,
        }
    };

    visited.remove(&coord);

    let module_metadata_artifact_url = match transport
        .gradle_module_metadata_artifact_url(
            &group,
            &name,
            &selected_version,
            &scope,
            &allowed_repos,
        )
        .await
    {
        Ok(url) => url,
        Err(reason) => {
            return unresolved_dependency(
                group,
                name,
                raw_version.clone(),
                selected_version,
                scope,
                reason,
            );
        }
    };

    let repo_base = transitive_resolution
        .source_repo_url
        .as_deref()
        .or_else(|| allowed_repos.first().map(|r| r.url.as_str()))
        .unwrap_or("https://repo.maven.apache.org/maven2");
    let artifact_url = module_metadata_artifact_url.unwrap_or_else(|| {
        artifact_url_for_descriptor(
            repo_base,
            &group,
            &name,
            &selected_version,
            &dep.classifier,
            &dep.extension,
        )
    });

    ResolvedDependency {
        group,
        name,
        version: raw_version,
        selected_version,
        dependencies: transitive_resolution.dependencies,
        resolved: true,
        failure_reason: String::new(),
        artifact_url,
        artifact_size: 0,
        artifact_sha256: String::new(),
        scope,
    }
}

fn unresolved_dependency(
    group: String,
    name: String,
    version: String,
    selected_version: String,
    scope: String,
    failure_reason: String,
) -> ResolvedDependency {
    ResolvedDependency {
        group,
        name,
        version,
        selected_version,
        dependencies: Vec::new(),
        resolved: false,
        failure_reason,
        artifact_url: String::new(),
        artifact_size: 0,
        artifact_sha256: String::new(),
        scope,
    }
}

fn constraint_versions(
    constraints: &[DependencyDescriptor],
) -> Result<HashMap<(String, String), StaticVersionConstraint>, String> {
    let mut versions =
        HashMap::<(String, String), StaticVersionConstraint>::with_capacity(constraints.len());
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
        let version = selected_static_version(constraint).ok_or_else(|| {
            format!(
                "Unsupported dependency constraint {}:{}:{}: no static version selector is native-ready",
                constraint.group, constraint.name, constraint.version
            )
        })?;
        if let Some(reason) = unsupported_version_selector_reason(&version) {
            return Err(format!(
                "Unsupported dependency constraint {}:{}:{}: {}",
                constraint.group, constraint.name, version, reason
            ));
        }
        validate_rejected_versions("dependency constraint", constraint, &version)?;
        let key = (constraint.group.clone(), constraint.name.clone());
        match versions.get(&key) {
            Some(existing)
                if compare_versions(&version, &existing.version) != std::cmp::Ordering::Greater => {
            }
            _ => {
                versions.insert(
                    key,
                    StaticVersionConstraint {
                        version,
                        rejected_versions: constraint.rejected_versions.clone(),
                    },
                );
            }
        }
    }
    Ok(versions)
}

fn effective_dependency(
    dep: &DependencyDescriptor,
    constraints: &HashMap<(String, String), StaticVersionConstraint>,
    target_scope: &str,
) -> DependencyDescriptor {
    let constrained_version = constraints.get(&(dep.group.clone(), dep.name.clone()));
    let requested_version = selected_static_version(dep).unwrap_or_else(|| dep.version.clone());
    let version = select_version_with_constraint(&requested_version, constrained_version);
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
    constrained_version: Option<&StaticVersionConstraint>,
) -> String {
    let Some(constrained_version) = constrained_version else {
        return requested_version.to_string();
    };
    if requested_version.trim().is_empty() {
        return constrained_version.version.clone();
    }
    if unsupported_version_selector_reason(requested_version).is_some() {
        return requested_version.to_string();
    }
    if compare_versions(&constrained_version.version, requested_version)
        == std::cmp::Ordering::Greater
    {
        constrained_version.version.clone()
    } else {
        requested_version.to_string()
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
            strict_version: String::new(),
            required_version: String::new(),
            preferred_version: String::new(),
            rejected_versions: Vec::new(),
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
            include_groups: Vec::new(),
            exclude_groups: Vec::new(),
            include_group_prefixes: Vec::new(),
            exclude_group_prefixes: Vec::new(),
            include_modules: Vec::new(),
            exclude_modules: Vec::new(),
            include_module_versions: Vec::new(),
            exclude_module_versions: Vec::new(),
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
    fn graph_request_applies_direct_rich_version_constraint_fields() {
        let mut dependency = dep("org.example", "demo", "");
        dependency.preferred_version = "1.5".to_string();
        let mut constraint = dep("org.example", "demo", "");
        constraint.strict_version = "2.0".to_string();

        let request =
            build_dependency_graph_request(&[dependency], &[constraint], &[], "runtime").unwrap();

        assert_eq!(request.dependencies[0].version, "2.0");
    }

    #[test]
    fn graph_request_rejects_direct_rich_version_rejecting_selected_version() {
        let mut dependency = dep("org.example", "demo", "");
        dependency.required_version = "1.5".to_string();
        dependency.rejected_versions = vec!["1.5".to_string()];

        let error = build_dependency_graph_request(&[dependency], &[], &[], "").unwrap_err();

        assert!(error.contains("selected version is rejected"));
    }

    #[test]
    fn graph_request_rejects_direct_rich_version_dynamic_rejects() {
        let mut constraint = dep("org.example", "demo", "");
        constraint.strict_version = "2.0".to_string();
        constraint.rejected_versions = vec!["1.+".to_string()];

        let error = build_dependency_graph_request(
            &[dep("org.example", "demo", "1.0")],
            &[constraint],
            &[],
            "",
        )
        .unwrap_err();

        assert!(error.contains("rejected version 1.+"));
        assert!(error.contains("wildcard '+' selectors"));
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
        let error = build_dependency_graph_request(
            &[dep("org.example", "demo", "1.0")],
            &[changing],
            &[],
            "",
        )
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

        assert_eq!(
            request.repositories[0].url,
            "http://repo.example.test/maven"
        );
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
