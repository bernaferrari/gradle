//! Gradle version catalog parser (`libs.versions.toml`).
//!
//! Parses the TOML-based dependency catalog format used by Gradle since 7.0.
//! A version catalog has four sections:
//! - `[versions]` — version aliases (e.g., `junit = "4.13"`)
//! - `[libraries]` — library aliases with group/artifact/version references
//! - `[bundles]` — groups of library aliases
//! - `[plugins]` — plugin aliases

use std::collections::HashMap;

use serde::Deserialize;
use tonic::{Request, Response, Status};

use crate::proto::{
    version_catalog_service_server::VersionCatalogService, ParseVersionCatalogRequest,
    ParseVersionCatalogResponse, ProtoBundle, ProtoLibrary, ProtoPlugin, ProtoVersion,
};

// ---------------------------------------------------------------------------
// TOML deserialization types
// ---------------------------------------------------------------------------

/// Top-level TOML structure for `libs.versions.toml`.
#[derive(Debug, Default, Deserialize)]
pub struct VersionCatalogToml {
    #[serde(default)]
    pub versions: HashMap<String, TomlVersionValue>,
    #[serde(default)]
    pub libraries: HashMap<String, TomlLibraryValue>,
    #[serde(default)]
    pub bundles: HashMap<String, Vec<String>>,
    #[serde(default)]
    pub plugins: HashMap<String, TomlPluginValue>,
}

/// A version can be a simple string or a rich object with `strictly`/`prefer`/`require`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum TomlVersionValue {
    Simple(String),
    Rich {
        strictly: Option<String>,
        prefer: Option<String>,
        require: Option<String>,
    },
}

/// A library can be a simple string ("group:artifact:version") or a rich object.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum TomlLibraryValue {
    Simple(String),
    Rich {
        group: Option<String>,
        module: Option<TomlModuleValue>,
        name: Option<String>,
        version: Option<TomlVersionRef>,
    },
}

/// Module reference (group:name as a single string "group:name").
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum TomlModuleValue {
    Simple(String),
}

/// Version reference — either a string (literal or alias) or a rich object.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum TomlVersionRef {
    Simple(String),
    Rich {
        strictly: Option<String>,
        prefer: Option<String>,
        require: Option<String>,
        reject: Option<Vec<String>>,
        #[serde(rename = "ref")]
        ref_alias: Option<String>,
    },
}

/// A plugin can be a simple string (ID) or a rich object.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum TomlPluginValue {
    Simple(String),
    Rich {
        id: Option<String>,
        version: Option<TomlVersionRef>,
    },
}

// ---------------------------------------------------------------------------
// Parsed IR types
// ---------------------------------------------------------------------------

/// A fully resolved version entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedVersion {
    pub alias: String,
    pub version: String,
    pub strictly: Option<String>,
    pub prefer: Option<String>,
    pub require: Option<String>,
}

/// A fully resolved library entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedLibrary {
    pub alias: String,
    pub group: String,
    pub artifact: String,
    pub version_ref: Option<String>,
    pub version_literal: Option<String>,
}

/// A fully resolved bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedBundle {
    pub alias: String,
    pub library_aliases: Vec<String>,
}

/// A fully resolved plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPlugin {
    pub alias: String,
    pub id: String,
    pub version_ref: Option<String>,
    pub version_literal: Option<String>,
}

/// Complete parsed version catalog.
#[derive(Debug, Clone, Default)]
pub struct VersionCatalog {
    pub versions: Vec<ResolvedVersion>,
    pub libraries: Vec<ResolvedLibrary>,
    pub bundles: Vec<ResolvedBundle>,
    pub plugins: Vec<ResolvedPlugin>,
}

// ---------------------------------------------------------------------------
// Service implementation
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct VersionCatalogServiceImpl;

impl VersionCatalogServiceImpl {
    pub const fn new() -> Self {
        Self
    }

    /// Parse a TOML version catalog from string content.
    pub fn parse_catalog(content: &str) -> Result<VersionCatalog, String> {
        let toml: VersionCatalogToml =
            toml::from_str(content).map_err(|e| format!("TOML parse error: {}", e))?;

        let mut catalog = VersionCatalog::default();

        // Parse versions
        for (alias, value) in &toml.versions {
            match value {
                TomlVersionValue::Simple(v) => {
                    catalog.versions.push(ResolvedVersion {
                        alias: alias.clone(),
                        version: v.clone(),
                        strictly: None,
                        prefer: None,
                        require: None,
                    });
                }
                TomlVersionValue::Rich {
                    strictly,
                    prefer,
                    require,
                } => {
                    catalog.versions.push(ResolvedVersion {
                        alias: alias.clone(),
                        version: require
                            .as_deref()
                            .or(prefer.as_deref())
                            .or(strictly.as_deref())
                            .unwrap_or("")
                            .to_string(),
                        strictly: strictly.clone(),
                        prefer: prefer.clone(),
                        require: require.clone(),
                    });
                }
            }
        }

        // Parse libraries
        for (alias, value) in &toml.libraries {
            match value {
                TomlLibraryValue::Simple(notation) => {
                    // Parse "group:artifact:version" notation
                    let parts: Vec<&str> = notation.splitn(3, ':').collect();
                    let (group, artifact, version) = match parts.as_slice() {
                        [g, a, v] => (*g, *a, Some(*v)),
                        [g, a] => (*g, *a, None),
                        _ => ("", notation.as_str(), None),
                    };
                    let version_literal = version.map(String::from);
                    catalog.libraries.push(ResolvedLibrary {
                        alias: alias.clone(),
                        group: group.to_string(),
                        artifact: artifact.to_string(),
                        version_ref: None,
                        version_literal,
                    });
                }
                TomlLibraryValue::Rich {
                    group,
                    module,
                    name,
                    version,
                } => {
                    let lib_group = group
                        .as_deref()
                        .or_else(|| {
                            module.as_ref().and_then(|m| {
                                let s = match m {
                                    TomlModuleValue::Simple(s) => s.as_str(),
                                };
                                s.split(':').next()
                            })
                        })
                        .unwrap_or("")
                        .to_string();

                    let lib_artifact = name
                        .as_deref()
                        .or_else(|| {
                            module.as_ref().and_then(|m| {
                                let s = match m {
                                    TomlModuleValue::Simple(s) => s.as_str(),
                                };
                                s.split(':').nth(1)
                            })
                        })
                        .unwrap_or("")
                        .to_string();

                    let (version_ref, version_literal) = match version {
                        None => (None, None),
                        Some(TomlVersionRef::Simple(v)) => {
                            // If the version string matches a version alias, it's a reference
                            if toml.versions.contains_key(v) {
                                (Some(v.clone()), None)
                            } else {
                                (None, Some(v.clone()))
                            }
                        }
                        Some(TomlVersionRef::Rich { require, ref_alias, .. }) => {
                            if let Some(alias) = ref_alias {
                                (Some(alias.clone()), None)
                            } else {
                                (None, require.clone())
                            }
                        }
                    };

                    catalog.libraries.push(ResolvedLibrary {
                        alias: alias.clone(),
                        group: lib_group,
                        artifact: lib_artifact,
                        version_ref,
                        version_literal,
                    });
                }
            }
        }

        // Parse bundles
        for (alias, libs) in &toml.bundles {
            catalog.bundles.push(ResolvedBundle {
                alias: alias.clone(),
                library_aliases: libs.clone(),
            });
        }

        // Parse plugins
        for (alias, value) in &toml.plugins {
            match value {
                TomlPluginValue::Simple(id) => {
                    catalog.plugins.push(ResolvedPlugin {
                        alias: alias.clone(),
                        id: id.clone(),
                        version_ref: None,
                        version_literal: None,
                    });
                }
                TomlPluginValue::Rich { id, version } => {
                    let plugin_id = id.as_deref().unwrap_or("").to_string();
                    let (version_ref, version_literal) = match version {
                        None => (None, None),
                        Some(TomlVersionRef::Simple(v)) => {
                            if toml.versions.contains_key(v) {
                                (Some(v.clone()), None)
                            } else {
                                (None, Some(v.clone()))
                            }
                        }
                        Some(TomlVersionRef::Rich { require, ref_alias, .. }) => {
                            if let Some(alias) = ref_alias {
                                (Some(alias.clone()), None)
                            } else {
                                (None, require.clone())
                            }
                        }
                    };
                    catalog.plugins.push(ResolvedPlugin {
                        alias: alias.clone(),
                        id: plugin_id,
                        version_ref,
                        version_literal,
                    });
                }
            }
        }

        Ok(catalog)
    }
}

#[tonic::async_trait]
impl VersionCatalogService for VersionCatalogServiceImpl {
    async fn parse_version_catalog(
        &self,
        request: Request<ParseVersionCatalogRequest>,
    ) -> Result<Response<ParseVersionCatalogResponse>, Status> {
        let req = request.into_inner();
        let catalog = Self::parse_catalog(&req.content).map_err(|e| {
            Status::invalid_argument(format!("Failed to parse version catalog: {}", e))
        })?;

        let versions = catalog
            .versions
            .into_iter()
            .map(|v| ProtoVersion {
                alias: v.alias,
                version: v.version,
            })
            .collect();

        let libraries = catalog
            .libraries
            .into_iter()
            .map(|l| ProtoLibrary {
                alias: l.alias,
                group: l.group,
                artifact: l.artifact,
                version_ref: l.version_ref.unwrap_or_default(),
                version_literal: l.version_literal.unwrap_or_default(),
            })
            .collect();

        let bundles = catalog
            .bundles
            .into_iter()
            .map(|b| ProtoBundle {
                alias: b.alias,
                library_aliases: b.library_aliases,
            })
            .collect();

        let plugins = catalog
            .plugins
            .into_iter()
            .map(|p| ProtoPlugin {
                alias: p.alias,
                id: p.id,
                version_ref: p.version_ref.unwrap_or_default(),
                version_literal: p.version_literal.unwrap_or_default(),
            })
            .collect();

        Ok(Response::new(ParseVersionCatalogResponse {
            versions,
            libraries,
            bundles,
            plugins,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_CATALOG: &str = r#"
[versions]
junit = "4.13"
guava = "32.1.2-jre"
kotlin = { strictly = "1.9.0" }

[libraries]
junit-junit = { group = "junit", name = "junit", version.ref = "junit" }
guava = "com.google.guava:guava:32.1.2-jre"
commons-lang3 = { group = "org.apache.commons", name = "commons-lang3", version = "3.14.0" }

[bundles]
testing = ["junit-junit"]

[plugins]
kotlin-jvm = { id = "org.jetbrains.kotlin.jvm", version.ref = "kotlin" }
spring-boot = "org.springframework.boot"
"#;

    #[test]
    fn test_parse_versions() {
        let catalog = VersionCatalogServiceImpl::parse_catalog(SAMPLE_CATALOG).unwrap();
        assert_eq!(catalog.versions.len(), 3);

        let junit = catalog.versions.iter().find(|v| v.alias == "junit").unwrap();
        assert_eq!(junit.version, "4.13");
        assert!(junit.strictly.is_none());

        let kotlin = catalog
            .versions
            .iter()
            .find(|v| v.alias == "kotlin")
            .unwrap();
        assert_eq!(kotlin.strictly.as_deref(), Some("1.9.0"));
    }

    #[test]
    fn test_parse_libraries() {
        let catalog = VersionCatalogServiceImpl::parse_catalog(SAMPLE_CATALOG).unwrap();
        assert_eq!(catalog.libraries.len(), 3);

        let junit = catalog
            .libraries
            .iter()
            .find(|l| l.alias == "junit-junit")
            .unwrap();
        assert_eq!(junit.group, "junit");
        assert_eq!(junit.artifact, "junit");
        assert_eq!(junit.version_ref.as_deref(), Some("junit"));

        let guava = catalog
            .libraries
            .iter()
            .find(|l| l.alias == "guava")
            .unwrap();
        assert_eq!(guava.group, "com.google.guava");
        assert_eq!(guava.artifact, "guava");
        assert_eq!(
            guava.version_literal.as_deref(),
            Some("32.1.2-jre")
        );

        let commons = catalog
            .libraries
            .iter()
            .find(|l| l.alias == "commons-lang3")
            .unwrap();
        assert_eq!(commons.group, "org.apache.commons");
        assert_eq!(commons.version_literal.as_deref(), Some("3.14.0"));
    }

    #[test]
    fn test_parse_bundles() {
        let catalog = VersionCatalogServiceImpl::parse_catalog(SAMPLE_CATALOG).unwrap();
        assert_eq!(catalog.bundles.len(), 1);

        let testing = catalog.bundles.first().unwrap();
        assert_eq!(testing.alias, "testing");
        assert_eq!(testing.library_aliases, vec!["junit-junit"]);
    }

    #[test]
    fn test_parse_plugins() {
        let catalog = VersionCatalogServiceImpl::parse_catalog(SAMPLE_CATALOG).unwrap();
        assert_eq!(catalog.plugins.len(), 2);

        let kotlin = catalog
            .plugins
            .iter()
            .find(|p| p.alias == "kotlin-jvm")
            .unwrap();
        assert_eq!(kotlin.id, "org.jetbrains.kotlin.jvm");
        assert_eq!(kotlin.version_ref.as_deref(), Some("kotlin"));

        let spring = catalog
            .plugins
            .iter()
            .find(|p| p.alias == "spring-boot")
            .unwrap();
        assert_eq!(spring.id, "org.springframework.boot");
        assert!(spring.version_ref.is_none());
    }

    #[test]
    fn test_empty_catalog() {
        let catalog = VersionCatalogServiceImpl::parse_catalog("").unwrap();
        assert!(catalog.versions.is_empty());
        assert!(catalog.libraries.is_empty());
        assert!(catalog.bundles.is_empty());
        assert!(catalog.plugins.is_empty());
    }

    #[test]
    fn test_invalid_toml() {
        let result = VersionCatalogServiceImpl::parse_catalog("not valid [[[[toml");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("TOML parse error"));
    }

    #[test]
    fn test_only_versions() {
        let catalog =
            VersionCatalogServiceImpl::parse_catalog("[versions]\nfoo = \"1.0\"").unwrap();
        assert_eq!(catalog.versions.len(), 1);
        assert!(catalog.libraries.is_empty());
    }

    #[test]
    fn test_rich_version_prefer() {
        let toml = r#"
[versions]
android-sdk = { prefer = "33", require = "33+" }
"#;
        let catalog = VersionCatalogServiceImpl::parse_catalog(toml).unwrap();
        let v = catalog.versions.first().unwrap();
        assert_eq!(v.version, "33+");
        assert_eq!(v.prefer.as_deref(), Some("33"));
        assert_eq!(v.require.as_deref(), Some("33+"));
    }

    #[test]
    fn test_module_notation() {
        let toml = r#"
[libraries]
core = { module = "com.example:core", version = "1.0" }
"#;
        let catalog = VersionCatalogServiceImpl::parse_catalog(toml).unwrap();
        let core = catalog.libraries.first().unwrap();
        assert_eq!(core.group, "com.example");
        assert_eq!(core.artifact, "core");
        assert_eq!(core.version_literal.as_deref(), Some("1.0"));
    }

    #[test]
    fn test_multiple_bundles() {
        let toml = r#"
[bundles]
testing = ["junit", "mockito"]
networking = ["okhttp", "retrofit"]
"#;
        let catalog = VersionCatalogServiceImpl::parse_catalog(toml).unwrap();
        assert_eq!(catalog.bundles.len(), 2);
        let testing = catalog
            .bundles
            .iter()
            .find(|b| b.alias == "testing")
            .unwrap();
        assert_eq!(testing.library_aliases.len(), 2);
    }

    #[test]
    fn test_plugin_with_literal_version() {
        let toml = r#"
[plugins]
spotless = { id = "com.diffplug.spotless", version = "6.25.0" }
"#;
        let catalog = VersionCatalogServiceImpl::parse_catalog(toml).unwrap();
        let plugin = catalog.plugins.first().unwrap();
        assert_eq!(plugin.id, "com.diffplug.spotless");
        assert_eq!(plugin.version_literal.as_deref(), Some("6.25.0"));
    }

    #[test]
    fn test_version_ref_resolution() {
        let toml = r#"
[versions]
kotlin = "1.9.22"

[libraries]
kotlin-stdlib = { group = "org.jetbrains.kotlin", name = "kotlin-stdlib", version.ref = "kotlin" }
"#;
        let catalog = VersionCatalogServiceImpl::parse_catalog(toml).unwrap();
        let lib = catalog.libraries.first().unwrap();
        assert_eq!(lib.version_ref.as_deref(), Some("kotlin"));
        assert!(lib.version_literal.is_none());
    }
}
