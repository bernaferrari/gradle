use std::collections::{HashMap, HashSet};

#[cfg(test)]
use crate::proto::ResolvedDependency;
use crate::proto::{DependencyDescriptor, RepositoryDescriptor};

use super::artifact_selection::maven_artifact_shape;
use super::gradle_module_metadata;
use super::metadata_source;
use super::repository_chain;
use super::resolved_graph::TransitiveResolution;
use super::resolveengine::graph::conflicts::resolve_conflicts;
use super::resolver_transport::DependencyResolverTransport;

/// Parsed dependency from a POM file.
#[derive(Clone)]
pub struct PomDependency {
    pub(crate) group: String,
    pub(crate) name: String,
    pub(crate) version: String,
    pub(crate) scope: String,
    pub(crate) optional: bool,
    pub(crate) classifier: String,
    pub(crate) type_field: String,
    pub(crate) exclusions: Vec<(String, String)>,
}

/// Parsed dependency-management defaults for a Maven dependency.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManagedDependency {
    pub(crate) version: String,
    pub(crate) scope: String,
    pub(crate) type_field: String,
    pub(crate) exclusions: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BomImport {
    pub(crate) group: String,
    pub(crate) name: String,
    pub(crate) version: String,
}

#[derive(Clone)]
pub(crate) struct PomRegularDependency {
    pom_dep: PomDependency,
    resolved_version: String,
    effective_scope: String,
    effective_exclusions: Vec<(String, String)>,
}

pub(crate) enum PomDependencyPlan {
    BomImport(BomImport),
    Regular(PomRegularDependency),
}

pub(crate) struct PomInheritanceState {
    current_pom: String,
    visited_parents: HashSet<(String, String, String)>,
    pub(crate) properties: HashMap<String, String>,
    pub(crate) managed: HashMap<(String, String), ManagedDependency>,
}

/// Parsed `<parent>` section from a POM file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ParentPom {
    pub(crate) group_id: String,
    pub(crate) artifact_id: String,
    pub(crate) version: String,
    pub(crate) relative_path: String,
}

/// Parse a POM file and extract regular dependencies using a byte-level scanner.
/// Handles false matches like `<dependencyManagement>`.
pub fn parse_pom_dependencies(pom_content: &str) -> Vec<PomDependency> {
    let mut dependencies = Vec::new();
    let bytes = pom_content.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    let dep_mgmt_open = b"<dependencyManagement";
    let dep_mgmt_close = b"</dependencyManagement>";

    while i < len {
        let pos = match find_open_tag_exact(bytes, i, b"dependency") {
            Some(p) => p,
            None => break,
        };

        let mut in_dep_mgmt = false;
        let mut scan = 0usize;
        while scan < pos {
            if let Some(dm_start) = bytes[scan..pos]
                .windows(dep_mgmt_open.len())
                .position(|w| w == dep_mgmt_open)
                .map(|p| scan + p)
            {
                let after_dm = dm_start + dep_mgmt_open.len();
                if bytes[after_dm..pos]
                    .windows(dep_mgmt_close.len())
                    .any(|w| w == dep_mgmt_close)
                {
                    scan = after_dm;
                    continue;
                }
                in_dep_mgmt = true;
                break;
            }
            break;
        }

        if in_dep_mgmt {
            i = pos + b"<dependency".len();
            continue;
        }

        let end_pos = match find_end_tag(bytes, pos, b"dependency") {
            Some(p) => p,
            None => break,
        };

        let group = extract_tag_text(bytes, pos, b"groupId").unwrap_or_default();
        let name = extract_tag_text(bytes, pos, b"artifactId").unwrap_or_default();
        let version = extract_tag_text(bytes, pos, b"version").unwrap_or_default();
        let scope = extract_tag_text(bytes, pos, b"scope").unwrap_or_default();
        let optional = extract_tag_text(bytes, pos, b"optional")
            .map(|v| v == "true")
            .unwrap_or(false);
        let classifier = extract_tag_text(bytes, pos, b"classifier").unwrap_or_default();
        let type_field = extract_tag_text(bytes, pos, b"type").unwrap_or_default();
        let exclusions = parse_pom_exclusions(bytes, pos, end_pos);

        if !group.is_empty() && !name.is_empty() {
            dependencies.push(PomDependency {
                group,
                name,
                version,
                scope,
                optional,
                classifier,
                type_field,
                exclusions,
            });
        }

        i = end_pos + b"</dependency>".len();
    }

    dependencies
}

/// Parse <dependencyManagement><dependencies> section from a POM.
/// Returns a map of (groupId, artifactId) -> managed defaults for dependencies.
pub fn parse_dependency_management(
    pom_content: &str,
) -> HashMap<(String, String), ManagedDependency> {
    let mut managed = HashMap::new();
    let bytes = pom_content.as_bytes();

    let dm_open = b"<dependencyManagement>";
    let dm_close = b"</dependencyManagement>";

    let dm_start = bytes
        .windows(dm_open.len())
        .position(|w| w == dm_open)
        .map(|p| p + dm_open.len());

    let dm_start = match dm_start {
        Some(s) => s,
        None => return managed,
    };

    let dm_end = bytes[dm_start..]
        .windows(dm_close.len())
        .position(|w| w == dm_close)
        .map(|p| dm_start + p)
        .unwrap_or(bytes.len());

    let mut i = dm_start;
    while i < dm_end {
        let dep_pos = match find_open_tag_exact(bytes, i, b"dependency") {
            Some(p) if p < dm_end => p,
            _ => break,
        };

        let dep_end_pos = match find_end_tag(bytes, dep_pos, b"dependency") {
            Some(p) if p < dm_end => p,
            _ => {
                i = dep_pos + b"<dependency".len();
                continue;
            }
        };

        let group = extract_tag_text(bytes, dep_pos, b"groupId").unwrap_or_default();
        let name = extract_tag_text(bytes, dep_pos, b"artifactId").unwrap_or_default();
        let version = extract_tag_text(bytes, dep_pos, b"version").unwrap_or_default();
        let scope = extract_tag_text(bytes, dep_pos, b"scope").unwrap_or_default();
        let type_field = extract_tag_text(bytes, dep_pos, b"type").unwrap_or_default();
        let exclusions = parse_pom_exclusions(bytes, dep_pos, dep_end_pos);

        if !group.is_empty() && !name.is_empty() && !version.is_empty() {
            managed.insert(
                (group, name),
                ManagedDependency {
                    version,
                    scope,
                    type_field,
                    exclusions,
                },
            );
        }

        i = dep_end_pos + b"</dependency>".len();
    }

    managed
}

pub fn managed_default_scope(dep: &PomDependency, managed: Option<&ManagedDependency>) -> String {
    if !dep.scope.is_empty() {
        if matches!(
            dep.scope.as_str(),
            "compile" | "runtime" | "test" | "provided" | "system" | "import"
        ) {
            return dep.scope.clone();
        }
        return "compile".to_string();
    }

    let managed_scope = managed.map(|dep| dep.scope.as_str()).unwrap_or_default();
    if matches!(
        managed_scope,
        "compile" | "runtime" | "test" | "provided" | "system" | "import"
    ) {
        managed_scope.to_string()
    } else {
        "compile".to_string()
    }
}

pub fn effective_exclusions(
    dep: &PomDependency,
    managed: Option<&ManagedDependency>,
) -> Vec<(String, String)> {
    if !dep.exclusions.is_empty() {
        return dep.exclusions.clone();
    }

    managed
        .map(|managed| managed.exclusions.clone())
        .unwrap_or_default()
}

pub(crate) fn plan_pom_dependency(
    parent_group: &str,
    parent_name: &str,
    dep: &PomDependency,
    managed: Option<&ManagedDependency>,
    properties: &HashMap<String, String>,
    inherited_exclusions: &[(String, String)],
) -> Option<PomDependencyPlan> {
    let effective_scope = managed_default_scope(dep, managed);

    if effective_scope == "import" && dep.type_field == "pom" {
        let version = interpolate_properties(&dep.version, properties);
        if version.is_empty() {
            return None;
        }
        return Some(PomDependencyPlan::BomImport(BomImport {
            group: dep.group.clone(),
            name: dep.name.clone(),
            version,
        }));
    }

    if effective_scope == "test" || effective_scope == "provided" || dep.optional {
        return None;
    }
    if dep.group == parent_group && dep.name == parent_name {
        return None;
    }
    if is_dependency_excluded(dep, inherited_exclusions) {
        return None;
    }

    let raw_dep_version = interpolate_properties(&dep.version, properties);
    let resolved_version = if raw_dep_version.is_empty() || raw_dep_version.starts_with("${") {
        managed
            .map(|managed| managed.version.clone())
            .unwrap_or(raw_dep_version)
    } else {
        raw_dep_version
    };

    Some(PomDependencyPlan::Regular(PomRegularDependency {
        pom_dep: dep.clone(),
        resolved_version,
        effective_scope,
        effective_exclusions: effective_exclusions(dep, managed),
    }))
}

pub(crate) fn refresh_regular_dependency_from_managed(
    plan: &mut PomRegularDependency,
    managed_versions: &HashMap<(String, String), ManagedDependency>,
) {
    if !(plan.resolved_version.starts_with("${") || plan.resolved_version.is_empty()) {
        return;
    }
    if let Some(managed) =
        managed_versions.get(&(plan.pom_dep.group.clone(), plan.pom_dep.name.clone()))
    {
        plan.resolved_version = managed.version.clone();
        plan.effective_scope = managed_default_scope(&plan.pom_dep, Some(managed));
        plan.effective_exclusions = effective_exclusions(&plan.pom_dep, Some(managed));
    }
}

pub(crate) fn child_descriptor_from_regular_dependency(
    plan: &PomRegularDependency,
) -> Option<(DependencyDescriptor, Vec<(String, String)>)> {
    if plan.resolved_version.is_empty() {
        return None;
    }
    let (classifier, extension) =
        maven_artifact_shape(&plan.pom_dep.classifier, &plan.pom_dep.type_field);
    Some((
        DependencyDescriptor {
            group: plan.pom_dep.group.clone(),
            name: plan.pom_dep.name.clone(),
            version: plan.resolved_version.clone(),
            classifier,
            extension,
            transitive: true,
            scope: plan.effective_scope.clone(),
            changing: false,
            optional: false,
            ivy_conf: String::new(),
            strict_version: String::new(),
            required_version: String::new(),
            preferred_version: String::new(),
            rejected_versions: Vec::new(),
        },
        plan.effective_exclusions.clone(),
    ))
}

impl PomInheritanceState {
    pub(crate) fn new(child_pom: &str) -> Self {
        Self {
            current_pom: child_pom.to_string(),
            visited_parents: HashSet::with_capacity(8),
            properties: parse_pom_properties(child_pom),
            managed: parse_dependency_management(child_pom),
        }
    }

    pub(crate) fn next_parent(&mut self) -> Option<ParentPom> {
        let parent = parse_parent_pom(&self.current_pom)?;
        let parent_key = (
            parent.group_id.clone(),
            parent.artifact_id.clone(),
            parent.version.clone(),
        );
        if !self.visited_parents.insert(parent_key) {
            return None;
        }
        Some(parent)
    }

    pub(crate) fn merge_parent_pom(&mut self, parent_pom: String) {
        let parent_props = parse_pom_properties(&parent_pom);
        for (key, value) in parent_props {
            self.properties.entry(key).or_insert(value);
        }

        let parent_managed = parse_dependency_management(&parent_pom);
        for (key, value) in parent_managed {
            self.managed.entry(key).or_insert(value);
        }

        self.current_pom = parent_pom;
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        HashMap<String, String>,
        HashMap<(String, String), ManagedDependency>,
    ) {
        (self.properties, self.managed)
    }
}

/// Resolve parent POM inheritance while delegating network/cache access to the
/// service-owned transport boundary.
pub(crate) async fn resolve_parent_inheritance<T: DependencyResolverTransport + Sync>(
    pom_content: &str,
    repos: &[RepositoryDescriptor],
    transport: &T,
    max_parent_depth: u32,
) -> (
    HashMap<String, String>,
    HashMap<(String, String), ManagedDependency>,
) {
    let mut inheritance = PomInheritanceState::new(pom_content);

    for _ in 0..max_parent_depth {
        let parent = match inheritance.next_parent() {
            Some(parent) => parent,
            None => break,
        };

        let mut parent_content = None;
        let parent_repos = repository_chain::repositories_for_dependency(
            repos,
            &parent.group_id,
            &parent.artifact_id,
            &parent.version,
        );
        for repo in &parent_repos {
            match transport
                .fetch_pom(&parent.group_id, &parent.artifact_id, &parent.version, repo)
                .await
            {
                Ok(content) => {
                    parent_content = Some(content);
                    break;
                }
                Err(error) => {
                    tracing::debug!(
                        parent_group = %parent.group_id,
                        parent_name = %parent.artifact_id,
                        parent_version = %parent.version,
                        repo = %repo.url,
                        error = %error,
                        "Failed to fetch parent POM"
                    );
                }
            }
        }

        let Some(parent_pom) = parent_content else {
            break;
        };

        inheritance.merge_parent_pom(parent_pom);

        tracing::debug!(
            parent_group = %parent.group_id,
            parent_name = %parent.artifact_id,
            parent_version = %parent.version,
            "Inherited properties and managed deps from parent POM"
        );
    }

    inheritance.into_parts()
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn resolve_transitive_dependencies<T: DependencyResolverTransport + Sync>(
    group: &str,
    name: &str,
    version: &str,
    scope: &str,
    repos: &[RepositoryDescriptor],
    transport: &T,
    visited: &mut HashSet<(String, String)>,
    depth: u32,
    inherited_exclusions: &[(String, String)],
    lenient: bool,
    max_parent_depth: u32,
) -> Result<TransitiveResolution, String> {
    let mut redirects = HashSet::new();
    let allowed_repos = repository_chain::repositories_for_dependency(repos, group, name, version);
    for repo in allowed_repos
        .iter()
        .filter(|repo| metadata_source::supports_gradle_module_metadata(repo))
    {
        match Box::pin(
            gradle_module_metadata::resolve_transitive_from_module_metadata_in_repo(
                transport,
                group,
                name,
                version,
                scope,
                repo,
                repos,
                visited,
                depth,
                inherited_exclusions,
                lenient,
                &mut redirects,
                0,
            ),
        )
        .await
        {
            Ok(Some(resolution)) => return Ok(resolution),
            Ok(None) => {}
            Err(error) => return Err(error),
        }
    }

    for repo in &allowed_repos {
        match transport.fetch_pom(group, name, version, repo).await {
            Ok(pom_content) => {
                let (properties, managed_versions) =
                    resolve_parent_inheritance(&pom_content, repos, transport, max_parent_depth)
                        .await;
                let pom_deps = parse_pom_dependencies(&pom_content);

                let mut bom_imports = Vec::new();
                let mut regular_deps = Vec::new();

                for ((bom_group, bom_name), managed) in &managed_versions {
                    if managed.scope == "import" && managed.type_field == "pom" {
                        let bom_version = interpolate_properties(&managed.version, &properties);
                        if !bom_version.is_empty() {
                            bom_imports.push((bom_group.clone(), bom_name.clone(), bom_version));
                        }
                    }
                }

                for pom_dep in &pom_deps {
                    let managed =
                        managed_versions.get(&(pom_dep.group.clone(), pom_dep.name.clone()));
                    match plan_pom_dependency(
                        group,
                        name,
                        pom_dep,
                        managed,
                        &properties,
                        inherited_exclusions,
                    ) {
                        Some(PomDependencyPlan::BomImport(import)) => {
                            bom_imports.push((import.group, import.name, import.version));
                        }
                        Some(PomDependencyPlan::Regular(plan)) => regular_deps.push(plan),
                        None => {}
                    }
                }

                let mut merged_managed = managed_versions;
                for (bom_group, bom_name, bom_version) in &bom_imports {
                    if let Ok(bom_pom) = transport
                        .fetch_pom(bom_group, bom_name, bom_version, repo)
                        .await
                    {
                        let bom_props = parse_pom_properties(&bom_pom);
                        let bom_managed = parse_dependency_management(&bom_pom);
                        for ((g, n), mut managed) in bom_managed {
                            let interpolated = interpolate_properties(&managed.version, &bom_props);
                            if !interpolated.is_empty() {
                                managed.version = interpolated;
                                merged_managed.entry((g, n)).or_insert(managed);
                            }
                        }
                        tracing::debug!(
                            bom_group = %bom_group,
                            bom_name = %bom_name,
                            bom_version = %bom_version,
                            entries = merged_managed.len(),
                            "Loaded BOM and merged managed dependencies"
                        );
                    }
                }

                for plan in &mut regular_deps {
                    refresh_regular_dependency_from_managed(plan, &merged_managed);
                }

                let mut transitive_deps = Vec::new();
                for plan in &regular_deps {
                    let Some((child_dep, effective_exclusions)) =
                        child_descriptor_from_regular_dependency(plan)
                    else {
                        continue;
                    };
                    let resolved = transport
                        .resolve_dependency(
                            &child_dep,
                            repos,
                            visited,
                            depth + 1,
                            &effective_exclusions,
                            lenient,
                        )
                        .await;
                    transitive_deps.push(resolved);
                }

                resolve_conflicts(&mut transitive_deps);
                tracing::debug!(
                    group = %group,
                    name = %name,
                    depth,
                    transitive = transitive_deps.len(),
                    "Resolved {} transitive dependencies (depth {})",
                    transitive_deps.len(),
                    depth
                );

                return Ok(TransitiveResolution {
                    dependencies: transitive_deps,
                    source_repo_url: Some(repo.url.clone()),
                });
            }
            Err(error) => {
                tracing::debug!(
                    group = %group,
                    name = %name,
                    repo = %repo.url,
                    error = %error,
                    "Failed to fetch POM from repo"
                );
            }
        }
    }

    if lenient {
        tracing::debug!(
            group = %group,
            name = %name,
            version = %version,
            depth,
            "No module metadata found; preserving lenient artifact-only tolerance"
        );
        Ok(TransitiveResolution {
            dependencies: Vec::new(),
            source_repo_url: None,
        })
    } else {
        Err(format!(
            "No Gradle Module Metadata or Maven POM found for {group}:{name}:{version} in configured repositories"
        ))
    }
}

/// Check if a dependency matches an exclusion pattern.
///
/// An exclusion with group `*` matches any group; artifactId `*` matches any
/// artifact. Both dimensions must match for the exclusion to apply.
pub(crate) fn matches_exclusion(
    dep_group: &str,
    dep_name: &str,
    excl_group: &str,
    excl_name: &str,
) -> bool {
    let group_matches = excl_group == "*" || excl_group == dep_group;
    let name_matches = excl_name == "*" || excl_name == dep_name;
    group_matches && name_matches
}

pub(crate) fn is_dependency_excluded(dep: &PomDependency, exclusions: &[(String, String)]) -> bool {
    exclusions.iter().any(|(excl_group, excl_name)| {
        matches_exclusion(&dep.group, &dep.name, excl_group, excl_name)
    })
}

/// Parse the `<parent>` section from a POM file.
pub(crate) fn parse_parent_pom(pom_content: &str) -> Option<ParentPom> {
    let bytes = pom_content.as_bytes();
    let pos = find_open_tag_exact(bytes, 0, b"parent")?;
    let _end_pos = find_end_tag(bytes, pos, b"parent")?;

    let group_id = extract_tag_text(bytes, pos, b"groupId").unwrap_or_default();
    let artifact_id = extract_tag_text(bytes, pos, b"artifactId").unwrap_or_default();
    let version = extract_tag_text(bytes, pos, b"version").unwrap_or_default();
    let relative_path = extract_tag_text(bytes, pos, b"relativePath").unwrap_or_default();

    if group_id.is_empty() || artifact_id.is_empty() || version.is_empty() {
        return None;
    }

    Some(ParentPom {
        group_id,
        artifact_id,
        version,
        relative_path,
    })
}

/// Parse properties from the `<properties>` section of a POM.
pub fn parse_pom_properties(pom_content: &str) -> HashMap<String, String> {
    let mut props = HashMap::new();
    let bytes = pom_content.as_bytes();

    let start = match find_open_tag_exact(bytes, 0, b"properties") {
        Some(p) => p,
        None => return props,
    };
    let end = match find_end_tag(bytes, start, b"properties") {
        Some(p) => p,
        None => return props,
    };

    let mut i = start + b"<properties>".len();
    while i < end {
        let tag_start = match bytes[i..].iter().position(|&b| b == b'<') {
            Some(p) => i + p,
            None => break,
        };
        if tag_start >= end {
            break;
        }

        let tag_end = match bytes[tag_start..].iter().position(|&b| b == b'>') {
            Some(p) => tag_start + p,
            None => break,
        };

        let tag_name = &bytes[tag_start + 1..tag_end];
        if tag_name.is_empty() || tag_name[0] == b'/' {
            i = tag_end + 1;
            continue;
        }

        let close_tag = format!("</{}", std::str::from_utf8(tag_name).unwrap_or_default());
        let close_bytes = close_tag.as_bytes();
        if let Some(val_end) = bytes[tag_end + 1..end]
            .windows(close_bytes.len())
            .position(|w| w == close_bytes)
            .map(|p| tag_end + 1 + p)
        {
            let value = std::str::from_utf8(&bytes[tag_end + 1..val_end])
                .unwrap_or_default()
                .trim()
                .to_string();
            let key = std::str::from_utf8(tag_name)
                .unwrap_or_default()
                .trim()
                .to_string();
            if !key.is_empty() {
                props.insert(key, value);
            }
            i = val_end + close_bytes.len();
        } else {
            i = tag_end + 1;
        }
    }

    props
}

/// Interpolate `${property.name}` references in a string.
pub fn interpolate_properties(value: &str, properties: &HashMap<String, String>) -> String {
    let mut result = value.to_string();
    let mut max_iterations = 10;
    while result.contains("${") && max_iterations > 0 {
        max_iterations -= 1;
        if let Some(start) = result.find("${") {
            if let Some(end) = result[start..].find('}') {
                let key = &result[start + 2..start + end];
                let replacement = properties.get(key).cloned().unwrap_or_else(|| match key {
                    "project.version" | "version" | "pom.version" => "0.0.0-unknown".to_string(),
                    "project.groupId" | "groupId" => "unknown".to_string(),
                    "project.artifactId" | "artifactId" => "unknown".to_string(),
                    _ => format!("${{{key}}}"),
                });
                result.replace_range(start..start + end + 1, &replacement);
            } else {
                break;
            }
        } else {
            break;
        }
    }
    result
}

/// Parse <exclusions> block within a single <dependency> element.
fn parse_pom_exclusions(bytes: &[u8], dep_start: usize, dep_end: usize) -> Vec<(String, String)> {
    let mut exclusions = Vec::new();

    let exclusions_open = b"<exclusions>";
    let exclusions_close = b"</exclusions>";
    let exclusion_open = b"<exclusion>";
    let exclusion_close = b"</exclusion>";

    let container_start = bytes[dep_start..dep_end]
        .windows(exclusions_open.len())
        .position(|w| w == exclusions_open)
        .map(|p| dep_start + p);

    let container_start = match container_start {
        Some(s) => s,
        None => return exclusions,
    };

    let content_start = container_start + exclusions_open.len();

    let container_end = bytes[content_start..dep_end]
        .windows(exclusions_close.len())
        .position(|w| w == exclusions_close)
        .map(|p| content_start + p)
        .unwrap_or(dep_end);

    let mut i = content_start;
    while i < container_end {
        let pos = bytes[i..container_end]
            .windows(exclusion_open.len())
            .position(|w| w == exclusion_open)
            .map(|p| i + p);

        let pos = match pos {
            Some(p) => p,
            None => break,
        };

        let excl_content_start = pos + exclusion_open.len();
        let excl_end = bytes[excl_content_start..container_end]
            .windows(exclusion_close.len())
            .position(|w| w == exclusion_close)
            .map(|p| excl_content_start + p);

        let excl_end = match excl_end {
            Some(e) => e,
            None => break,
        };

        let excl_group = extract_tag_text(bytes, pos, b"groupId").unwrap_or_default();
        let excl_name = extract_tag_text(bytes, pos, b"artifactId").unwrap_or_default();

        if !excl_group.is_empty() && !excl_name.is_empty() {
            exclusions.push((excl_group, excl_name));
        }

        i = excl_end + exclusion_close.len();
    }

    exclusions
}

fn find_open_tag_exact(bytes: &[u8], from: usize, tag: &[u8]) -> Option<usize> {
    let mut open_buf = [0u8; 64];
    open_buf[0] = b'<';
    let tag_len = tag.len().min(63);
    open_buf[1..=tag_len].copy_from_slice(&tag[..tag_len]);
    let open_bytes = &open_buf[..=tag_len];
    let mut search_from = from;

    while search_from < bytes.len() {
        if let Some(pos) = bytes[search_from..]
            .windows(open_bytes.len())
            .position(|w| w == open_bytes)
            .map(|pos| search_from + pos)
        {
            let after = pos + open_bytes.len();
            if after < bytes.len() {
                let next_char = bytes[after];
                if next_char == b'>'
                    || next_char == b' '
                    || next_char == b'\n'
                    || next_char == b'\r'
                    || next_char == b'\t'
                {
                    return Some(pos);
                }
                search_from = after;
                continue;
            }
            return Some(pos);
        }
        return None;
    }
    None
}

fn find_end_tag(bytes: &[u8], from: usize, tag: &[u8]) -> Option<usize> {
    let mut close_buf = [0u8; 65];
    close_buf[0] = b'<';
    close_buf[1] = b'/';
    let tag_len = tag.len().min(63);
    close_buf[2..=tag_len + 1].copy_from_slice(&tag[..tag_len]);
    let close_bytes = &close_buf[..=tag_len + 1];
    bytes[from..]
        .windows(close_bytes.len())
        .position(|w| w == close_bytes)
        .map(|pos| from + pos)
}

fn extract_tag_text(bytes: &[u8], parent_start: usize, tag: &[u8]) -> Option<String> {
    let mut open_buf = [0u8; 64];
    open_buf[0] = b'<';
    let tag_len = tag.len().min(63);
    open_buf[1..=tag_len].copy_from_slice(&tag[..tag_len]);
    let open_bytes = &open_buf[..=tag_len];

    let mut close_buf = [0u8; 65];
    close_buf[0] = b'<';
    close_buf[1] = b'/';
    close_buf[2..=tag_len + 1].copy_from_slice(&tag[..tag_len]);
    let close_bytes = &close_buf[..=tag_len + 1];

    let search_from = parent_start;
    if let Some(start_pos) = bytes[search_from..]
        .windows(open_bytes.len())
        .position(|w| w == open_bytes)
        .map(|pos| search_from + pos)
    {
        let content_start = start_pos + open_bytes.len();
        let content_start = content_start
            + bytes[content_start..]
                .iter()
                .position(|&b| b == b'>')
                .unwrap_or(0)
            + 1;

        if let Some(end_pos) = bytes[content_start..]
            .windows(close_bytes.len())
            .position(|w| w == close_bytes)
            .map(|pos| content_start + pos)
        {
            return Some(
                String::from_utf8_lossy(&bytes[content_start..end_pos])
                    .trim()
                    .to_string(),
            );
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_parent_pom() {
        let parent = parse_parent_pom(
            r#"<project>
  <parent>
    <groupId>org.springframework.boot</groupId>
    <artifactId>spring-boot-starter-parent</artifactId>
    <version>3.2.0</version>
    <relativePath>../pom.xml</relativePath>
  </parent>
</project>"#,
        )
        .unwrap();

        assert_eq!(parent.group_id, "org.springframework.boot");
        assert_eq!(parent.artifact_id, "spring-boot-starter-parent");
        assert_eq!(parent.version, "3.2.0");
        assert_eq!(parent.relative_path, "../pom.xml");
    }

    #[test]
    fn parses_and_interpolates_pom_properties() {
        let props = parse_pom_properties(
            r#"<project>
  <properties>
    <spring.version>6.1.0</spring.version>
    <jackson.version>2.16.0</jackson.version>
  </properties>
</project>"#,
        );

        assert_eq!(
            props.get("spring.version").map(String::as_str),
            Some("6.1.0")
        );
        assert_eq!(
            interpolate_properties("org.example:${spring.version}", &props),
            "org.example:6.1.0"
        );
        assert_eq!(
            interpolate_properties("${project.version}", &props),
            "0.0.0-unknown"
        );
    }

    #[test]
    fn matches_exact_and_wildcard_exclusions() {
        assert!(matches_exclusion(
            "org.example",
            "child",
            "org.example",
            "child"
        ));
        assert!(matches_exclusion("*", "child", "*", "child"));
        assert!(matches_exclusion(
            "org.example",
            "child",
            "org.example",
            "*"
        ));
        assert!(!matches_exclusion(
            "org.example",
            "child",
            "org.other",
            "child"
        ));
    }

    #[test]
    fn plans_bom_import_and_regular_dependency() {
        let mut properties = HashMap::new();
        properties.insert("bom.version".to_string(), "1.0".to_string());
        let bom = PomDependency {
            group: "org.example".to_string(),
            name: "bom".to_string(),
            version: "${bom.version}".to_string(),
            scope: "import".to_string(),
            optional: false,
            classifier: String::new(),
            type_field: "pom".to_string(),
            exclusions: Vec::new(),
        };

        match plan_pom_dependency("org.example", "root", &bom, None, &properties, &[]).unwrap() {
            PomDependencyPlan::BomImport(import) => {
                assert_eq!(import.group, "org.example");
                assert_eq!(import.name, "bom");
                assert_eq!(import.version, "1.0");
            }
            PomDependencyPlan::Regular(_) => panic!("expected BOM import"),
        }

        let regular = PomDependency {
            group: "org.example".to_string(),
            name: "child".to_string(),
            version: "2.0".to_string(),
            scope: "runtime".to_string(),
            optional: false,
            classifier: String::new(),
            type_field: "test-jar".to_string(),
            exclusions: Vec::new(),
        };
        let plan =
            match plan_pom_dependency("org.example", "root", &regular, None, &properties, &[])
                .unwrap()
            {
                PomDependencyPlan::Regular(plan) => plan,
                PomDependencyPlan::BomImport(_) => panic!("expected regular dependency"),
            };
        let (descriptor, exclusions) = child_descriptor_from_regular_dependency(&plan).unwrap();

        assert_eq!(descriptor.name, "child");
        assert_eq!(descriptor.version, "2.0");
        assert_eq!(descriptor.classifier, "tests");
        assert_eq!(descriptor.extension, "jar");
        assert!(exclusions.is_empty());
    }

    #[test]
    fn refreshes_unresolved_regular_dependency_from_bom_managed_version() {
        let dependency = PomDependency {
            group: "org.example".to_string(),
            name: "child".to_string(),
            version: "${missing.version}".to_string(),
            scope: String::new(),
            optional: false,
            classifier: String::new(),
            type_field: String::new(),
            exclusions: Vec::new(),
        };
        let mut plan = match plan_pom_dependency(
            "org.example",
            "root",
            &dependency,
            None,
            &HashMap::new(),
            &[],
        )
        .unwrap()
        {
            PomDependencyPlan::Regular(plan) => plan,
            PomDependencyPlan::BomImport(_) => panic!("expected regular dependency"),
        };
        let mut managed = HashMap::new();
        managed.insert(
            ("org.example".to_string(), "child".to_string()),
            ManagedDependency {
                version: "3.0".to_string(),
                scope: "runtime".to_string(),
                type_field: String::new(),
                exclusions: vec![("org.bad".to_string(), "*".to_string())],
            },
        );

        refresh_regular_dependency_from_managed(&mut plan, &managed);
        let (descriptor, exclusions) = child_descriptor_from_regular_dependency(&plan).unwrap();

        assert_eq!(descriptor.version, "3.0");
        assert_eq!(descriptor.scope, "runtime");
        assert_eq!(exclusions, vec![("org.bad".to_string(), "*".to_string())]);
    }

    #[test]
    fn skips_non_runtime_pom_dependencies() {
        let optional = PomDependency {
            group: "org.example".to_string(),
            name: "optional".to_string(),
            version: "1.0".to_string(),
            scope: "runtime".to_string(),
            optional: true,
            classifier: String::new(),
            type_field: String::new(),
            exclusions: Vec::new(),
        };
        assert!(
            plan_pom_dependency("org.example", "root", &optional, None, &HashMap::new(), &[])
                .is_none()
        );

        let self_dep = PomDependency {
            group: "org.example".to_string(),
            name: "root".to_string(),
            version: "1.0".to_string(),
            scope: "runtime".to_string(),
            optional: false,
            classifier: String::new(),
            type_field: String::new(),
            exclusions: Vec::new(),
        };
        assert!(
            plan_pom_dependency("org.example", "root", &self_dep, None, &HashMap::new(), &[])
                .is_none()
        );

        let excluded = PomDependency {
            group: "org.bad".to_string(),
            name: "child".to_string(),
            version: "1.0".to_string(),
            scope: "runtime".to_string(),
            optional: false,
            classifier: String::new(),
            type_field: String::new(),
            exclusions: Vec::new(),
        };
        assert!(plan_pom_dependency(
            "org.example",
            "root",
            &excluded,
            None,
            &HashMap::new(),
            &[("org.bad".to_string(), "*".to_string())],
        )
        .is_none());
    }

    #[test]
    fn inheritance_state_merges_parent_properties_and_managed_defaults() {
        let child_pom = r#"<project>
  <parent>
    <groupId>org.example</groupId>
    <artifactId>parent</artifactId>
    <version>1.0</version>
  </parent>
  <properties>
    <child.only>true</child.only>
    <shared>child</shared>
  </properties>
</project>"#;
        let parent_pom = r#"<project>
  <properties>
    <parent.only>true</parent.only>
    <shared>parent</shared>
  </properties>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>org.slf4j</groupId>
        <artifactId>slf4j-api</artifactId>
        <version>2.0.9</version>
      </dependency>
    </dependencies>
  </dependencyManagement>
</project>"#;

        let mut state = PomInheritanceState::new(child_pom);
        let parent = state.next_parent().unwrap();
        assert_eq!(parent.group_id, "org.example");
        assert_eq!(parent.artifact_id, "parent");

        state.merge_parent_pom(parent_pom.to_string());
        let (properties, managed) = state.into_parts();

        assert_eq!(
            properties.get("child.only").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            properties.get("parent.only").map(String::as_str),
            Some("true")
        );
        assert_eq!(properties.get("shared").map(String::as_str), Some("child"));
        assert_eq!(
            managed
                .get(&("org.slf4j".to_string(), "slf4j-api".to_string()))
                .map(|dep| dep.version.as_str()),
            Some("2.0.9")
        );
    }

    #[test]
    fn inheritance_state_stops_on_parent_cycle() {
        let pom = r#"<project>
  <parent>
    <groupId>org.example</groupId>
    <artifactId>parent</artifactId>
    <version>1.0</version>
  </parent>
</project>"#;

        let mut state = PomInheritanceState::new(pom);
        assert!(state.next_parent().is_some());
        state.merge_parent_pom(pom.to_string());
        assert!(state.next_parent().is_none());
    }

    struct StaticPomTransport {
        pom: String,
    }

    #[tonic::async_trait]
    impl DependencyResolverTransport for StaticPomTransport {
        async fn fetch_pom(
            &self,
            _group: &str,
            _name: &str,
            _version: &str,
            _repo: &RepositoryDescriptor,
        ) -> Result<String, String> {
            Ok(self.pom.clone())
        }

        async fn fetch_gradle_module_metadata(
            &self,
            _group: &str,
            _name: &str,
            _version: &str,
            _repo: &RepositoryDescriptor,
        ) -> Result<Option<String>, String> {
            Ok(None)
        }

        async fn resolve_dependency(
            &self,
            dep: &DependencyDescriptor,
            _repos: &[RepositoryDescriptor],
            _visited: &mut std::collections::HashSet<(String, String)>,
            _depth: u32,
            _inherited_exclusions: &[(String, String)],
            _lenient: bool,
        ) -> ResolvedDependency {
            ResolvedDependency {
                group: dep.group.clone(),
                name: dep.name.clone(),
                version: dep.version.clone(),
                selected_version: dep.version.clone(),
                dependencies: Vec::new(),
                resolved: true,
                failure_reason: String::new(),
                artifact_url: String::new(),
                artifact_size: 0,
                artifact_sha256: String::new(),
                scope: dep.scope.clone(),
            }
        }

        async fn fetch_available_versions(
            &self,
            _group: &str,
            _name: &str,
            _repos: &[RepositoryDescriptor],
        ) -> (
            Vec<String>,
            Option<crate::server::dependency_solver::maven_metadata::MavenMetadata>,
        ) {
            (Vec::new(), None)
        }

        async fn resolve_snapshot_version(
            &self,
            _group: &str,
            _name: &str,
            raw_version: &str,
            _repos: &[RepositoryDescriptor],
        ) -> String {
            raw_version.to_string()
        }

        async fn gradle_module_metadata_artifact_url(
            &self,
            _group: &str,
            _name: &str,
            _version: &str,
            _scope: &str,
            _repos: &[RepositoryDescriptor],
        ) -> Result<Option<String>, String> {
            Ok(None)
        }
    }

    fn repo() -> RepositoryDescriptor {
        RepositoryDescriptor {
            id: "repo".to_string(),
            url: "https://repo.example.test/maven".to_string(),
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

    #[tokio::test]
    async fn async_parent_inheritance_uses_transport_boundary() {
        let child_pom = r#"<project>
  <parent>
    <groupId>org.example</groupId>
    <artifactId>parent</artifactId>
    <version>1.0</version>
  </parent>
</project>"#;
        let transport = StaticPomTransport {
            pom: r#"<project>
  <properties>
    <parent.version>1.0</parent.version>
  </properties>
</project>"#
                .to_string(),
        };

        let (properties, managed) =
            resolve_parent_inheritance(child_pom, &[repo()], &transport, 10).await;

        assert_eq!(
            properties.get("parent.version").map(String::as_str),
            Some("1.0")
        );
        assert!(managed.is_empty());
    }
}
