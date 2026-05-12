use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const CONTRACT_SCHEMA: &str = "gradle-substrate.cyclonedx-sbom.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycloneDxSbomContract {
    pub schema: String,
    pub spec_version: String,
    pub serial_number: String,
    pub timestamp: String,
    pub root_component: CycloneDxComponent,
    #[serde(default)]
    pub components: Vec<CycloneDxComponent>,
    #[serde(default)]
    pub dependencies: Vec<CycloneDxDependency>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycloneDxComponent {
    #[serde(rename = "type")]
    pub component_type: String,
    #[serde(rename = "bom-ref")]
    pub bom_ref: String,
    pub group: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub purl: String,
    #[serde(default)]
    pub properties: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycloneDxDependency {
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(default, rename = "dependsOn")]
    pub depends_on: Vec<String>,
}

impl CycloneDxSbomContract {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != CONTRACT_SCHEMA {
            return Err(format!(
                "unsupported CycloneDX contract schema '{}' (expected '{}')",
                self.schema, CONTRACT_SCHEMA
            ));
        }
        if self.spec_version.trim().is_empty()
            || self.serial_number.trim().is_empty()
            || self.timestamp.trim().is_empty()
        {
            return Err(
                "CycloneDX contract is missing spec_version, serial_number, or timestamp"
                    .to_string(),
            );
        }
        self.root_component.validate("root component")?;
        for component in &self.components {
            component.validate("component")?;
        }
        for dependency in &self.dependencies {
            if dependency.reference.trim().is_empty() {
                return Err("CycloneDX dependency is missing ref".to_string());
            }
        }
        Ok(())
    }
}

impl CycloneDxComponent {
    fn validate(&self, label: &str) -> Result<(), String> {
        if self.component_type.trim().is_empty()
            || self.bom_ref.trim().is_empty()
            || self.name.trim().is_empty()
            || self.version.trim().is_empty()
        {
            return Err(format!(
                "CycloneDX {label} is missing type, bom-ref, name, or version"
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CycloneDxBom<'a> {
    #[serde(rename = "bomFormat")]
    bom_format: &'static str,
    #[serde(rename = "specVersion")]
    spec_version: &'a str,
    #[serde(rename = "serialNumber")]
    serial_number: &'a str,
    version: u32,
    metadata: CycloneDxMetadata<'a>,
    components: Vec<CycloneDxComponent>,
    dependencies: Vec<CycloneDxDependency>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CycloneDxMetadata<'a> {
    timestamp: &'a str,
    component: &'a CycloneDxComponent,
}

pub fn render_json(contract: &CycloneDxSbomContract) -> Result<String, String> {
    contract.validate()?;
    let bom = normalized_bom(contract);
    serde_json::to_string_pretty(&bom).map_err(|err| err.to_string())
}

pub fn render_xml(contract: &CycloneDxSbomContract) -> Result<String, String> {
    contract.validate()?;
    let bom = normalized_bom(contract);
    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str(&format!(
        "<bom xmlns=\"http://cyclonedx.org/schema/bom/{}\" serialNumber=\"{}\" version=\"{}\">\n",
        xml_escape(bom.spec_version),
        xml_escape(bom.serial_number),
        bom.version
    ));
    xml.push_str("  <metadata>\n");
    xml.push_str(&format!(
        "    <timestamp>{}</timestamp>\n",
        xml_escape(bom.metadata.timestamp)
    ));
    write_component(&mut xml, "    ", bom.metadata.component);
    xml.push_str("  </metadata>\n");
    xml.push_str("  <components>\n");
    for component in &bom.components {
        write_component(&mut xml, "    ", component);
    }
    xml.push_str("  </components>\n");
    xml.push_str("  <dependencies>\n");
    for dependency in &bom.dependencies {
        xml.push_str(&format!(
            "    <dependency ref=\"{}\">\n",
            xml_escape(&dependency.reference)
        ));
        for child in &dependency.depends_on {
            xml.push_str(&format!(
                "      <dependency ref=\"{}\"/>\n",
                xml_escape(child)
            ));
        }
        xml.push_str("    </dependency>\n");
    }
    xml.push_str("  </dependencies>\n");
    xml.push_str("</bom>\n");
    Ok(xml)
}

fn normalized_bom(contract: &CycloneDxSbomContract) -> CycloneDxBom<'_> {
    let mut components = contract.components.clone();
    components.sort_by(|a, b| {
        (&a.bom_ref, &a.group, &a.name, &a.version)
            .cmp(&(&b.bom_ref, &b.group, &b.name, &b.version))
    });
    let mut dependencies = contract.dependencies.clone();
    for dependency in &mut dependencies {
        dependency.depends_on.sort();
    }
    dependencies.sort_by(|a, b| a.reference.cmp(&b.reference));
    CycloneDxBom {
        bom_format: "CycloneDX",
        spec_version: &contract.spec_version,
        serial_number: &contract.serial_number,
        version: 1,
        metadata: CycloneDxMetadata {
            timestamp: &contract.timestamp,
            component: &contract.root_component,
        },
        components,
        dependencies,
    }
}

fn write_component(xml: &mut String, indent: &str, component: &CycloneDxComponent) {
    xml.push_str(&format!(
        "{indent}<component type=\"{}\" bom-ref=\"{}\">\n",
        xml_escape(&component.component_type),
        xml_escape(&component.bom_ref)
    ));
    if !component.group.is_empty() {
        xml.push_str(&format!(
            "{indent}  <group>{}</group>\n",
            xml_escape(&component.group)
        ));
    }
    xml.push_str(&format!(
        "{indent}  <name>{}</name>\n",
        xml_escape(&component.name)
    ));
    xml.push_str(&format!(
        "{indent}  <version>{}</version>\n",
        xml_escape(&component.version)
    ));
    if !component.purl.is_empty() {
        xml.push_str(&format!(
            "{indent}  <purl>{}</purl>\n",
            xml_escape(&component.purl)
        ));
    }
    if !component.properties.is_empty() {
        xml.push_str(&format!("{indent}  <properties>\n"));
        for (name, value) in &component.properties {
            xml.push_str(&format!(
                "{indent}    <property name=\"{}\">{}</property>\n",
                xml_escape(name),
                xml_escape(value)
            ));
        }
        xml.push_str(&format!("{indent}  </properties>\n"));
    }
    xml.push_str(&format!("{indent}</component>\n"));
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_contract() -> CycloneDxSbomContract {
        CycloneDxSbomContract {
            schema: CONTRACT_SCHEMA.to_string(),
            spec_version: "1.6".to_string(),
            serial_number: "urn:uuid:00000000-0000-0000-0000-000000000001".to_string(),
            timestamp: "2026-05-12T10:00:00Z".to_string(),
            root_component: CycloneDxComponent {
                component_type: "application".to_string(),
                bom_ref: "pkg:maven/org.example/app@1.0.0?project_path=%3A".to_string(),
                group: "org.example".to_string(),
                name: "app".to_string(),
                version: "1.0.0".to_string(),
                purl: "pkg:maven/org.example/app@1.0.0".to_string(),
                properties: BTreeMap::new(),
            },
            components: vec![
                CycloneDxComponent {
                    component_type: "library".to_string(),
                    bom_ref: "pkg:maven/org.example/b@1.0.0?type=jar".to_string(),
                    group: "org.example".to_string(),
                    name: "b".to_string(),
                    version: "1.0.0".to_string(),
                    purl: "pkg:maven/org.example/b@1.0.0?type=jar".to_string(),
                    properties: BTreeMap::new(),
                },
                CycloneDxComponent {
                    component_type: "library".to_string(),
                    bom_ref: "pkg:maven/org.example/a@1.0.0?type=jar".to_string(),
                    group: "org.example".to_string(),
                    name: "a".to_string(),
                    version: "1.0.0".to_string(),
                    purl: "pkg:maven/org.example/a@1.0.0?type=jar".to_string(),
                    properties: BTreeMap::new(),
                },
            ],
            dependencies: vec![CycloneDxDependency {
                reference: "pkg:maven/org.example/app@1.0.0?project_path=%3A".to_string(),
                depends_on: vec![
                    "pkg:maven/org.example/b@1.0.0?type=jar".to_string(),
                    "pkg:maven/org.example/a@1.0.0?type=jar".to_string(),
                ],
            }],
        }
    }

    #[test]
    fn renders_deterministic_json_from_explicit_contract() {
        let json = render_json(&sample_contract()).unwrap();
        assert!(json.contains("\"bomFormat\": \"CycloneDX\""));
        assert!(json.contains("\"specVersion\": \"1.6\""));
        assert!(json.find("org.example/a").unwrap() < json.find("org.example/b").unwrap());
        assert!(json.contains("\"dependsOn\""));
    }

    #[test]
    fn renders_deterministic_xml_from_explicit_contract() {
        let xml = render_xml(&sample_contract()).unwrap();
        assert!(xml.contains("http://cyclonedx.org/schema/bom/1.6"));
        assert!(xml.contains("<metadata>"));
        assert!(xml.find("org.example/a").unwrap() < xml.find("org.example/b").unwrap());
        assert!(xml.contains("<dependencies>"));
    }

    #[test]
    fn rejects_missing_required_contract_fields() {
        let mut contract = sample_contract();
        contract.root_component.bom_ref.clear();
        let err = render_json(&contract).unwrap_err();
        assert!(err.contains("root component"));
    }
}
