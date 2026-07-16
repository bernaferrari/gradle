//! Structured IR for settings-level `includeBuild` composite declarations.
//!
//! Parsing is intentionally fail-closed for execution: the IR exists so diagnostics
//! can name concrete included-build paths. Substitution / included-build execution
//! is not enabled by this module.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Feature marker emitted when settings declare `includeBuild(...)`.
pub const COMPOSITE_SUBSTITUTION_SETTINGS: &str = "composite-substitution:settings";

/// One settings-level included build captured from DSL source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalIncludedBuild {
    /// Path argument as written in settings (`"included-lib"`, `'../plugins'`, …).
    pub path: String,
    /// Best-effort name hint (final path segment), empty when unknown.
    pub name_hint: String,
    /// Settings file that declared the includeBuild.
    pub source_file: String,
}

impl CanonicalIncludedBuild {
    pub fn new(path: impl Into<String>, source_file: impl Into<String>) -> Self {
        let path = path.into();
        let name_hint = name_hint_from_path(&path);
        Self {
            path,
            name_hint,
            source_file: source_file.into(),
        }
    }
}

/// Parse `includeBuild("…")` / `includeBuild('…')` / Groovy `includeBuild '…'` lines.
pub fn parse_include_build_declarations(
    text: &str,
    source_file: &str,
) -> Vec<CanonicalIncludedBuild> {
    let mut builds = Vec::new();
    for line in text.lines() {
        if let Some(path) = parse_include_build_path(line) {
            builds.push(CanonicalIncludedBuild::new(path, source_file));
        }
    }
    builds
}

/// Path-only view used by directory resolution helpers.
pub fn parse_include_build_paths(text: &str) -> Vec<String> {
    text.lines().filter_map(parse_include_build_path).collect()
}

fn parse_include_build_path(line: &str) -> Option<String> {
    let trimmed = strip_line_comment(line.trim());
    if trimmed.is_empty() || !trimmed.contains("includeBuild") {
        return None;
    }

    // Require the call to begin the statement (allow leading labels/annotations later).
    let include_idx = trimmed.find("includeBuild")?;
    if trimmed[..include_idx]
        .chars()
        .any(|ch| !ch.is_whitespace())
    {
        return None;
    }

    let rest = trimmed[include_idx + "includeBuild".len()..].trim_start();
    let rest = rest.strip_prefix('(').map(str::trim_start).unwrap_or(rest);
    // Drop trailing call / block crumbs: `)`, `{ … }`, trailing comments already stripped.
    let rest = rest
        .trim_end_matches(';')
        .trim_end()
        .strip_suffix(')')
        .map(str::trim_end)
        .unwrap_or(rest)
        .trim_end_matches('{')
        .trim();

    extract_quoted(rest).filter(|path| !path.is_empty())
}

fn strip_line_comment(line: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    let bytes = line.as_bytes();
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        let ch = bytes[i] as char;
        match ch {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '/' if !in_single && !in_double && bytes[i + 1] as char == '/' => {
                return line[..i].trim_end();
            }
            _ => {}
        }
        i += 1;
    }
    line
}

fn extract_quoted(input: &str) -> Option<String> {
    let input = input.trim();
    if let Some(rest) = input.strip_prefix('"') {
        return rest.split_once('"').map(|(value, _)| value.to_string());
    }
    if let Some(rest) = input.strip_prefix('\'') {
        return rest.split_once('\'').map(|(value, _)| value.to_string());
    }
    None
}

fn name_hint_from_path(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && *name != "." && *name != "..")
        .unwrap_or("")
        .to_string()
}

/// Stable, comma-separated path list for diagnostics and feature encoding.
pub fn format_included_build_paths(builds: &[CanonicalIncludedBuild]) -> String {
    let mut paths = builds
        .iter()
        .map(|build| build.path.as_str())
        .filter(|path| !path.is_empty())
        .collect::<Vec<_>>();
    paths.sort_unstable();
    paths.dedup();
    paths.join(",")
}

/// Encode the fail-closed feature marker, optionally attaching IR paths.
pub fn encode_composite_substitution_feature(builds: &[CanonicalIncludedBuild]) -> String {
    let paths = format_included_build_paths(builds);
    if paths.is_empty() {
        COMPOSITE_SUBSTITUTION_SETTINGS.to_string()
    } else {
        format!("{COMPOSITE_SUBSTITUTION_SETTINGS}@{paths}")
    }
}

pub fn is_composite_substitution_feature(feature: &str) -> bool {
    let feature = feature.trim();
    feature == COMPOSITE_SUBSTITUTION_SETTINGS
        || feature.starts_with(&format!("{COMPOSITE_SUBSTITUTION_SETTINGS}@"))
}

/// Paths embedded in an extended feature marker (`…@path1,path2`).
pub fn paths_from_composite_feature(feature: &str) -> Vec<String> {
    let feature = feature.trim();
    let Some(rest) = feature.strip_prefix(COMPOSITE_SUBSTITUTION_SETTINGS) else {
        return Vec::new();
    };
    let Some(paths) = rest.strip_prefix('@') else {
        return Vec::new();
    };
    paths
        .split(',')
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// Admission diagnostic for settings-level composite substitution.
pub fn composite_substitution_reason(configuration_name: &str, feature: &str) -> String {
    let paths = paths_from_composite_feature(feature);
    if paths.is_empty() {
        format!(
            "dependency configuration '{}' uses JVM-owned settings/includeBuild/buildSrc composite setup ('{}'); Rust cannot yet separate that configuration setup from selected root task execution, so strict Rust execution is rejected before task dispatch",
            configuration_name, COMPOSITE_SUBSTITUTION_SETTINGS
        )
    } else {
        format!(
            "dependency configuration '{}' uses JVM-owned settings/includeBuild/buildSrc composite setup ('{}') for included build path(s) [{}]; Rust cannot yet separate that configuration setup from selected root task execution, so strict Rust execution is rejected before task dispatch",
            configuration_name,
            COMPOSITE_SUBSTITUTION_SETTINGS,
            paths.join(", ")
        )
    }
}

/// Configuration-replay rejection when settings IR lists included builds.
pub fn composite_settings_replay_reason(builds: &[CanonicalIncludedBuild]) -> Option<String> {
    if builds.is_empty() {
        return None;
    }
    let paths = format_included_build_paths(builds);
    Some(format!(
        "settings includeBuild composite IR is not executable in Rust configuration replay (paths: [{paths}])"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiple_kotlin_and_groovy_include_builds() {
        let text = r#"
rootProject.name = "root"
includeBuild("included-lib")
includeBuild("plugins")
includeBuild 'legacy-tools'
// includeBuild("commented-out")
include(":app")
"#;
        let builds = parse_include_build_declarations(text, "settings.gradle.kts");
        assert_eq!(builds.len(), 3);
        assert_eq!(builds[0].path, "included-lib");
        assert_eq!(builds[0].name_hint, "included-lib");
        assert_eq!(builds[0].source_file, "settings.gradle.kts");
        assert_eq!(builds[1].path, "plugins");
        assert_eq!(builds[2].path, "legacy-tools");
        assert_eq!(
            parse_include_build_paths(text),
            vec![
                "included-lib".to_string(),
                "plugins".to_string(),
                "legacy-tools".to_string()
            ]
        );
    }

    #[test]
    fn encodes_and_decodes_feature_paths() {
        let builds = vec![
            CanonicalIncludedBuild::new("plugins", "settings.gradle.kts"),
            CanonicalIncludedBuild::new("included-lib", "settings.gradle.kts"),
        ];
        let feature = encode_composite_substitution_feature(&builds);
        assert_eq!(
            feature,
            "composite-substitution:settings@included-lib,plugins"
        );
        assert!(is_composite_substitution_feature(&feature));
        assert_eq!(
            paths_from_composite_feature(&feature),
            vec!["included-lib".to_string(), "plugins".to_string()]
        );
        let reason = composite_substitution_reason("::composite-build", &feature);
        assert!(reason.contains("included-lib"));
        assert!(reason.contains("plugins"));
        assert!(reason.contains("before task dispatch"));
    }
}
