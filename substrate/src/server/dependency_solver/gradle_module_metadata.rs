use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleDependency {
    pub group: String,
    pub module: String,
    pub version: String,
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
    #[serde(default)]
    files: Vec<VariantFile>,
    #[serde(default)]
    capabilities: Vec<Capability>,
}

#[derive(Debug, Deserialize)]
struct VariantDependency {
    group: String,
    module: String,
    #[serde(default)]
    version: VersionRequirement,
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

    let candidates: Vec<&Variant> = metadata
        .variants
        .iter()
        .filter(|variant| variant_usage(variant).as_deref() == Some(usage))
        .collect();

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
        if !dep.version.strictly.is_empty()
            || !dep.version.prefers.is_empty()
            || !dep.version.rejects.is_empty()
        {
            return Ok(Some(ModuleMetadataSelection::Unsupported(format!(
                "unsupported Gradle Module Metadata rich version for {}:{}",
                dep.group, dep.module
            ))));
        }
        if dep.version.requires.trim().is_empty() {
            return Ok(Some(ModuleMetadataSelection::Unsupported(format!(
                "unsupported Gradle Module Metadata dependency without required version for {}:{}",
                dep.group, dep.module
            ))));
        }
        dependencies.push(ModuleDependency {
            group: dep.group.clone(),
            module: dep.module.clone(),
            version: dep.version.requires.clone(),
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
}
