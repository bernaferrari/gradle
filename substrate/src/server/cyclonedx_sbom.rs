use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::Read;
use std::path::Path;

use quick_xml::events::Event;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const CONTRACT_SCHEMA: &str = "gradle-substrate.cyclonedx-sbom.v1";
pub const RESOLUTION_GRAPH_SCHEMA: &str = "gradle-substrate.cyclonedx-resolution-graph.v1";

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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub licenses: Vec<CycloneDxLicenseChoice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycloneDxLicenseChoice {
    pub license: CycloneDxLicense,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycloneDxLicense {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycloneDxDependency {
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(default, rename = "dependsOn")]
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycloneDxResolutionGraphEvidence {
    pub schema: String,
    #[serde(default)]
    pub configurations: Vec<CycloneDxResolutionConfiguration>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycloneDxResolutionConfiguration {
    pub name: String,
    #[serde(default)]
    pub components: Vec<CycloneDxResolvedComponent>,
    #[serde(default)]
    pub dependencies: Vec<CycloneDxResolvedDependency>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycloneDxResolvedComponent {
    pub id: String,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub module: String,
    #[serde(default)]
    pub version: String,
    #[serde(default, rename = "projectPath")]
    pub project_path: String,
    #[serde(default, rename = "artifactPath")]
    pub artifact_path: String,
    #[serde(default, rename = "artifactType")]
    pub artifact_type: String,
    #[serde(default, rename = "artifactExtension")]
    pub artifact_extension: String,
    #[serde(default, rename = "artifactClassifier")]
    pub artifact_classifier: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycloneDxResolvedDependency {
    pub from: String,
    #[serde(default)]
    pub requested: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CycloneDxDraftOptions {
    pub spec_version: String,
    pub serial_number: String,
    pub timestamp: String,
    pub root_group: String,
    pub root_name: String,
    pub root_version: String,
    pub root_component_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CycloneDxCapturedTaskOptions {
    pub spec_version: String,
    pub root_group: String,
    pub root_name: String,
    pub root_version: String,
    pub root_component_type: String,
    pub include_bom_serial_number: bool,
    pub json_output: String,
    pub xml_output: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CycloneDxAggregateOptions {
    pub spec_version: String,
    pub serial_number: String,
    pub timestamp: String,
    pub root_group: String,
    pub root_name: String,
    pub root_version: String,
    pub root_component_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CycloneDxIdentityPolicy {
    pub build_id: String,
    pub task_path: String,
    pub root_group: String,
    pub root_name: String,
    pub root_version: String,
    pub schema_version: String,
    pub timestamp_ms: i64,
}

impl CycloneDxIdentityPolicy {
    pub fn serial_number(&self) -> String {
        deterministic_serial_number(&[
            &self.build_id,
            &self.task_path,
            &self.root_group,
            &self.root_name,
            &self.root_version,
            &self.schema_version,
        ])
    }

    pub fn timestamp(&self) -> Result<String, String> {
        timestamp_from_epoch_millis(self.timestamp_ms)
    }
}

pub fn validate_captured_task_options(
    inputs: &BTreeMap<String, String>,
) -> Result<CycloneDxCapturedTaskOptions, String> {
    let spec_version = normalize_schema_version(value(inputs, "cyclonedx_schema_version"));
    let root_name = value(inputs, "cyclonedx_component_name").to_string();
    let root_version = value(inputs, "cyclonedx_component_version").to_string();
    let root_component_type = value(inputs, "cyclonedx_project_type").to_lowercase();
    let json_output = value(inputs, "cyclonedx_json_output").to_string();
    let xml_output = value(inputs, "cyclonedx_xml_output").to_string();

    let mut missing = Vec::new();
    if spec_version.is_empty() {
        missing.push("schema-version");
    }
    if root_name.trim().is_empty() {
        missing.push("component-name");
    }
    if root_version.trim().is_empty() {
        missing.push("component-version");
    }
    if root_component_type.trim().is_empty() {
        missing.push("project-type");
    }
    if json_output.trim().is_empty() && xml_output.trim().is_empty() {
        missing.push("json-or-xml-output");
    }
    if !missing.is_empty() {
        return Err(format!(
            "CycloneDX captured task options are missing {}",
            missing.join(",")
        ));
    }

    Ok(CycloneDxCapturedTaskOptions {
        spec_version,
        root_group: value(inputs, "cyclonedx_component_group").to_string(),
        root_name,
        root_version,
        root_component_type,
        include_bom_serial_number: value(inputs, "cyclonedx_include_bom_serial_number")
            .eq_ignore_ascii_case("true"),
        json_output,
        xml_output,
    })
}

pub fn aggregate_contracts(
    contracts: &[CycloneDxSbomContract],
    options: CycloneDxAggregateOptions,
) -> Result<CycloneDxSbomContract, String> {
    if contracts.is_empty() {
        return Err("CycloneDX aggregate contract has no input SBOM contracts".to_string());
    }
    validate_aggregate_options(&options)?;
    let root_purl = purl(
        &options.root_group,
        &options.root_name,
        &options.root_version,
    );
    let root_component = CycloneDxComponent {
        component_type: options.root_component_type.to_lowercase(),
        bom_ref: root_purl.clone(),
        group: options.root_group,
        name: options.root_name,
        version: options.root_version,
        purl: root_purl,
        properties: BTreeMap::new(),
        licenses: Vec::new(),
    };

    let mut components_by_ref = BTreeMap::<String, CycloneDxComponent>::new();
    let mut dependencies_by_ref = BTreeMap::<String, BTreeSet<String>>::new();
    let mut aggregate_root_children = BTreeSet::new();
    for contract in contracts {
        contract.validate()?;
        merge_component(&mut components_by_ref, contract.root_component.clone())?;
        aggregate_root_children.insert(contract.root_component.bom_ref.clone());
        for component in &contract.components {
            merge_component(&mut components_by_ref, component.clone())?;
        }
        for dependency in &contract.dependencies {
            dependencies_by_ref
                .entry(dependency.reference.clone())
                .or_default()
                .extend(dependency.depends_on.iter().cloned());
        }
    }
    dependencies_by_ref
        .entry(root_component.bom_ref.clone())
        .or_default()
        .extend(aggregate_root_children);

    let mut components = components_by_ref.into_values().collect::<Vec<_>>();
    components.sort_by(|a, b| a.bom_ref.cmp(&b.bom_ref));
    let dependencies = dependencies_by_ref
        .into_iter()
        .map(|(reference, depends_on)| CycloneDxDependency {
            reference,
            depends_on: depends_on.into_iter().collect(),
        })
        .collect::<Vec<_>>();

    let contract = CycloneDxSbomContract {
        schema: CONTRACT_SCHEMA.to_string(),
        spec_version: options.spec_version,
        serial_number: options.serial_number,
        timestamp: options.timestamp,
        root_component,
        components,
        dependencies,
    };
    contract.validate()?;
    Ok(contract)
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
        for license in &self.licenses {
            if license.license.name.trim().is_empty() {
                return Err(format!("CycloneDX {label} has an empty license name"));
            }
        }
        Ok(())
    }
}

impl CycloneDxResolutionGraphEvidence {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != RESOLUTION_GRAPH_SCHEMA {
            return Err(format!(
                "unsupported CycloneDX resolution graph schema '{}' (expected '{}')",
                self.schema, RESOLUTION_GRAPH_SCHEMA
            ));
        }
        if self.configurations.is_empty() {
            return Err("CycloneDX resolution graph has no configurations".to_string());
        }
        for configuration in &self.configurations {
            configuration.validate()?;
        }
        Ok(())
    }
}

impl CycloneDxResolutionConfiguration {
    fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("CycloneDX resolution graph configuration is missing name".to_string());
        }
        if self.components.is_empty() {
            return Err(format!(
                "CycloneDX resolution graph configuration '{}' has no components",
                self.name
            ));
        }
        let mut component_ids = BTreeSet::new();
        for component in &self.components {
            if component.id.trim().is_empty() {
                return Err(format!(
                    "CycloneDX resolution graph configuration '{}' has a component without id",
                    self.name
                ));
            }
            if !component_ids.insert(component.id.clone()) {
                return Err(format!(
                    "CycloneDX resolution graph configuration '{}' has duplicate component '{}'",
                    self.name, component.id
                ));
            }
        }
        for dependency in &self.dependencies {
            if dependency.from.trim().is_empty() || dependency.to.trim().is_empty() {
                return Err(format!(
                    "CycloneDX resolution graph configuration '{}' has a dependency without from/to",
                    self.name
                ));
            }
            if !component_ids.contains(&dependency.from) || !component_ids.contains(&dependency.to)
            {
                return Err(format!(
                    "CycloneDX resolution graph configuration '{}' has dependency edge '{} -> {}' outside component set",
                    self.name, dependency.from, dependency.to
                ));
            }
        }
        Ok(())
    }
}

fn value<'a>(inputs: &'a BTreeMap<String, String>, key: &str) -> &'a str {
    inputs.get(key).map(String::as_str).unwrap_or_default()
}

fn normalize_schema_version(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let version = trimmed.strip_prefix("VERSION_").unwrap_or(trimmed);
    let digits = version.replace('_', ".");
    if digits.contains('.') {
        digits
    } else if digits.len() == 2 {
        format!("{}.{}", &digits[0..1], &digits[1..])
    } else {
        digits
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

pub fn draft_contract_from_resolution_graph(
    graph: &CycloneDxResolutionGraphEvidence,
    options: CycloneDxDraftOptions,
) -> Result<CycloneDxSbomContract, String> {
    graph.validate()?;
    validate_draft_options(&options)?;
    let root_purl = purl(
        &options.root_group,
        &options.root_name,
        &options.root_version,
    );
    let root_component = CycloneDxComponent {
        component_type: options.root_component_type.to_lowercase(),
        bom_ref: root_purl.clone(),
        group: options.root_group,
        name: options.root_name,
        version: options.root_version,
        purl: root_purl,
        properties: BTreeMap::new(),
        licenses: Vec::new(),
    };

    let mut components_by_id = BTreeMap::new();
    let mut dependencies_by_ref = BTreeMap::<String, BTreeSet<String>>::new();
    for configuration in &graph.configurations {
        for component in &configuration.components {
            if component.group.is_empty()
                || component.module.is_empty()
                || component.version.is_empty()
            {
                continue;
            }
            let bom_ref = purl_for_component(component);
            components_by_id.insert(component.id.clone(), (component, bom_ref.clone()));
            dependencies_by_ref.entry(bom_ref).or_default();
        }
    }

    let mut root_children = BTreeSet::new();
    for configuration in &graph.configurations {
        for dependency in &configuration.dependencies {
            let Some((_, from_ref)) = components_by_id.get(&dependency.from) else {
                continue;
            };
            let Some((_, to_ref)) = components_by_id.get(&dependency.to) else {
                continue;
            };
            dependencies_by_ref
                .entry(from_ref.clone())
                .or_default()
                .insert(to_ref.clone());
            root_children.insert(from_ref.clone());
        }
    }

    let components = components_by_id
        .into_values()
        .map(|(component, bom_ref)| {
            let mut properties = BTreeMap::new();
            if !component.artifact_path.is_empty() {
                properties.insert(
                    "gradle:artifactPath".to_string(),
                    component.artifact_path.clone(),
                );
            }
            if !component.artifact_type.is_empty() {
                properties.insert(
                    "gradle:artifactType".to_string(),
                    component.artifact_type.clone(),
                );
            }
            if !component.artifact_extension.is_empty() {
                properties.insert(
                    "gradle:artifactExtension".to_string(),
                    component.artifact_extension.clone(),
                );
            }
            if !component.artifact_classifier.is_empty() {
                properties.insert(
                    "gradle:artifactClassifier".to_string(),
                    component.artifact_classifier.clone(),
                );
            }
            let metadata = if component.artifact_path.is_empty() {
                PomComponentMetadata::default()
            } else {
                read_pom_component_metadata(Path::new(&component.artifact_path)).unwrap_or_default()
            };
            for (key, value) in metadata.properties() {
                properties.insert(key, value);
            }
            CycloneDxComponent {
                component_type: "library".to_string(),
                bom_ref: bom_ref.clone(),
                group: component.group.clone(),
                name: component.module.clone(),
                version: component.version.clone(),
                purl: bom_ref,
                properties,
                licenses: metadata.licenses,
            }
        })
        .collect::<Vec<_>>();

    let mut dependencies = dependencies_by_ref
        .into_iter()
        .map(|(reference, depends_on)| CycloneDxDependency {
            reference,
            depends_on: depends_on.into_iter().collect(),
        })
        .collect::<Vec<_>>();
    dependencies.push(CycloneDxDependency {
        reference: root_component.bom_ref.clone(),
        depends_on: root_children.into_iter().collect(),
    });

    let contract = CycloneDxSbomContract {
        schema: CONTRACT_SCHEMA.to_string(),
        spec_version: options.spec_version,
        serial_number: options.serial_number,
        timestamp: options.timestamp,
        root_component,
        components,
        dependencies,
    };
    contract.validate()?;
    Ok(contract)
}

fn validate_draft_options(options: &CycloneDxDraftOptions) -> Result<(), String> {
    if options.spec_version.trim().is_empty()
        || options.serial_number.trim().is_empty()
        || options.timestamp.trim().is_empty()
        || options.root_name.trim().is_empty()
        || options.root_version.trim().is_empty()
        || options.root_component_type.trim().is_empty()
    {
        return Err("CycloneDX draft options are missing spec/root identity fields".to_string());
    }
    Ok(())
}

fn validate_aggregate_options(options: &CycloneDxAggregateOptions) -> Result<(), String> {
    if options.spec_version.trim().is_empty()
        || options.serial_number.trim().is_empty()
        || options.timestamp.trim().is_empty()
        || options.root_name.trim().is_empty()
        || options.root_version.trim().is_empty()
        || options.root_component_type.trim().is_empty()
    {
        return Err(
            "CycloneDX aggregate options are missing spec/root identity fields".to_string(),
        );
    }
    Ok(())
}

fn deterministic_serial_number(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update([0]);
    }
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "urn:uuid:{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    )
}

fn timestamp_from_epoch_millis(timestamp_ms: i64) -> Result<String, String> {
    let timestamp = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(timestamp_ms)
        .ok_or_else(|| format!("invalid CycloneDX timestamp millis '{timestamp_ms}'"))?;
    Ok(timestamp.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

fn merge_component(
    components_by_ref: &mut BTreeMap<String, CycloneDxComponent>,
    component: CycloneDxComponent,
) -> Result<(), String> {
    if let Some(existing) = components_by_ref.get(&component.bom_ref) {
        if existing != &component {
            return Err(format!(
                "CycloneDX aggregate input contains conflicting component '{}'",
                component.bom_ref
            ));
        }
    } else {
        components_by_ref.insert(component.bom_ref.clone(), component);
    }
    Ok(())
}

fn purl(group: &str, name: &str, version: &str) -> String {
    purl_with_qualifiers(group, name, version, &[])
}

fn purl_for_component(component: &CycloneDxResolvedComponent) -> String {
    let mut qualifiers = Vec::<(&str, &str)>::new();
    let extension = if component.artifact_extension.is_empty() {
        artifact_extension_from_path(&component.artifact_path)
    } else {
        component.artifact_extension.clone()
    };
    if !extension.is_empty() && extension != "jar" {
        qualifiers.push(("type", extension.as_str()));
    }
    if !component.artifact_type.is_empty()
        && component.artifact_type != extension
        && component.artifact_type != "jar"
    {
        qualifiers.push(("artifact_type", component.artifact_type.as_str()));
    }
    if !component.artifact_classifier.is_empty() {
        qualifiers.push(("classifier", component.artifact_classifier.as_str()));
    }
    purl_with_qualifiers(
        &component.group,
        &component.module,
        &component.version,
        &qualifiers,
    )
}

fn artifact_extension_from_path(path: &str) -> String {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_string()
}

fn purl_with_qualifiers(
    group: &str,
    name: &str,
    version: &str,
    qualifiers: &[(&str, &str)],
) -> String {
    let base = if group.is_empty() {
        format!("pkg:maven/{name}@{version}")
    } else {
        format!("pkg:maven/{group}/{name}@{version}")
    };
    if qualifiers.is_empty() {
        return base;
    }
    let mut sorted = qualifiers
        .iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(key, value)| (*key, *value))
        .collect::<Vec<_>>();
    sorted.sort_unstable_by(|a, b| a.0.cmp(b.0));
    let encoded = sorted
        .into_iter()
        .map(|(key, value)| format!("{key}={}", purl_encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{base}?{encoded}")
}

fn purl_encode(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PomComponentMetadata {
    name: String,
    description: String,
    url: String,
    licenses: Vec<CycloneDxLicenseChoice>,
}

impl PomComponentMetadata {
    fn properties(&self) -> Vec<(String, String)> {
        let mut properties = Vec::new();
        if !self.name.is_empty() {
            properties.push(("maven:pomName".to_string(), self.name.clone()));
        }
        if !self.description.is_empty() {
            properties.push(("maven:pomDescription".to_string(), self.description.clone()));
        }
        if !self.url.is_empty() {
            properties.push(("maven:pomUrl".to_string(), self.url.clone()));
        }
        properties
    }
}

fn read_pom_component_metadata(artifact_path: &Path) -> Option<PomComponentMetadata> {
    let pom =
        read_adjacent_pom(artifact_path).or_else(|| read_embedded_maven_pom(artifact_path))?;
    Some(parse_pom_component_metadata(&pom))
}

fn read_adjacent_pom(artifact_path: &Path) -> Option<String> {
    let file_stem = artifact_path.file_stem()?.to_str()?;
    let pom_path = artifact_path.with_file_name(format!("{file_stem}.pom"));
    std::fs::read_to_string(pom_path).ok()
}

fn read_embedded_maven_pom(artifact_path: &Path) -> Option<String> {
    if artifact_path.extension().and_then(|ext| ext.to_str()) != Some("jar") {
        return None;
    }
    let file = File::open(artifact_path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).ok()?;
        let name = entry.name().to_string();
        if name.starts_with("META-INF/maven/") && name.ends_with("/pom.xml") {
            let mut contents = String::new();
            if entry.read_to_string(&mut contents).is_ok() {
                return Some(contents);
            }
        }
    }
    None
}

fn parse_pom_component_metadata(pom: &str) -> PomComponentMetadata {
    let mut reader = quick_xml::Reader::from_str(pom);
    reader.trim_text(true);

    let mut metadata = PomComponentMetadata::default();
    let mut path = Vec::<String>::new();
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(event)) => {
                let name = String::from_utf8_lossy(event.name().local_name().as_ref()).to_string();
                path.push(name);
            }
            Ok(Event::Text(event)) => {
                let text = event.unescape().unwrap_or_default().trim().to_string();
                if text.is_empty() {
                    buf.clear();
                    continue;
                }
                match path.as_slice() {
                    [project, name] if project == "project" && name == "name" => {
                        metadata.name = text;
                    }
                    [project, description]
                        if project == "project" && description == "description" =>
                    {
                        metadata.description = text;
                    }
                    [project, url] if project == "project" && url == "url" => {
                        metadata.url = text;
                    }
                    [project, licenses, license, name]
                        if project == "project"
                            && licenses == "licenses"
                            && license == "license"
                            && name == "name" =>
                    {
                        metadata.licenses.push(CycloneDxLicenseChoice {
                            license: CycloneDxLicense { name: text },
                        });
                    }
                    _ => {}
                }
            }
            Ok(Event::End(_)) => {
                path.pop();
            }
            Ok(Event::Eof) => break,
            Err(_) => return PomComponentMetadata::default(),
            _ => {}
        }
        buf.clear();
    }
    metadata
        .licenses
        .sort_by(|a, b| a.license.name.cmp(&b.license.name));
    metadata
        .licenses
        .dedup_by(|a, b| a.license.name == b.license.name);
    metadata
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
    if !component.licenses.is_empty() {
        xml.push_str(&format!("{indent}  <licenses>\n"));
        for license in &component.licenses {
            xml.push_str(&format!(
                "{indent}    <license><name>{}</name></license>\n",
                xml_escape(&license.license.name)
            ));
        }
        xml.push_str(&format!("{indent}  </licenses>\n"));
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
                licenses: Vec::new(),
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
                    licenses: Vec::new(),
                },
                CycloneDxComponent {
                    component_type: "library".to_string(),
                    bom_ref: "pkg:maven/org.example/a@1.0.0?type=jar".to_string(),
                    group: "org.example".to_string(),
                    name: "a".to_string(),
                    version: "1.0.0".to_string(),
                    purl: "pkg:maven/org.example/a@1.0.0?type=jar".to_string(),
                    properties: BTreeMap::new(),
                    licenses: Vec::new(),
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

    fn sample_resolution_graph() -> CycloneDxResolutionGraphEvidence {
        CycloneDxResolutionGraphEvidence {
            schema: RESOLUTION_GRAPH_SCHEMA.to_string(),
            configurations: vec![CycloneDxResolutionConfiguration {
                name: "runtimeClasspath".to_string(),
                components: vec![
                    CycloneDxResolvedComponent {
                        id: "org.example:app:1.0".to_string(),
                        group: "org.example".to_string(),
                        module: "app".to_string(),
                        version: "1.0".to_string(),
                        project_path: String::new(),
                        artifact_path: String::new(),
                        artifact_type: String::new(),
                        artifact_extension: String::new(),
                        artifact_classifier: String::new(),
                    },
                    CycloneDxResolvedComponent {
                        id: "org.example:lib:1.1".to_string(),
                        group: "org.example".to_string(),
                        module: "lib".to_string(),
                        version: "1.1".to_string(),
                        project_path: String::new(),
                        artifact_path: "/repo/lib-1.1.jar".to_string(),
                        artifact_type: "jar".to_string(),
                        artifact_extension: "jar".to_string(),
                        artifact_classifier: String::new(),
                    },
                ],
                dependencies: vec![CycloneDxResolvedDependency {
                    from: "org.example:app:1.0".to_string(),
                    requested: "org.example:lib:1.+".to_string(),
                    to: "org.example:lib:1.1".to_string(),
                }],
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
        let mut contract = sample_contract();
        contract.components[0]
            .licenses
            .push(CycloneDxLicenseChoice {
                license: CycloneDxLicense {
                    name: "Apache-2.0".to_string(),
                },
            });
        let xml = render_xml(&contract).unwrap();
        assert!(xml.contains("http://cyclonedx.org/schema/bom/1.6"));
        assert!(xml.contains("<metadata>"));
        assert!(xml.find("org.example/a").unwrap() < xml.find("org.example/b").unwrap());
        assert!(xml.contains("<license><name>Apache-2.0</name></license>"));
        assert!(xml.contains("<dependencies>"));
    }

    #[test]
    fn rejects_missing_required_contract_fields() {
        let mut contract = sample_contract();
        contract.root_component.bom_ref.clear();
        let err = render_json(&contract).unwrap_err();
        assert!(err.contains("root component"));
    }

    #[test]
    fn accepts_valid_resolution_graph_evidence() {
        sample_resolution_graph().validate().unwrap();
    }

    #[test]
    fn rejects_resolution_graph_edges_outside_component_set() {
        let mut graph = sample_resolution_graph();
        graph.configurations[0].dependencies[0].to = "org.example:missing:1.0".to_string();
        let err = graph.validate().unwrap_err();
        assert!(err.contains("outside component set"));
    }

    #[test]
    fn rejects_duplicate_resolution_graph_components() {
        let mut graph = sample_resolution_graph();
        let duplicate = graph.configurations[0].components[0].clone();
        graph.configurations[0].components.push(duplicate);
        let err = graph.validate().unwrap_err();
        assert!(err.contains("duplicate component"));
    }

    #[test]
    fn validates_captured_cyclonedx_task_options() {
        let inputs = BTreeMap::from([
            (
                "cyclonedx_schema_version".to_string(),
                "VERSION_16".to_string(),
            ),
            (
                "cyclonedx_component_group".to_string(),
                "org.example".to_string(),
            ),
            ("cyclonedx_component_name".to_string(), "demo".to_string()),
            ("cyclonedx_component_version".to_string(), "1.0".to_string()),
            ("cyclonedx_project_type".to_string(), "LIBRARY".to_string()),
            (
                "cyclonedx_include_bom_serial_number".to_string(),
                "true".to_string(),
            ),
            (
                "cyclonedx_json_output".to_string(),
                "/tmp/bom.json".to_string(),
            ),
        ]);

        let options = validate_captured_task_options(&inputs).unwrap();
        assert_eq!("1.6", options.spec_version);
        assert_eq!("org.example", options.root_group);
        assert_eq!("demo", options.root_name);
        assert_eq!("library", options.root_component_type);
        assert!(options.include_bom_serial_number);
        assert_eq!("/tmp/bom.json", options.json_output);
    }

    #[test]
    fn derives_stable_serial_and_timestamp_for_cyclonedx_identity() {
        let policy = CycloneDxIdentityPolicy {
            build_id: "build-123".to_string(),
            task_path: ":cyclonedxDirectBom".to_string(),
            root_group: "org.example".to_string(),
            root_name: "demo".to_string(),
            root_version: "1.0".to_string(),
            schema_version: "1.6".to_string(),
            timestamp_ms: 1_778_595_445_123,
        };

        assert_eq!(policy.serial_number(), policy.serial_number());
        assert!(policy.serial_number().starts_with("urn:uuid:"));
        assert_eq!("2026-05-12T14:17:25Z", policy.timestamp().unwrap());

        let mut other = policy.clone();
        other.task_path = ":cyclonedxBom".to_string();
        assert_ne!(policy.serial_number(), other.serial_number());
    }

    #[test]
    fn rejects_incomplete_captured_cyclonedx_task_options() {
        let inputs = BTreeMap::from([(
            "cyclonedx_schema_version".to_string(),
            "VERSION_16".to_string(),
        )]);

        let err = validate_captured_task_options(&inputs).unwrap_err();
        assert!(err.contains("component-name"));
        assert!(err.contains("component-version"));
        assert!(err.contains("project-type"));
        assert!(err.contains("json-or-xml-output"));
    }

    #[test]
    fn drafts_deterministic_contract_from_resolution_graph_evidence() {
        let contract = draft_contract_from_resolution_graph(
            &sample_resolution_graph(),
            CycloneDxDraftOptions {
                spec_version: "1.6".to_string(),
                serial_number: "urn:uuid:00000000-0000-0000-0000-000000000002".to_string(),
                timestamp: "2026-05-12T11:30:00Z".to_string(),
                root_group: "org.example".to_string(),
                root_name: "demo".to_string(),
                root_version: "1.0".to_string(),
                root_component_type: "application".to_string(),
            },
        )
        .unwrap();

        assert_eq!(CONTRACT_SCHEMA, contract.schema);
        assert_eq!(
            "pkg:maven/org.example/demo@1.0",
            contract.root_component.bom_ref
        );
        assert!(contract
            .components
            .iter()
            .any(|component| component.bom_ref == "pkg:maven/org.example/lib@1.1"));
        assert!(contract.components.iter().any(|component| {
            component.bom_ref == "pkg:maven/org.example/lib@1.1"
                && component
                    .properties
                    .get("gradle:artifactPath")
                    .map(|path| path == "/repo/lib-1.1.jar")
                    .unwrap_or(false)
        }));
        assert!(contract.dependencies.iter().any(|dependency| {
            dependency.reference == "pkg:maven/org.example/app@1.0"
                && dependency
                    .depends_on
                    .contains(&"pkg:maven/org.example/lib@1.1".to_string())
        }));
        assert!(render_json(&contract)
            .unwrap()
            .contains("pkg:maven/org.example/lib@1.1"));
    }

    #[test]
    fn drafts_contract_with_adjacent_pom_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let jar_path = dir.path().join("lib-1.1.jar");
        let pom_path = dir.path().join("lib-1.1.pom");
        std::fs::write(&jar_path, b"not-a-real-jar").unwrap();
        std::fs::write(
            &pom_path,
            r#"
<project>
  <name>Example Lib</name>
  <description>Useful &amp; small</description>
  <url>https://example.test/lib</url>
  <licenses>
    <license>
      <name>Apache-2.0</name>
    </license>
  </licenses>
</project>
"#,
        )
        .unwrap();

        let mut graph = sample_resolution_graph();
        graph.configurations[0].components[1].artifact_path =
            jar_path.to_string_lossy().to_string();

        let contract = draft_contract_from_resolution_graph(
            &graph,
            CycloneDxDraftOptions {
                spec_version: "1.6".to_string(),
                serial_number: "urn:uuid:00000000-0000-0000-0000-000000000003".to_string(),
                timestamp: "2026-05-12T12:00:00Z".to_string(),
                root_group: "org.example".to_string(),
                root_name: "demo".to_string(),
                root_version: "1.0".to_string(),
                root_component_type: "application".to_string(),
            },
        )
        .unwrap();

        let component = contract
            .components
            .iter()
            .find(|component| component.bom_ref == "pkg:maven/org.example/lib@1.1")
            .unwrap();
        assert_eq!(
            Some(&jar_path.to_string_lossy().to_string()),
            component.properties.get("gradle:artifactPath")
        );
        assert_eq!(
            Some(&"Example Lib".to_string()),
            component.properties.get("maven:pomName")
        );
        assert_eq!(
            Some(&"Useful & small".to_string()),
            component.properties.get("maven:pomDescription")
        );
        assert_eq!(
            Some(&"https://example.test/lib".to_string()),
            component.properties.get("maven:pomUrl")
        );
        assert_eq!(
            vec![CycloneDxLicenseChoice {
                license: CycloneDxLicense {
                    name: "Apache-2.0".to_string(),
                },
            }],
            component.licenses
        );
    }

    #[test]
    fn drafts_contract_with_maven_purl_artifact_qualifiers() {
        let mut graph = sample_resolution_graph();
        graph.configurations[0].components[1].artifact_path =
            "/repo/lib-1.1-sources.zip".to_string();
        graph.configurations[0].components[1].artifact_type = "zip".to_string();
        graph.configurations[0].components[1].artifact_extension = "zip".to_string();
        graph.configurations[0].components[1].artifact_classifier = "sources".to_string();

        let contract = draft_contract_from_resolution_graph(
            &graph,
            CycloneDxDraftOptions {
                spec_version: "1.6".to_string(),
                serial_number: "urn:uuid:00000000-0000-0000-0000-000000000004".to_string(),
                timestamp: "2026-05-12T12:30:00Z".to_string(),
                root_group: "org.example".to_string(),
                root_name: "demo".to_string(),
                root_version: "1.0".to_string(),
                root_component_type: "application".to_string(),
            },
        )
        .unwrap();

        let component = contract
            .components
            .iter()
            .find(|component| component.name == "lib")
            .unwrap();
        assert_eq!(
            "pkg:maven/org.example/lib@1.1?classifier=sources&type=zip",
            component.bom_ref
        );
        assert_eq!(component.bom_ref, component.purl);
        assert_eq!(
            Some(&"zip".to_string()),
            component.properties.get("gradle:artifactType")
        );
        assert_eq!(
            Some(&"sources".to_string()),
            component.properties.get("gradle:artifactClassifier")
        );
    }

    #[test]
    fn aggregates_sbom_contracts_deterministically() {
        let mut first = sample_contract();
        first.root_component.bom_ref = "pkg:maven/org.example/app-a@1.0".to_string();
        first.root_component.purl = first.root_component.bom_ref.clone();
        first.root_component.name = "app-a".to_string();
        first.dependencies = vec![CycloneDxDependency {
            reference: first.root_component.bom_ref.clone(),
            depends_on: vec!["pkg:maven/org.example/a@1.0.0?type=jar".to_string()],
        }];

        let mut second = sample_contract();
        second.root_component.bom_ref = "pkg:maven/org.example/app-b@1.0".to_string();
        second.root_component.purl = second.root_component.bom_ref.clone();
        second.root_component.name = "app-b".to_string();
        second.dependencies = vec![CycloneDxDependency {
            reference: second.root_component.bom_ref.clone(),
            depends_on: vec!["pkg:maven/org.example/b@1.0.0?type=jar".to_string()],
        }];

        let aggregate = aggregate_contracts(
            &[second, first],
            CycloneDxAggregateOptions {
                spec_version: "1.6".to_string(),
                serial_number: "urn:uuid:00000000-0000-0000-0000-000000000005".to_string(),
                timestamp: "2026-05-12T13:00:00Z".to_string(),
                root_group: "org.example".to_string(),
                root_name: "aggregate".to_string(),
                root_version: "1.0".to_string(),
                root_component_type: "application".to_string(),
            },
        )
        .unwrap();

        assert_eq!(
            "pkg:maven/org.example/aggregate@1.0",
            aggregate.root_component.bom_ref
        );
        assert_eq!(
            vec![
                "pkg:maven/org.example/a@1.0.0?type=jar".to_string(),
                "pkg:maven/org.example/app-a@1.0".to_string(),
                "pkg:maven/org.example/app-b@1.0".to_string(),
                "pkg:maven/org.example/b@1.0.0?type=jar".to_string(),
            ],
            aggregate
                .components
                .iter()
                .map(|component| component.bom_ref.clone())
                .collect::<Vec<_>>()
        );
        let root_dependency = aggregate
            .dependencies
            .iter()
            .find(|dependency| dependency.reference == aggregate.root_component.bom_ref)
            .unwrap();
        assert_eq!(
            vec![
                "pkg:maven/org.example/app-a@1.0".to_string(),
                "pkg:maven/org.example/app-b@1.0".to_string(),
            ],
            root_dependency.depends_on
        );
        assert!(render_json(&aggregate).unwrap().contains("aggregate"));
    }

    #[test]
    fn rejects_conflicting_aggregate_components() {
        let first = sample_contract();
        let mut second = sample_contract();
        second.components[0].version = "2.0.0".to_string();

        let err = aggregate_contracts(
            &[first, second],
            CycloneDxAggregateOptions {
                spec_version: "1.6".to_string(),
                serial_number: "urn:uuid:00000000-0000-0000-0000-000000000006".to_string(),
                timestamp: "2026-05-12T13:30:00Z".to_string(),
                root_group: "org.example".to_string(),
                root_name: "aggregate".to_string(),
                root_version: "1.0".to_string(),
                root_component_type: "application".to_string(),
            },
        )
        .unwrap_err();
        assert!(err.contains("conflicting component"));
    }
}
