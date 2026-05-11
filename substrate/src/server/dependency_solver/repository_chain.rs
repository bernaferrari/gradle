use crate::proto::RepositoryDescriptor;

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
            include_groups: Vec::new(),
            exclude_groups: Vec::new(),
            include_group_prefixes: Vec::new(),
            exclude_group_prefixes: Vec::new(),
            include_modules: Vec::new(),
            exclude_modules: Vec::new(),
            include_module_versions: Vec::new(),
            exclude_module_versions: Vec::new(),
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
            include_groups: repo.include_groups.clone(),
            exclude_groups: repo.exclude_groups.clone(),
            include_group_prefixes: repo.include_group_prefixes.clone(),
            exclude_group_prefixes: repo.exclude_group_prefixes.clone(),
            include_modules: repo.include_modules.clone(),
            exclude_modules: repo.exclude_modules.clone(),
            include_module_versions: repo.include_module_versions.clone(),
            exclude_module_versions: repo.exclude_module_versions.clone(),
        })
        .collect()
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

pub fn repository_allows_dependency(
    repo: &RepositoryDescriptor,
    group: &str,
    module: &str,
    version: &str,
) -> bool {
    let group = group.trim();
    let module = module.trim();
    let version = version.trim();
    let module_key = format!("{group}:{module}");
    let module_version_key = format!("{module_key}:{version}");
    (repo.include_groups.is_empty()
        && repo.include_group_prefixes.is_empty()
        && repo.include_modules.is_empty()
        && repo.include_module_versions.is_empty()
        || repo
            .include_groups
            .iter()
            .any(|candidate| candidate == group)
        || repo
            .include_group_prefixes
            .iter()
            .any(|candidate| group_matches_prefix(group, candidate))
        || repo
            .include_modules
            .iter()
            .any(|candidate| candidate == &module_key)
        || repo
            .include_module_versions
            .iter()
            .any(|candidate| candidate == &module_version_key))
        && !repo
            .exclude_groups
            .iter()
            .any(|candidate| candidate == group)
        && !repo
            .exclude_group_prefixes
            .iter()
            .any(|candidate| group_matches_prefix(group, candidate))
        && !repo
            .exclude_modules
            .iter()
            .any(|candidate| candidate == &module_key)
        && !repo
            .exclude_module_versions
            .iter()
            .any(|candidate| candidate == &module_version_key)
}

pub fn repositories_for_dependency(
    repos: &[RepositoryDescriptor],
    group: &str,
    module: &str,
    version: &str,
) -> Vec<RepositoryDescriptor> {
    repos
        .iter()
        .filter(|repo| repository_allows_dependency(repo, group, module, version))
        .cloned()
        .collect()
}

pub fn unsupported_repository_version_filter_reason(
    repos: &[RepositoryDescriptor],
    version: &str,
) -> Option<String> {
    if repositories_have_version_filters(repos)
        && version_selector_is_dynamic_for_repository_filter(version)
    {
        return Some(format!(
            "Repository version content filters require a static version selector, got '{version}'"
        ));
    }
    None
}

fn repository_has_version_filters(repo: &RepositoryDescriptor) -> bool {
    !repo.include_module_versions.is_empty() || !repo.exclude_module_versions.is_empty()
}

fn repositories_have_version_filters(repos: &[RepositoryDescriptor]) -> bool {
    repos.iter().any(repository_has_version_filters)
}

fn version_selector_is_dynamic_for_repository_filter(version: &str) -> bool {
    let trimmed = version.trim();
    trimmed.contains(',')
        || trimmed.starts_with('[')
        || trimmed.starts_with('(')
        || trimmed == "LATEST"
        || trimmed == "RELEASE"
}

fn group_matches_prefix(group: &str, prefix: &str) -> bool {
    let prefix = prefix.trim();
    group == prefix
        || group
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn repository_group_prefix_matches_dotted_children_only() {
        let mut selected = repo("selected", "https://repo.example.test/maven");
        selected.include_group_prefixes = vec!["org.example".to_string()];

        assert!(repository_allows_dependency(
            &selected,
            "org.example.child",
            "demo",
            "1.0"
        ));
        assert!(!repository_allows_dependency(
            &selected,
            "org.examples",
            "demo",
            "1.0"
        ));
    }

    #[test]
    fn repository_version_filters_reject_dynamic_selectors() {
        let mut filtered = repo("filtered", "https://repo.example.test/maven");
        filtered.include_module_versions = vec!["org.example:demo:1.0".to_string()];

        assert!(
            unsupported_repository_version_filter_reason(&[filtered], "[1.0,2.0)")
                .unwrap()
                .contains("static version selector")
        );
    }
}
