//! Ivy XML descriptor parser.
//!
//! Parses Apache Ivy `ivy.xml` files to extract module metadata, configurations,
//! dependencies (with conf mappings), publications, and exclusions.
//!
//! Follows the same tag-scanning pattern as the existing POM parser in
//! `dependency_resolution.rs`.

/// Parsed Ivy descriptor.
#[derive(Debug, Clone, PartialEq)]
pub struct IvyDescriptor {
    pub org: String,
    pub module: String,
    pub revision: String,
    pub status: String,
    pub dependencies: Vec<IvyDependency>,
    pub configurations: Vec<IvyConfiguration>,
    pub publications: Vec<IvyArtifact>,
}

/// A dependency declaration in an Ivy descriptor.
#[derive(Debug, Clone, PartialEq)]
pub struct IvyDependency {
    pub org: String,
    pub name: String,
    pub rev: String,
    /// Conf mapping like "compile->default(compile)" or "runtime->*".
    pub conf: String,
    pub changing: bool,
    pub transitive: bool,
    pub optional: bool,
    pub exclusions: Vec<(String, String)>,
    /// Explicit artifact declarations inside this dependency.
    pub artifacts: Vec<IvyArtifact>,
}

/// An Ivy configuration (scope) definition.
#[derive(Debug, Clone, PartialEq)]
pub struct IvyConfiguration {
    pub name: String,
    pub description: String,
    pub extends: Vec<String>,
    pub visibility: String,
}

/// An artifact publication or dependency artifact.
#[derive(Debug, Clone, PartialEq)]
pub struct IvyArtifact {
    pub name: String,
    pub r#type: String,
    pub ext: String,
    pub conf: String,
    pub url: String,
}

// ---------------------------------------------------------------------------
// XML scanning helpers
// ---------------------------------------------------------------------------

/// Find the start position of `<tag>` (exact match) starting from `from`.
fn find_open_tag(bytes: &[u8], from: usize, tag: &[u8]) -> Option<usize> {
    let open = b"<";
    let needle = {
        let mut v = open.to_vec();
        v.extend_from_slice(tag);
        v
    };
    let mut pos = from;
    while pos < bytes.len() {
        if let Some(idx) = bytes[pos..].windows(needle.len()).position(|w| w == &needle[..]) {
            let abs = pos + idx;
            // Ensure it's actually a tag boundary (next char is whitespace or '>')
            let end = abs + needle.len();
            if end < bytes.len() {
                let next = bytes[end];
                if next == b'>' || next == b' ' || next == b'\n' || next == b'\r' || next == b'/' {
                    return Some(abs);
                }
            } else {
                return Some(abs);
            }
            pos = abs + 1;
        } else {
            break;
        }
    }
    None
}

/// Find the matching closing `</tag>` for a tag opened at `open_pos`.
fn find_end_tag(bytes: &[u8], open_pos: usize, tag: &[u8]) -> Option<usize> {
    let close_needle = {
        let mut v = b"</".to_vec();
        v.extend_from_slice(tag);
        v.push(b'>');
        v
    };
    let mut pos = open_pos + 1;
    while pos < bytes.len() {
        if let Some(idx) = bytes[pos..].windows(close_needle.len()).position(|w| w == &close_needle[..]) {
            return Some(pos + idx + close_needle.len());
        }
        pos += 1;
    }
    None
}

/// Extract text content of `<tag>...</tag>` within the range [from, to).
#[allow(dead_code)]
fn extract_tag(bytes: &[u8], from: usize, to: usize, tag: &[u8]) -> String {
    let open_needle = {
        let mut v = b"<".to_vec();
        v.extend_from_slice(tag);
        v.push(b'>');
        v
    };
    if let Some(start) = bytes[from..to].windows(open_needle.len()).position(|w| w == &open_needle[..]) {
        let content_start = from + start + open_needle.len();
        let close_needle = {
            let mut v = b"</".to_vec();
            v.extend_from_slice(tag);
            v.push(b'>');
            v
        };
        if let Some(end) = bytes[content_start..to].windows(close_needle.len()).position(|w| w == &close_needle[..]) {
            return String::from_utf8_lossy(&bytes[content_start..content_start + end]).trim().to_owned();
        }
    }
    String::new()
}

/// Extract an attribute value from a tag at `pos`. Returns the value between
/// the first `"` after `attr=` and the closing `"`.
fn extract_attr(bytes: &[u8], from: usize, to: usize, attr: &[u8]) -> Option<String> {
    let needle = {
        let mut v = attr.to_vec();
        v.extend_from_slice(b"=\"");
        v
    };
    if let Some(start) = bytes[from..to].windows(needle.len()).position(|w| w == &needle[..]) {
        let val_start = from + start + needle.len();
        if let Some(end) = bytes[val_start..to].iter().position(|&b| b == b'"') {
            let val = String::from_utf8_lossy(&bytes[val_start..val_start + end]).into_owned();
            return Some(val);
        }
    }
    None
}

/// Check if a tag at `pos` is self-closing (`<tag ... />`).
fn is_self_closing(bytes: &[u8], pos: usize, tag: &[u8]) -> bool {
    let end = pos + 1 + tag.len();
    if end >= bytes.len() {
        return false;
    }
    // Look for `/>` before `>` within the tag
    let search_end = match bytes[end..].iter().position(|&b| b == b'>') {
        Some(idx) => end + idx,
        None => return false,
    };
    bytes[end..search_end].contains(&b'/')
}

/// Find the end of a self-closing or opening tag at `pos`.
/// Skips over quoted attribute values (both single and double quotes).
fn tag_end(bytes: &[u8], pos: usize) -> usize {
    let mut i = pos;
    let len = bytes.len();
    while i < len {
        match bytes[i] {
            b'"' => {
                // Skip to closing quote
                if let Some(end) = bytes[i + 1..].iter().position(|&b| b == b'"') {
                    i += 1 + end + 1;
                } else {
                    i += 1;
                }
            }
            b'\'' => {
                // Skip to closing quote
                if let Some(end) = bytes[i + 1..].iter().position(|&b| b == b'\'') {
                    i += 1 + end + 1;
                } else {
                    i += 1;
                }
            }
            b'>' => return i + 1,
            _ => i += 1,
        }
    }
    len
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse an Ivy XML descriptor string.
pub fn parse_ivy(xml: &str) -> Result<IvyDescriptor, String> {
    let bytes = xml.as_bytes();
    if bytes.is_empty() {
        return Err("Empty Ivy descriptor".to_string());
    }

    // Parse <info>
    let info = parse_info(bytes).ok_or("Missing <info> section")?;

    // Parse <configurations>
    let configurations = parse_configurations(bytes);

    // Parse <dependencies>
    let dependencies = parse_dependencies(bytes);

    // Parse <publications>
    let publications = parse_publications(bytes);

    Ok(IvyDescriptor {
        org: info.0,
        module: info.1,
        revision: info.2,
        status: info.3,
        dependencies,
        configurations,
        publications,
    })
}

/// Parse `<info>` → (org, module, revision, status).
fn parse_info(bytes: &[u8]) -> Option<(String, String, String, String)> {
    let pos = find_open_tag(bytes, 0, b"info")?;
    let end = find_end_tag(bytes, pos, b"info").unwrap_or(tag_end(bytes, pos));

    let org = extract_attr(bytes, pos, end, b"organisation")
        .or_else(|| extract_attr(bytes, pos, end, b"org"));
    let module = extract_attr(bytes, pos, end, b"name")
        .or_else(|| extract_attr(bytes, pos, end, b"module"));
    let revision = extract_attr(bytes, pos, end, b"revision")
        .or_else(|| extract_attr(bytes, pos, end, b"rev"));
    let status = extract_attr(bytes, pos, end, b"status");

    Some((
        org.unwrap_or_default(),
        module.unwrap_or_default(),
        revision.unwrap_or_default(),
        status.unwrap_or_default(),
    ))
}

/// Parse all `<conf>` elements inside `<configurations>`.
fn parse_configurations(bytes: &[u8]) -> Vec<IvyConfiguration> {
    let mut configs = Vec::new();
    let pos = match find_open_tag(bytes, 0, b"configurations") {
        Some(p) => p,
        None => return configs,
    };
    let end = match find_end_tag(bytes, pos, b"configurations") {
        Some(p) => p,
        None => return configs,
    };

    let mut search = pos + b"<configurations>".len();
    while search < end {
        let conf_pos = match find_open_tag(bytes, search, b"conf") {
            Some(p) if p < end => p,
            _ => break,
        };
        let conf_end = tag_end(bytes, conf_pos);

        let name = extract_attr(bytes, conf_pos, conf_end, b"name").unwrap_or_default();
        let description = extract_attr(bytes, conf_pos, conf_end, b"description").unwrap_or_default();
        let extends_str = extract_attr(bytes, conf_pos, conf_end, b"extends").unwrap_or_default();
        let visibility = extract_attr(bytes, conf_pos, conf_end, b"visibility").unwrap_or_default();

        if !name.is_empty() {
            configs.push(IvyConfiguration {
                name,
                description,
                extends: if extends_str.is_empty() {
                    Vec::new()
                } else {
                    extends_str.split(',').map(|s| s.trim().to_string()).collect()
                },
                visibility: if visibility.is_empty() {
                    "public".to_string()
                } else {
                    visibility
                },
            });
        }

        search = conf_end;
    }

    configs
}

/// Parse all `<dependency>` elements inside `<dependencies>`.
fn parse_dependencies(bytes: &[u8]) -> Vec<IvyDependency> {
    let mut deps = Vec::new();
    let pos = match find_open_tag(bytes, 0, b"dependencies") {
        Some(p) => p,
        None => return deps,
    };
    let end = match find_end_tag(bytes, pos, b"dependencies") {
        Some(p) => p,
        None => return deps,
    };

    let mut search = pos + b"<dependencies>".len();
    while search < end {
        let dep_pos = match find_open_tag(bytes, search, b"dependency") {
            Some(p) if p < end => p,
            _ => break,
        };
        let dep_tag_end = tag_end(bytes, dep_pos);

        let org = extract_attr(bytes, dep_pos, dep_tag_end, b"org")
            .or_else(|| extract_attr(bytes, dep_pos, dep_tag_end, b"organisation"))
            .unwrap_or_default();
        let name = extract_attr(bytes, dep_pos, dep_tag_end, b"name").unwrap_or_default();
        let rev = extract_attr(bytes, dep_pos, dep_tag_end, b"rev")
            .or_else(|| extract_attr(bytes, dep_pos, dep_tag_end, b"revision"))
            .unwrap_or_default();
        let conf = extract_attr(bytes, dep_pos, dep_tag_end, b"conf")
            .unwrap_or_else(|| "default->default".to_string());
        let changing = extract_attr(bytes, dep_pos, dep_tag_end, b"changing").as_deref() == Some("true");
        let transitive = extract_attr(bytes, dep_pos, dep_tag_end, b"transitive").as_deref() != Some("false");
        let optional = extract_attr(bytes, dep_pos, dep_tag_end, b"optional").as_deref() == Some("true");

        // Parse <exclude> children
        let mut exclusions = Vec::new();
        let dep_end = if is_self_closing(bytes, dep_pos, b"dependency") {
            dep_tag_end
        } else {
            find_end_tag(bytes, dep_pos, b"dependency").unwrap_or(dep_tag_end)
        };

        let mut excl_search = dep_tag_end;
        while excl_search < dep_end {
            let excl_pos = match find_open_tag(bytes, excl_search, b"exclude") {
                Some(p) if p < dep_end => p,
                _ => break,
            };
            let excl_tag_end = tag_end(bytes, excl_pos);
            let excl_org = extract_attr(bytes, excl_pos, excl_tag_end, b"org")
                .or_else(|| extract_attr(bytes, excl_pos, excl_tag_end, b"organisation"))
                .unwrap_or_default();
            let excl_name = extract_attr(bytes, excl_pos, excl_tag_end, b"name")
                .or_else(|| extract_attr(bytes, excl_pos, excl_tag_end, b"module"))
                .unwrap_or_default();
            if !excl_org.is_empty() || !excl_name.is_empty() {
                exclusions.push((excl_org, excl_name));
            }
            excl_search = excl_tag_end;
        }

        // Parse <artifact> children
        let mut artifacts = Vec::new();
        let mut art_search = dep_tag_end;
        while art_search < dep_end {
            let art_pos = match find_open_tag(bytes, art_search, b"artifact") {
                Some(p) if p < dep_end => p,
                _ => break,
            };
            let art_tag_end = tag_end(bytes, art_pos);
            let art_name = extract_attr(bytes, art_pos, art_tag_end, b"name").unwrap_or_default();
            let art_type = extract_attr(bytes, art_pos, art_tag_end, b"type").unwrap_or_default();
            let art_ext = extract_attr(bytes, art_pos, art_tag_end, b"ext").unwrap_or_default();
            let art_conf = extract_attr(bytes, art_pos, art_tag_end, b"conf").unwrap_or_default();
            let art_url = extract_attr(bytes, art_pos, art_tag_end, b"url").unwrap_or_default();
            if !art_name.is_empty() {
                artifacts.push(IvyArtifact {
                    name: art_name,
                    r#type: if art_type.is_empty() { "jar".to_string() } else { art_type },
                    ext: if art_ext.is_empty() { "jar".to_string() } else { art_ext },
                    conf: if art_conf.is_empty() { "*".to_string() } else { art_conf },
                    url: art_url,
                });
            }
            art_search = art_tag_end;
        }

        if !name.is_empty() {
            deps.push(IvyDependency {
                org,
                name,
                rev,
                conf,
                changing,
                transitive,
                optional,
                exclusions,
                artifacts,
            });
        }

        search = dep_end;
    }

    deps
}

/// Parse all `<artifact>` elements inside `<publications>`.
fn parse_publications(bytes: &[u8]) -> Vec<IvyArtifact> {
    let mut pubs = Vec::new();
    let pos = match find_open_tag(bytes, 0, b"publications") {
        Some(p) => p,
        None => return pubs,
    };
    let end = match find_end_tag(bytes, pos, b"publications") {
        Some(p) => p,
        None => return pubs,
    };

    let mut search = pos + b"<publications>".len();
    while search < end {
        let art_pos = match find_open_tag(bytes, search, b"artifact") {
            Some(p) if p < end => p,
            _ => break,
        };
        let art_tag_end = tag_end(bytes, art_pos);

        let name = extract_attr(bytes, art_pos, art_tag_end, b"name").unwrap_or_default();
        let art_type = extract_attr(bytes, art_pos, art_tag_end, b"type").unwrap_or_default();
        let ext = extract_attr(bytes, art_pos, art_tag_end, b"ext").unwrap_or_default();
        let conf = extract_attr(bytes, art_pos, art_tag_end, b"conf").unwrap_or_default();
        let url = extract_attr(bytes, art_pos, art_tag_end, b"url").unwrap_or_default();

        if !name.is_empty() {
            pubs.push(IvyArtifact {
                name,
                r#type: if art_type.is_empty() { "jar".to_string() } else { art_type },
                ext: if ext.is_empty() { "jar".to_string() } else { ext },
                conf: if conf.is_empty() { "*".to_string() } else { conf },
                url,
            });
        }

        search = art_tag_end;
    }

    pubs
}

/// Parse a conf mapping like "compile->default(compile)" into
/// (from_conf, to_conf, to_conf_ext).
/// Returns ("*", "*") for "*" mappings.
pub fn parse_conf_mapping(mapping: &str) -> (String, String, String) {
    let mut parts = mapping.splitn(2, "->");
    let from = parts.next().unwrap_or("*").trim().to_string();
    let to_full = parts.next().unwrap_or("*").trim();

    // Extract inner part from "default(compile)"
    let (to, ext) = if let Some(paren) = to_full.find('(') {
        let base = to_full[..paren].trim();
        let inner = if to_full.ends_with(')') {
            &to_full[paren + 1..to_full.len() - 1]
        } else {
            &to_full[paren + 1..]
        };
        (base.to_string(), inner.trim().to_string())
    } else {
        (to_full.to_string(), String::new())
    };

    (from, to, ext)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_IVY: &str = r#"
<ivy-module version="2.0">
  <info organisation="com.example" module="mylib" revision="1.0" status="release"/>
  <configurations>
    <conf name="default" visibility="public"/>
    <conf name="compile" extends="default" visibility="public"/>
    <conf name="runtime" extends="compile" visibility="public"/>
  </configurations>
  <dependencies>
    <dependency org="org.slf4j" name="slf4j-api" rev="2.0.7" conf="compile->default"/>
  </dependencies>
  <publications>
    <artifact name="mylib" type="jar" ext="jar" conf="runtime"/>
  </publications>
</ivy-module>
"#;

    #[test]
    fn test_parse_minimal_ivy() {
        let desc = parse_ivy(MINIMAL_IVY).unwrap();
        assert_eq!(desc.org, "com.example");
        assert_eq!(desc.module, "mylib");
        assert_eq!(desc.revision, "1.0");
        assert_eq!(desc.status, "release");
    }

    #[test]
    fn test_parse_configurations() {
        let desc = parse_ivy(MINIMAL_IVY).unwrap();
        assert_eq!(desc.configurations.len(), 3);
        assert_eq!(desc.configurations[0].name, "default");
        assert_eq!(desc.configurations[1].name, "compile");
        assert_eq!(desc.configurations[1].extends, vec!["default"]);
        assert_eq!(desc.configurations[2].name, "runtime");
        assert_eq!(desc.configurations[2].extends, vec!["compile"]);
    }

    #[test]
    fn test_parse_dependency() {
        let desc = parse_ivy(MINIMAL_IVY).unwrap();
        assert_eq!(desc.dependencies.len(), 1);
        let dep = &desc.dependencies[0];
        assert_eq!(dep.org, "org.slf4j");
        assert_eq!(dep.name, "slf4j-api");
        assert_eq!(dep.rev, "2.0.7");
        assert_eq!(dep.conf, "compile->default");
        assert!(dep.transitive);
        assert!(!dep.optional);
    }

    #[test]
    fn test_parse_publications() {
        let desc = parse_ivy(MINIMAL_IVY).unwrap();
        assert_eq!(desc.publications.len(), 1);
        let art = &desc.publications[0];
        assert_eq!(art.name, "mylib");
        assert_eq!(art.r#type, "jar");
        assert_eq!(art.conf, "runtime");
    }

    #[test]
    fn test_parse_dependency_with_exclusions() {
        let xml = r#"
<ivy-module version="2.0">
  <info organisation="com.example" module="app" revision="1.0"/>
  <dependencies>
    <dependency org="org.springframework" name="spring-core" rev="5.3.20" conf="compile->default">
      <exclude org="commons-logging" module="commons-logging"/>
    </dependency>
  </dependencies>
</ivy-module>
"#;
        let desc = parse_ivy(xml).unwrap();
        assert_eq!(desc.dependencies.len(), 1);
        assert_eq!(desc.dependencies[0].exclusions.len(), 1);
        assert_eq!(desc.dependencies[0].exclusions[0], ("commons-logging".to_string(), "commons-logging".to_string()));
    }

    #[test]
    fn test_parse_dependency_changing_optional() {
        let xml = r#"
<ivy-module version="2.0">
  <info organisation="com.example" module="app" revision="1.0"/>
  <dependencies>
    <dependency org="com.example" name="snap" rev="1.0-SNAPSHOT" changing="true" transitive="false" optional="true" conf="compile->default"/>
  </dependencies>
</ivy-module>
"#;
        let desc = parse_ivy(xml).unwrap();
        let dep = &desc.dependencies[0];
        assert!(dep.changing);
        assert!(!dep.transitive);
        assert!(dep.optional);
    }

    #[test]
    fn test_parse_dependency_with_artifacts() {
        let xml = r#"
<ivy-module version="2.0">
  <info organisation="com.example" module="app" revision="1.0"/>
  <dependencies>
    <dependency org="com.example" name="native" rev="1.0" conf="runtime->default">
      <artifact name="native-linux" type="so" ext="so" conf="linux"/>
      <artifact name="native-darwin" type="dylib" ext="dylib" conf="macos"/>
    </dependency>
  </dependencies>
</ivy-module>
"#;
        let desc = parse_ivy(xml).unwrap();
        let dep = &desc.dependencies[0];
        assert_eq!(dep.artifacts.len(), 2);
        assert_eq!(dep.artifacts[0].name, "native-linux");
        assert_eq!(dep.artifacts[0].r#type, "so");
        assert_eq!(dep.artifacts[0].conf, "linux");
        assert_eq!(dep.artifacts[1].name, "native-darwin");
        assert_eq!(dep.artifacts[1].ext, "dylib");
    }

    #[test]
    fn test_parse_empty_ivy() {
        assert!(parse_ivy("").is_err());
    }

    #[test]
    fn test_parse_no_dependencies() {
        let xml = r#"
<ivy-module version="2.0">
  <info org="com.example" module="empty" rev="1.0"/>
</ivy-module>
"#;
        let desc = parse_ivy(xml).unwrap();
        assert!(desc.dependencies.is_empty());
        assert!(desc.configurations.is_empty());
        assert!(desc.publications.is_empty());
    }

    #[test]
    fn test_parse_conf_mapping_simple() {
        let (from, to, ext) = parse_conf_mapping("compile->default");
        assert_eq!(from, "compile");
        assert_eq!(to, "default");
        assert!(ext.is_empty());
    }

    #[test]
    fn test_parse_conf_mapping_with_inner() {
        let (from, to, ext) = parse_conf_mapping("compile->default(compile)");
        assert_eq!(from, "compile");
        assert_eq!(to, "default");
        assert_eq!(ext, "compile");
    }

    #[test]
    fn test_parse_conf_mapping_star() {
        let (from, to, ext) = parse_conf_mapping("*->*");
        assert_eq!(from, "*");
        assert_eq!(to, "*");
        assert!(ext.is_empty());
    }

    #[test]
    fn test_parse_conf_mapping_runtime() {
        let (from, to, ext) = parse_conf_mapping("runtime->default(runtime)");
        assert_eq!(from, "runtime");
        assert_eq!(to, "default");
        assert_eq!(ext, "runtime");
    }

    #[test]
    fn test_parse_multiple_exclusions() {
        let xml = r#"
<ivy-module version="2.0">
  <info organisation="com.example" module="app" revision="1.0"/>
  <dependencies>
    <dependency org="org.springframework.boot" name="spring-boot-starter" rev="2.7.0" conf="compile->default">
      <exclude org="org.springframework.boot" module="spring-boot-starter-logging"/>
      <exclude org="org.springframework.boot" module="spring-boot-starter-tomcat"/>
    </dependency>
  </dependencies>
</ivy-module>
"#;
        let desc = parse_ivy(xml).unwrap();
        let dep = &desc.dependencies[0];
        assert_eq!(dep.exclusions.len(), 2);
    }

    #[test]
    fn test_parse_multiple_dependencies() {
        let xml = r#"
<ivy-module version="2.0">
  <info org="com.example" module="app" rev="2.0"/>
  <dependencies>
    <dependency org="com.google.guava" name="guava" rev="31.1-jre" conf="compile->default"/>
    <dependency org="org.apache.commons" name="commons-lang3" rev="3.12.0" conf="compile->default"/>
    <dependency org="junit" name="junit" rev="4.13.2" conf="test->default"/>
  </dependencies>
</ivy-module>
"#;
        let desc = parse_ivy(xml).unwrap();
        assert_eq!(desc.dependencies.len(), 3);
        assert_eq!(desc.dependencies[0].name, "guava");
        assert_eq!(desc.dependencies[1].name, "commons-lang3");
        assert_eq!(desc.dependencies[2].name, "junit");
    }

    #[test]
    fn test_self_closing_dependency() {
        let xml = r#"
<ivy-module version="2.0">
  <info org="com.example" module="app" rev="1.0"/>
  <dependencies>
    <dependency org="com.example" name="lib" rev="1.0" conf="compile->default"/>
  </dependencies>
</ivy-module>
"#;
        let desc = parse_ivy(xml).unwrap();
        assert_eq!(desc.dependencies.len(), 1);
    }
}
