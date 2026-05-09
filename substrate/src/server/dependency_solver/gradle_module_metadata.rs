use serde::Deserialize;
use std::collections::BTreeMap;

use super::ivyresolve::strategy::compare_versions;

#[derive(Debug, Clone, PartialEq, Eq)]
struct StaticVersionRequirement {
    version: String,
    rejects: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleDependency {
    pub group: String,
    pub module: String,
    pub version: String,
    pub exclusions: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleArtifact {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedVariant {
    pub name: String,
    pub dependencies: Vec<ModuleDependency>,
    pub artifacts: Vec<ModuleArtifact>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleMetadataSelection {
    Selected(SelectedVariant),
    Unsupported(String),
}

#[derive(Debug, Deserialize)]
struct GradleModuleMetadata {
    #[serde(default)]
    component: Component,
    #[serde(default)]
    variants: Vec<Variant>,
}

#[derive(Debug, Default, Deserialize)]
struct Component {
    #[serde(default)]
    group: String,
    #[serde(default)]
    module: String,
    #[serde(default)]
    version: String,
}

#[derive(Debug, Deserialize)]
struct Variant {
    name: String,
    #[serde(default)]
    attributes: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    dependencies: Vec<VariantDependency>,
    #[serde(default, rename = "dependencyConstraints")]
    dependency_constraints: Vec<VariantDependency>,
    #[serde(default)]
    files: Vec<VariantFile>,
    #[serde(default)]
    capabilities: Vec<Capability>,
    #[serde(default, rename = "available-at")]
    available_at: Option<AvailableAt>,
}

#[derive(Debug, Deserialize)]
struct VariantDependency {
    group: String,
    module: String,
    #[serde(default)]
    version: VersionRequirement,
    #[serde(default)]
    excludes: Vec<DependencyExclude>,
}

#[derive(Debug, Default, Deserialize)]
struct VersionRequirement {
    #[serde(default)]
    requires: String,
    #[serde(default)]
    prefers: String,
    #[serde(default)]
    strictly: String,
    #[serde(default)]
    rejects: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct VariantFile {
    name: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct DependencyExclude {
    group: String,
    module: String,
}

#[derive(Debug, Deserialize)]
struct AvailableAt {
    #[allow(dead_code)]
    url: String,
    #[allow(dead_code)]
    group: String,
    #[allow(dead_code)]
    module: String,
    #[allow(dead_code)]
    version: String,
}

#[derive(Debug, Deserialize)]
struct Capability {
    group: String,
    name: String,
    version: String,
}

pub fn select_jvm_variant(
    json: &str,
    target_scope: &str,
    root_group: &str,
    root_module: &str,
    root_version: &str,
) -> Result<Option<ModuleMetadataSelection>, String> {
    let metadata = serde_json::from_str::<GradleModuleMetadata>(json)
        .map_err(|e| format!("Failed to parse Gradle Module Metadata: {e}"))?;
    if metadata.variants.is_empty() {
        return Ok(None);
    }

    let component_group = if metadata.component.group.is_empty() {
        root_group
    } else {
        &metadata.component.group
    };
    let component_module = if metadata.component.module.is_empty() {
        root_module
    } else {
        &metadata.component.module
    };
    let component_version = if metadata.component.version.is_empty() {
        root_version
    } else {
        &metadata.component.version
    };

    let usage = if matches!(
        target_scope,
        "runtime" | "runtimeClasspath" | "implementation"
    ) {
        "java-runtime"
    } else {
        "java-api"
    };

    let usage_candidates: Vec<&Variant> = metadata
        .variants
        .iter()
        .filter(|variant| variant_usage(variant).as_deref() == Some(usage))
        .collect();

    let candidates = preferred_jvm_candidates(&usage_candidates);

    let candidates = if candidates.is_empty() && usage == "java-api" {
        metadata
            .variants
            .iter()
            .filter(|variant| variant.name == "apiElements")
            .collect()
    } else if candidates.is_empty() && usage == "java-runtime" {
        metadata
            .variants
            .iter()
            .filter(|variant| variant.name == "runtimeElements")
            .collect()
    } else {
        candidates
    };

    if candidates.is_empty() {
        return Ok(Some(ModuleMetadataSelection::Unsupported(format!(
            "unsupported Gradle Module Metadata: no JVM variant for usage {usage}"
        ))));
    }
    if candidates.len() > 1 {
        return Ok(Some(ModuleMetadataSelection::Unsupported(format!(
            "unsupported Gradle Module Metadata: ambiguous JVM variants for usage {usage}"
        ))));
    }

    let variant = candidates[0];
    if variant.available_at.is_some() {
        return Ok(Some(ModuleMetadataSelection::Unsupported(format!(
            "unsupported Gradle Module Metadata available-at redirect on variant {}",
            variant.name
        ))));
    }
    let dependency_constraints = match static_dependency_constraints(variant) {
        Ok(constraints) => constraints,
        Err(selection) => return Ok(Some(selection)),
    };
    for capability in &variant.capabilities {
        if capability.group != component_group
            || capability.name != component_module
            || capability.version != component_version
        {
            return Ok(Some(ModuleMetadataSelection::Unsupported(format!(
                "unsupported Gradle Module Metadata capability {}:{}:{} on variant {}",
                capability.group, capability.name, capability.version, variant.name
            ))));
        }
    }

    let mut dependencies = Vec::with_capacity(variant.dependencies.len());
    for dep in &variant.dependencies {
        let dependency_version =
            match static_version_requirement(&dep.version, &dep.group, &dep.module, "dependency") {
                Ok(version) => version,
                Err(selection) => return Ok(Some(selection)),
            };
        let required_version = match apply_dependency_constraint(
            &dep.group,
            &dep.module,
            &dependency_version,
            &dependency_constraints,
        ) {
            Ok(version) => version,
            Err(selection) => return Ok(Some(selection)),
        };
        let exclusions = match static_dependency_exclusions(dep) {
            Ok(exclusions) => exclusions,
            Err(selection) => return Ok(Some(selection)),
        };
        dependencies.push(ModuleDependency {
            group: dep.group.clone(),
            module: dep.module.clone(),
            version: required_version,
            exclusions,
        });
    }

    let artifacts = variant
        .files
        .iter()
        .map(|file| ModuleArtifact {
            name: file.name.clone(),
            url: file.url.clone(),
        })
        .collect();

    Ok(Some(ModuleMetadataSelection::Selected(SelectedVariant {
        name: variant.name.clone(),
        dependencies,
        artifacts,
    })))
}

fn variant_usage(variant: &Variant) -> Option<String> {
    variant
        .attributes
        .get("org.gradle.usage")
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

fn preferred_jvm_candidates<'a>(candidates: &[&'a Variant]) -> Vec<&'a Variant> {
    let jar_candidates: Vec<&Variant> = candidates
        .iter()
        .copied()
        .filter(|variant| {
            variant_attribute(variant, "org.gradle.libraryelements").as_deref() == Some("jar")
        })
        .collect();
    let candidates = if jar_candidates.is_empty() {
        candidates.to_vec()
    } else {
        jar_candidates
    };

    let standard_jvm_candidates: Vec<&Variant> = candidates
        .iter()
        .copied()
        .filter(|variant| {
            variant_attribute(variant, "org.gradle.jvm.environment")
                .as_deref()
                .map(|value| value == "standard-jvm")
                .unwrap_or(true)
        })
        .collect();
    if standard_jvm_candidates.is_empty() {
        candidates
    } else {
        standard_jvm_candidates
    }
}

fn variant_attribute(variant: &Variant, name: &str) -> Option<String> {
    variant
        .attributes
        .get(name)
        .and_then(|value| value.as_str())
        .map(str::to_string)
}

fn static_dependency_constraints(
    variant: &Variant,
) -> Result<BTreeMap<(String, String), StaticVersionRequirement>, ModuleMetadataSelection> {
    let mut constraints: BTreeMap<(String, String), StaticVersionRequirement> = BTreeMap::new();
    for constraint in &variant.dependency_constraints {
        if !constraint.excludes.is_empty() {
            return Err(ModuleMetadataSelection::Unsupported(format!(
                "unsupported Gradle Module Metadata dependency constraint exclusions for {}:{}",
                constraint.group, constraint.module
            )));
        }
        let version = static_version_requirement(
            &constraint.version,
            &constraint.group,
            &constraint.module,
            "dependency constraint",
        )?;
        let key = (constraint.group.clone(), constraint.module.clone());
        match constraints.get(&key) {
            Some(existing)
                if compare_versions(&version.version, &existing.version)
                    != std::cmp::Ordering::Greater => {}
            _ => {
                constraints.insert(key, version);
            }
        }
    }
    Ok(constraints)
}

fn static_version_requirement(
    version: &VersionRequirement,
    group: &str,
    module: &str,
    role: &str,
) -> Result<StaticVersionRequirement, ModuleMetadataSelection> {
    let selected = if !version.strictly.trim().is_empty() {
        version.strictly.trim()
    } else if !version.requires.trim().is_empty() {
        version.requires.trim()
    } else if !version.prefers.trim().is_empty() {
        version.prefers.trim()
    } else {
        return Err(ModuleMetadataSelection::Unsupported(format!(
            "unsupported Gradle Module Metadata {role} without required version for {group}:{module}"
        )));
    };
    if let Some(reason) = super::graph_builder::unsupported_version_selector_reason(selected) {
        return Err(ModuleMetadataSelection::Unsupported(format!(
            "unsupported Gradle Module Metadata {role} version for {group}:{module}: {reason}"
        )));
    }
    let mut rejects = Vec::with_capacity(version.rejects.len());
    for rejected in &version.rejects {
        let rejected = rejected.trim();
        if rejected.is_empty() {
            return Err(ModuleMetadataSelection::Unsupported(format!(
                "unsupported Gradle Module Metadata empty rejected version for {role} {group}:{module}"
            )));
        }
        if let Some(reason) = super::graph_builder::unsupported_version_selector_reason(rejected) {
            return Err(ModuleMetadataSelection::Unsupported(format!(
                "unsupported Gradle Module Metadata rejected version for {role} {group}:{module}: {reason}"
            )));
        }
        rejects.push(rejected.to_string());
    }
    if rejects.iter().any(|rejected| rejected == selected) {
        return Err(ModuleMetadataSelection::Unsupported(format!(
            "unsupported Gradle Module Metadata {role} for {group}:{module}: selected version {selected} is rejected"
        )));
    }
    Ok(StaticVersionRequirement {
        version: selected.to_string(),
        rejects,
    })
}

fn apply_dependency_constraint(
    group: &str,
    module: &str,
    required_version: &StaticVersionRequirement,
    constraints: &BTreeMap<(String, String), StaticVersionRequirement>,
) -> Result<String, ModuleMetadataSelection> {
    let Some(constrained_version) = constraints.get(&(group.to_string(), module.to_string()))
    else {
        return Ok(required_version.version.clone());
    };
    let selected = if compare_versions(&constrained_version.version, &required_version.version)
        == std::cmp::Ordering::Greater
    {
        constrained_version.version.clone()
    } else {
        required_version.version.clone()
    };
    if required_version
        .rejects
        .iter()
        .chain(constrained_version.rejects.iter())
        .any(|rejected| rejected == &selected)
    {
        return Err(ModuleMetadataSelection::Unsupported(format!(
            "unsupported Gradle Module Metadata dependency for {group}:{module}: selected version {selected} is rejected"
        )));
    }
    Ok(selected)
}

fn static_dependency_exclusions(
    dependency: &VariantDependency,
) -> Result<Vec<(String, String)>, ModuleMetadataSelection> {
    let mut exclusions = Vec::with_capacity(dependency.excludes.len());
    for exclusion in &dependency.excludes {
        if exclusion.group.trim().is_empty() || exclusion.module.trim().is_empty() {
            return Err(ModuleMetadataSelection::Unsupported(format!(
                "unsupported Gradle Module Metadata malformed dependency exclusion for {}:{}",
                dependency.group, dependency.module
            )));
        }
        exclusions.push((exclusion.group.clone(), exclusion.module.clone()));
    }
    Ok(exclusions)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_runtime_variant_dependencies() {
        let json = r#"{
          "formatVersion": "1.1",
          "component": {"group":"org.example","module":"root","version":"1.0"},
          "variants": [
            {"name":"apiElements","attributes":{"org.gradle.usage":"java-api"},"dependencies":[]},
            {"name":"runtimeElements","attributes":{"org.gradle.usage":"java-runtime"},
             "dependencies":[{"group":"org.example","module":"runtime","version":{"requires":"2.0"}}],
             "files":[{"name":"root-1.0.jar","url":"root-1.0.jar"}]}
          ]
        }"#;

        let selected = select_jvm_variant(json, "runtime", "org.example", "root", "1.0")
            .unwrap()
            .unwrap();

        match selected {
            ModuleMetadataSelection::Selected(variant) => {
                assert_eq!(variant.name, "runtimeElements");
                assert_eq!(variant.dependencies[0].module, "runtime");
                assert_eq!(variant.artifacts[0].url, "root-1.0.jar");
            }
            ModuleMetadataSelection::Unsupported(reason) => panic!("{reason}"),
        }
    }

    #[test]
    fn rejects_custom_capability() {
        let json = r#"{
          "component": {"group":"org.example","module":"root","version":"1.0"},
          "variants": [
            {"name":"runtimeElements","attributes":{"org.gradle.usage":"java-runtime"},
             "capabilities":[{"group":"org.example","name":"feature","version":"1.0"}]}
          ]
        }"#;

        let selected = select_jvm_variant(json, "runtime", "org.example", "root", "1.0")
            .unwrap()
            .unwrap();

        assert!(matches!(
            selected,
            ModuleMetadataSelection::Unsupported(reason) if reason.contains("capability")
        ));
    }

    #[test]
    fn rejects_available_at_redirect() {
        let json = r#"{
          "component": {"group":"org.example","module":"root","version":"1.0"},
          "variants": [
            {"name":"runtimeElements","attributes":{"org.gradle.usage":"java-runtime"},
             "available-at":{"url":"root-1.0.module","group":"org.example","module":"root-jvm","version":"1.0"}}
          ]
        }"#;

        let selected = select_jvm_variant(json, "runtime", "org.example", "root", "1.0")
            .unwrap()
            .unwrap();

        assert!(matches!(
            selected,
            ModuleMetadataSelection::Unsupported(reason) if reason.contains("available-at")
        ));
    }

    #[test]
    fn applies_static_dependency_constraints_to_matching_dependencies() {
        let json = r#"{
          "component": {"group":"org.example","module":"root","version":"1.0"},
          "variants": [
            {"name":"runtimeElements","attributes":{"org.gradle.usage":"java-runtime"},
             "dependencies":[
               {"group":"org.example","module":"child","version":{"requires":"1.0"}},
               {"group":"org.example","module":"other","version":{"requires":"1.0"}}
             ],
             "dependencyConstraints":[{"group":"org.example","module":"child","version":{"requires":"2.0"}}]}
          ]
        }"#;

        let selected = select_jvm_variant(json, "runtime", "org.example", "root", "1.0")
            .unwrap()
            .unwrap();

        match selected {
            ModuleMetadataSelection::Selected(variant) => {
                assert_eq!(variant.dependencies.len(), 2);
                assert_eq!(variant.dependencies[0].module, "child");
                assert_eq!(variant.dependencies[0].version, "2.0");
                assert_eq!(variant.dependencies[1].module, "other");
                assert_eq!(variant.dependencies[1].version, "1.0");
            }
            ModuleMetadataSelection::Unsupported(reason) => panic!("{reason}"),
        }
    }

    #[test]
    fn dependency_constraints_do_not_create_dependencies() {
        let json = r#"{
          "component": {"group":"org.example","module":"root","version":"1.0"},
          "variants": [
            {"name":"runtimeElements","attributes":{"org.gradle.usage":"java-runtime"},
             "dependencies":[],
             "dependencyConstraints":[{"group":"org.example","module":"constrained","version":{"requires":"2.0"}}]}
          ]
        }"#;

        let selected = select_jvm_variant(json, "runtime", "org.example", "root", "1.0")
            .unwrap()
            .unwrap();

        match selected {
            ModuleMetadataSelection::Selected(variant) => {
                assert!(variant.dependencies.is_empty());
            }
            ModuleMetadataSelection::Unsupported(reason) => panic!("{reason}"),
        }
    }

    #[test]
    fn applies_static_strict_dependency_constraints() {
        let json = r#"{
          "component": {"group":"org.example","module":"root","version":"1.0"},
          "variants": [
            {"name":"runtimeElements","attributes":{"org.gradle.usage":"java-runtime"},
             "dependencies":[{"group":"org.example","module":"child","version":{"requires":"1.0"}}],
             "dependencyConstraints":[{"group":"org.example","module":"child","version":{"strictly":"2.0"}}]}
          ]
        }"#;

        let selected = select_jvm_variant(json, "runtime", "org.example", "root", "1.0")
            .unwrap()
            .unwrap();

        match selected {
            ModuleMetadataSelection::Selected(variant) => {
                assert_eq!(variant.dependencies[0].version, "2.0");
            }
            ModuleMetadataSelection::Unsupported(reason) => panic!("{reason}"),
        }
    }

    #[test]
    fn applies_static_preferred_dependency_version_without_requires() {
        let json = r#"{
          "component": {"group":"org.example","module":"root","version":"1.0"},
          "variants": [
            {"name":"runtimeElements","attributes":{"org.gradle.usage":"java-runtime"},
             "dependencies":[{"group":"org.example","module":"child","version":{"prefers":"2.0"}}]}
          ]
        }"#;

        let selected = select_jvm_variant(json, "runtime", "org.example", "root", "1.0")
            .unwrap()
            .unwrap();

        match selected {
            ModuleMetadataSelection::Selected(variant) => {
                assert_eq!(variant.dependencies[0].version, "2.0");
            }
            ModuleMetadataSelection::Unsupported(reason) => panic!("{reason}"),
        }
    }

    #[test]
    fn rejects_selected_rejected_dependency_version() {
        let json = r#"{
          "component": {"group":"org.example","module":"root","version":"1.0"},
          "variants": [
            {"name":"runtimeElements","attributes":{"org.gradle.usage":"java-runtime"},
             "dependencies":[{"group":"org.example","module":"child","version":{"requires":"2.0","rejects":["2.0"]}}]}
          ]
        }"#;

        let selected = select_jvm_variant(json, "runtime", "org.example", "root", "1.0")
            .unwrap()
            .unwrap();

        assert!(matches!(
            selected,
            ModuleMetadataSelection::Unsupported(reason) if reason.contains("selected version 2.0 is rejected")
        ));
    }

    #[test]
    fn rejects_dynamic_rejected_dependency_version() {
        let json = r#"{
          "component": {"group":"org.example","module":"root","version":"1.0"},
          "variants": [
            {"name":"runtimeElements","attributes":{"org.gradle.usage":"java-runtime"},
             "dependencies":[{"group":"org.example","module":"child","version":{"requires":"2.0","rejects":["2.+"]}}]}
          ]
        }"#;

        let selected = select_jvm_variant(json, "runtime", "org.example", "root", "1.0")
            .unwrap()
            .unwrap();

        assert!(matches!(
            selected,
            ModuleMetadataSelection::Unsupported(reason) if reason.contains("rejected version")
        ));
    }

    #[test]
    fn rejects_dependency_constraints_without_required_version() {
        let json = r#"{
          "component": {"group":"org.example","module":"root","version":"1.0"},
          "variants": [
            {"name":"runtimeElements","attributes":{"org.gradle.usage":"java-runtime"},
             "dependencies":[{"group":"org.example","module":"child","version":{"requires":"1.0"}}],
             "dependencyConstraints":[{"group":"org.example","module":"child","version":{}}]}
          ]
        }"#;

        let selected = select_jvm_variant(json, "runtime", "org.example", "root", "1.0")
            .unwrap()
            .unwrap();

        assert!(matches!(
            selected,
            ModuleMetadataSelection::Unsupported(reason) if reason.contains("constraint without required version")
        ));
    }

    #[test]
    fn rejects_dependency_constraint_exclusions() {
        let json = r#"{
          "component": {"group":"org.example","module":"root","version":"1.0"},
          "variants": [
            {"name":"runtimeElements","attributes":{"org.gradle.usage":"java-runtime"},
             "dependencies":[{"group":"org.example","module":"child","version":{"requires":"1.0"}}],
             "dependencyConstraints":[{"group":"org.example","module":"child","version":{"requires":"2.0"},
               "excludes":[{"group":"org.bad","module":"bad"}]}]}
          ]
        }"#;

        let selected = select_jvm_variant(json, "runtime", "org.example", "root", "1.0")
            .unwrap()
            .unwrap();

        assert!(matches!(
            selected,
            ModuleMetadataSelection::Unsupported(reason) if reason.contains("constraint exclusions")
        ));
    }

    #[test]
    fn captures_static_dependency_exclusions() {
        let json = r#"{
          "component": {"group":"org.example","module":"root","version":"1.0"},
          "variants": [
            {"name":"runtimeElements","attributes":{"org.gradle.usage":"java-runtime"},
             "dependencies":[{"group":"org.example","module":"child","version":{"requires":"2.0"},
               "excludes":[{"group":"org.bad","module":"bad"}]}]}
          ]
        }"#;

        let selected = select_jvm_variant(json, "runtime", "org.example", "root", "1.0")
            .unwrap()
            .unwrap();

        match selected {
            ModuleMetadataSelection::Selected(variant) => {
                assert_eq!(
                    variant.dependencies[0].exclusions,
                    vec![("org.bad".to_string(), "bad".to_string())]
                );
            }
            ModuleMetadataSelection::Unsupported(reason) => panic!("{reason}"),
        }
    }

    #[test]
    fn rejects_malformed_dependency_exclusions() {
        let json = r#"{
          "component": {"group":"org.example","module":"root","version":"1.0"},
          "variants": [
            {"name":"runtimeElements","attributes":{"org.gradle.usage":"java-runtime"},
             "dependencies":[{"group":"org.example","module":"child","version":{"requires":"2.0"},
               "excludes":[{"group":"","module":"bad"}]}]}
          ]
        }"#;

        let selected = select_jvm_variant(json, "runtime", "org.example", "root", "1.0")
            .unwrap()
            .unwrap();

        assert!(matches!(
            selected,
            ModuleMetadataSelection::Unsupported(reason) if reason.contains("malformed dependency exclusion")
        ));
    }

    #[test]
    fn prefers_jar_runtime_variant_over_sources_variant() {
        let json = r#"{
          "component": {"group":"com.squareup.okio","module":"okio","version":"3.6.0"},
          "variants": [
            {"name":"jvmRuntimeElements-published","attributes":{"org.gradle.usage":"java-runtime","org.gradle.libraryelements":"jar","org.gradle.jvm.environment":"standard-jvm"}},
            {"name":"jvmSourcesElements-published","attributes":{"org.gradle.usage":"java-runtime","org.gradle.libraryelements":"sources","org.gradle.jvm.environment":"standard-jvm"}}
          ]
        }"#;

        let selected = select_jvm_variant(json, "runtime", "com.squareup.okio", "okio", "3.6.0")
            .unwrap()
            .unwrap();

        match selected {
            ModuleMetadataSelection::Selected(variant) => {
                assert_eq!(variant.name, "jvmRuntimeElements-published");
            }
            ModuleMetadataSelection::Unsupported(reason) => panic!("{reason}"),
        }
    }

    #[test]
    fn prefers_standard_jvm_runtime_variant_over_android_variant() {
        let json = r#"{
          "component": {"group":"com.google.guava","module":"guava","version":"33.2.1-jre"},
          "variants": [
            {"name":"jreRuntimeElements","attributes":{"org.gradle.usage":"java-runtime","org.gradle.libraryelements":"jar","org.gradle.jvm.environment":"standard-jvm"}},
            {"name":"androidRuntimeElements","attributes":{"org.gradle.usage":"java-runtime","org.gradle.libraryelements":"jar","org.gradle.jvm.environment":"android"}}
          ]
        }"#;

        let selected =
            select_jvm_variant(json, "runtime", "com.google.guava", "guava", "33.2.1-jre")
                .unwrap()
                .unwrap();

        match selected {
            ModuleMetadataSelection::Selected(variant) => {
                assert_eq!(variant.name, "jreRuntimeElements");
            }
            ModuleMetadataSelection::Unsupported(reason) => panic!("{reason}"),
        }
    }
}
