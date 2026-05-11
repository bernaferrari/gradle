use std::collections::HashMap;

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
}
