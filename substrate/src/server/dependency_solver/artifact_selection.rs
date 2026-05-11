/// Gradle-shaped artifact selection helpers for Maven coordinates.
///
/// These functions intentionally live under `dependency_solver` rather than the
/// transport service so artifact identity, classifier/extension normalization,
/// and Maven type mapping stay close to graph-building semantics.
pub fn group_to_path(group: &str) -> String {
    let dot_count = group.bytes().filter(|&b| b == b'.').count();
    let mut path = String::with_capacity(group.len() + dot_count);
    for b in group.bytes() {
        if b == b'.' {
            path.push('/');
        } else {
            path.push(b as char);
        }
    }
    path
}

pub fn normalize_extension(extension: &str) -> String {
    let extension = extension.trim_start_matches('.');
    if extension.is_empty() {
        "jar".to_string()
    } else {
        extension.to_string()
    }
}

pub fn is_metadata_extension(extension: &str) -> bool {
    matches!(
        normalize_extension(extension).as_str(),
        "pom" | "module" | "ivy" | "maven-metadata.xml"
    )
}

pub fn artifact_cache_key(
    group: &str,
    name: &str,
    version: &str,
    classifier: &str,
    extension: &str,
) -> String {
    let extension = normalize_extension(extension);
    let mut key = String::with_capacity(
        group.len() + name.len() + version.len() + classifier.len() + extension.len() + 4,
    );
    key.push_str(group);
    key.push(':');
    key.push_str(name);
    key.push(':');
    key.push_str(version);
    key.push(':');
    key.push_str(classifier);
    key.push(':');
    key.push_str(&extension);
    key
}

pub fn artifact_url_for_descriptor(
    repo_base: &str,
    group: &str,
    name: &str,
    version: &str,
    classifier: &str,
    extension: &str,
) -> String {
    let extension = normalize_extension(extension);
    let classifier_suffix = if classifier.is_empty() {
        String::new()
    } else {
        format!("-{classifier}")
    };
    format!(
        "{}/{}/{}/{}/{}-{}{}.{}",
        repo_base.trim_end_matches('/'),
        group_to_path(group),
        name,
        version,
        name,
        version,
        classifier_suffix,
        extension
    )
}

pub fn artifact_file_parts_from_url(
    artifact_url: &str,
    name: &str,
    version: &str,
) -> Option<(String, String)> {
    let path = reqwest::Url::parse(artifact_url)
        .ok()
        .and_then(|url| url.path_segments()?.next_back().map(str::to_string))
        .or_else(|| artifact_url.rsplit('/').next().map(str::to_string))?;
    let prefix = format!("{name}-{version}");
    if !path.starts_with(&prefix) {
        return None;
    }
    let dot = path.rfind('.')?;
    let extension = path[dot + 1..].to_string();
    if extension.is_empty() {
        return None;
    }
    let suffix = &path[prefix.len()..dot];
    if !suffix.is_empty() && !suffix.starts_with('-') {
        return None;
    }
    let classifier = suffix.strip_prefix('-').unwrap_or("").to_string();
    Some((classifier, extension))
}

pub fn maven_artifact_shape(classifier: &str, type_field: &str) -> (String, String) {
    let classifier = classifier.trim();
    let type_field = type_field.trim();
    let effective_type = if type_field.is_empty() {
        "jar"
    } else {
        type_field
    };
    let extension = if maven_type_has_jar_extension(effective_type) {
        "jar"
    } else {
        effective_type
    };
    let effective_classifier = if !classifier.is_empty() {
        classifier
    } else {
        implicit_classifier_for_maven_type(effective_type).unwrap_or("")
    };
    (effective_classifier.to_string(), extension.to_string())
}

fn maven_type_has_jar_extension(type_field: &str) -> bool {
    matches!(
        type_field,
        "test-jar" | "ejb-client" | "ejb" | "bundle" | "maven-plugin" | "eclipse-plugin"
    )
}

fn implicit_classifier_for_maven_type(type_field: &str) -> Option<&'static str> {
    match type_field {
        "test-jar" => Some("tests"),
        "ejb-client" => Some("client"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_url_uses_classifier_and_extension() {
        assert_eq!(
            "https://repo.example.test/maven/org/example/demo/1.0/demo-1.0-sources.jar",
            artifact_url_for_descriptor(
                "https://repo.example.test/maven/",
                "org.example",
                "demo",
                "1.0",
                "sources",
                ".jar",
            )
        );
    }

    #[test]
    fn artifact_file_parts_extract_classifier_and_extension() {
        assert_eq!(
            Some(("linux".to_string(), "so".to_string())),
            artifact_file_parts_from_url(
                "https://repo.example.test/org/example/native/1.0/native-1.0-linux.so",
                "native",
                "1.0",
            )
        );
    }

    #[test]
    fn maven_artifact_shape_matches_gradle_special_types() {
        assert_eq!(
            ("tests".to_string(), "jar".to_string()),
            maven_artifact_shape("", "test-jar")
        );
        assert_eq!(
            ("client".to_string(), "jar".to_string()),
            maven_artifact_shape("", "ejb-client")
        );
        assert_eq!(
            ("custom".to_string(), "jar".to_string()),
            maven_artifact_shape("custom", "test-jar")
        );
        assert_eq!(
            ("".to_string(), "aar".to_string()),
            maven_artifact_shape("", "aar")
        );
    }
}
