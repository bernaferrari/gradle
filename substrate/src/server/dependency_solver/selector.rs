use crate::proto::DependencyDescriptor;

use super::ivyresolve::strategy::compare_versions;
use super::maven_metadata::MavenMetadata;

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

pub(crate) fn selected_static_version(descriptor: &DependencyDescriptor) -> Option<String> {
    [
        descriptor.strict_version.as_str(),
        descriptor.required_version.as_str(),
        descriptor.version.as_str(),
        descriptor.preferred_version.as_str(),
    ]
    .into_iter()
    .map(str::trim)
    .find(|version| !version.is_empty())
    .map(ToString::to_string)
}

pub(crate) fn validate_rejected_versions(
    role: &str,
    descriptor: &DependencyDescriptor,
    selected_version: &str,
) -> Result<(), String> {
    for rejected in &descriptor.rejected_versions {
        let rejected = rejected.trim();
        if rejected.is_empty() {
            return Err(format!(
                "Unsupported {role} {}:{}:{}: empty rejected version is not native-ready",
                descriptor.group, descriptor.name, selected_version
            ));
        }
        if let Some(reason) = unsupported_version_selector_reason(rejected) {
            return Err(format!(
                "Unsupported {role} {}:{}:{}: rejected version {rejected}: {reason}",
                descriptor.group, descriptor.name, selected_version
            ));
        }
        if rejected == selected_version {
            return Err(format!(
                "Unsupported {role} {}:{}:{}: selected version is rejected",
                descriptor.group, descriptor.name, selected_version
            ));
        }
    }
    Ok(())
}

/// Resolve a supported Maven version selector to a concrete version.
///
/// Unsupported dynamic selectors such as empty strings and Ivy-style wildcard
/// selectors return `None` so callers can fail closed instead of treating them
/// as exact versions.
pub(crate) fn resolve_version_range(
    range: &str,
    available: &[String],
    metadata: Option<&MavenMetadata>,
) -> Option<String> {
    let range = range.trim();
    if range.is_empty() || range.contains('+') {
        return None;
    }

    if range == "latest.release" || range == "latest.integration" {
        if let Some(meta) = metadata {
            if let Some(release) = &meta.versioning.release {
                return Some(release.clone());
            }
        }
        return available.last().cloned();
    }
    if range == "LATEST" {
        if let Some(meta) = metadata {
            if let Some(latest) = &meta.versioning.latest {
                return Some(latest.clone());
            }
        }
        return available.last().cloned();
    }
    if range == "RELEASE" {
        if let Some(meta) = metadata {
            if let Some(release) = &meta.versioning.release {
                return Some(release.clone());
            }
        }
        return available.last().cloned();
    }

    if !range.starts_with('[') && !range.starts_with('(') {
        return Some(range.to_string());
    }
    if !(range.ends_with(']') || range.ends_with(')')) || range.len() < 2 {
        return None;
    }

    let inner: &str = &range[1..range.len() - 1];
    let parts: Vec<&str> = inner.split(',').collect();
    if parts.len() != 2 {
        return None;
    }

    let start = parts[0].trim();
    let end = parts[1].trim();
    let start_inclusive = range.starts_with('[');
    let end_inclusive = range.ends_with(']');

    use std::cmp::Ordering;
    let matching: Vec<&String> = available
        .iter()
        .filter(|v| {
            if !start.is_empty() {
                let cmp = compare_versions(v, start);
                if start_inclusive && cmp == Ordering::Less {
                    return false;
                }
                if !start_inclusive && cmp != Ordering::Greater {
                    return false;
                }
            }
            if !end.is_empty() {
                let cmp = compare_versions(v, end);
                if end_inclusive && cmp == Ordering::Greater {
                    return false;
                }
                if !end_inclusive && cmp != Ordering::Less {
                    return false;
                }
            }
            true
        })
        .collect();

    matching.last().map(|v| (*v).clone())
}

pub(crate) fn requires_version_metadata(selector: &str) -> bool {
    selector.contains(',')
        || selector.starts_with('[')
        || selector.starts_with('(')
        || selector == "LATEST"
        || selector == "RELEASE"
}

pub(crate) fn resolution_strategy_label(selector: &str) -> &'static str {
    if selector.contains(',') || selector.starts_with('[') || selector.starts_with('(') {
        "range"
    } else if selector.ends_with("-SNAPSHOT") {
        "snapshot"
    } else {
        "latest"
    }
}

fn looks_like_version_range(version: &str) -> bool {
    (version.starts_with('[') || version.starts_with('('))
        && (version.ends_with(']') || version.ends_with(')'))
        && version.contains(',')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::dependency_solver::maven_metadata::MavenVersioning;

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
    fn resolves_exact_and_range_selectors() {
        let available = vec![
            "1.0.0".to_string(),
            "1.5.0".to_string(),
            "2.0.0".to_string(),
            "2.5.0".to_string(),
        ];

        assert_eq!(
            resolve_version_range("1.0.0", &available, None),
            Some("1.0.0".to_string())
        );
        assert_eq!(
            resolve_version_range("[1.0.0,2.0.0)", &available, None),
            Some("1.5.0".to_string())
        );
        assert_eq!(
            resolve_version_range("(1.0,)", &available, None),
            Some("2.5.0".to_string())
        );
    }

    #[test]
    fn resolves_latest_and_release_from_metadata() {
        let available = vec!["1.0.0".to_string(), "2.0.0".to_string()];
        let metadata = MavenMetadata {
            group_id: String::new(),
            artifact_id: String::new(),
            versioning: MavenVersioning {
                latest: Some("2.0.0".to_string()),
                release: Some("1.5.0".to_string()),
                last_updated: None,
                snapshot: None,
                versions: available.clone(),
            },
        };

        assert_eq!(
            resolve_version_range("RELEASE", &available, Some(&metadata)),
            Some("1.5.0".to_string())
        );
        assert_eq!(
            resolve_version_range("LATEST", &available, Some(&metadata)),
            Some("2.0.0".to_string())
        );
        assert_eq!(
            resolve_version_range("latest.release", &available, Some(&metadata)),
            Some("1.5.0".to_string())
        );
    }

    #[test]
    fn rejects_unsupported_range_patterns() {
        let available = vec!["1.0.0".to_string(), "1.2.0".to_string()];

        assert_eq!(resolve_version_range("1.+", &available, None), None);
        assert_eq!(resolve_version_range("", &available, None), None);
        assert_eq!(resolve_version_range("  ", &available, None), None);
    }

    #[test]
    fn classifies_selectors_requiring_metadata() {
        assert!(requires_version_metadata("[1.0,2.0)"));
        assert!(requires_version_metadata("(1.0,)"));
        assert!(requires_version_metadata("LATEST"));
        assert!(requires_version_metadata("RELEASE"));
        assert!(!requires_version_metadata("1.2.3"));
    }

    #[test]
    fn labels_resolution_strategy_for_diagnostics() {
        assert_eq!(resolution_strategy_label("[1.0,2.0)"), "range");
        assert_eq!(resolution_strategy_label("1.0-SNAPSHOT"), "snapshot");
        assert_eq!(resolution_strategy_label("LATEST"), "latest");
    }
}
