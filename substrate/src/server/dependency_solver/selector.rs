use crate::proto::DependencyDescriptor;

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

fn looks_like_version_range(version: &str) -> bool {
    (version.starts_with('[') || version.starts_with('('))
        && (version.ends_with(']') || version.ends_with(')'))
        && version.contains(',')
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
