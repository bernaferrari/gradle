use std::collections::BTreeMap;

pub fn jvm_usage_for_scope(scope: &str) -> &'static str {
    if matches!(scope, "runtime" | "runtimeClasspath" | "implementation") {
        "java-runtime"
    } else {
        "java-api"
    }
}

pub fn variant_attribute(
    attributes: &BTreeMap<String, serde_json::Value>,
    name: &str,
) -> Option<String> {
    attributes
        .get(name)
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

pub fn variant_usage(attributes: &BTreeMap<String, serde_json::Value>) -> Option<String> {
    variant_attribute(attributes, "org.gradle.usage")
}

pub fn has_jar_library_elements(attributes: &BTreeMap<String, serde_json::Value>) -> bool {
    variant_attribute(attributes, "org.gradle.libraryelements").as_deref() == Some("jar")
}

pub fn is_standard_jvm_environment(attributes: &BTreeMap<String, serde_json::Value>) -> bool {
    variant_attribute(attributes, "org.gradle.jvm.environment")
        .as_deref()
        .map(|value| value == "standard-jvm")
        .unwrap_or(true)
}

pub fn capability_matches_component(
    capability_group: &str,
    capability_name: &str,
    capability_version: &str,
    component_group: &str,
    component_module: &str,
    component_version: &str,
) -> bool {
    capability_group == component_group
        && capability_name == component_module
        && capability_version == component_version
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attrs(values: &[(&str, &str)]) -> BTreeMap<String, serde_json::Value> {
        values
            .iter()
            .map(|(key, value)| {
                (
                    (*key).to_string(),
                    serde_json::Value::String((*value).to_string()),
                )
            })
            .collect()
    }

    #[test]
    fn jvm_usage_tracks_gradle_runtime_scopes() {
        assert_eq!("java-runtime", jvm_usage_for_scope("runtimeClasspath"));
        assert_eq!("java-runtime", jvm_usage_for_scope("implementation"));
        assert_eq!("java-api", jvm_usage_for_scope("api"));
    }

    #[test]
    fn preferred_jvm_predicates_match_attributes() {
        let attributes = attrs(&[
            ("org.gradle.usage", "java-runtime"),
            ("org.gradle.libraryelements", "jar"),
            ("org.gradle.jvm.environment", "standard-jvm"),
        ]);

        assert_eq!(Some("java-runtime".to_string()), variant_usage(&attributes));
        assert!(has_jar_library_elements(&attributes));
        assert!(is_standard_jvm_environment(&attributes));
    }

    #[test]
    fn default_jvm_environment_is_standard() {
        assert!(is_standard_jvm_environment(&BTreeMap::new()));
    }
}
