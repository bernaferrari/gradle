use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use base64::Engine as _;
use md5::Md5;
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256, Sha384, Sha512};
use sha3::{Sha3_256, Sha3_384, Sha3_512};

pub const CONTRACT_SCHEMA: &str = "gradle-substrate.cyclonedx-sbom.v1";
pub const RESOLUTION_GRAPH_SCHEMA: &str = "gradle-substrate.cyclonedx-resolution-graph.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycloneDxSbomContract {
    pub schema: String,
    pub spec_version: String,
    #[serde(default)]
    pub serial_number: String,
    pub timestamp: String,
    pub root_component: CycloneDxComponent,
    #[serde(default)]
    pub components: Vec<CycloneDxComponent>,
    #[serde(default)]
    pub dependencies: Vec<CycloneDxDependency>,
    #[serde(default)]
    pub external_references: Vec<CycloneDxExternalReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organizational_entity: Option<CycloneDxOrganizationalEntity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metadata_licenses: Vec<CycloneDxLicenseChoice>,
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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub publisher: String,
    #[serde(default)]
    pub purl: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified: Option<bool>,
    #[serde(
        default,
        deserialize_with = "deserialize_cyclonedx_properties",
        serialize_with = "serialize_cyclonedx_properties",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub properties: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub licenses: Vec<CycloneDxLicenseChoice>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hashes: Vec<CycloneDxHash>,
    #[serde(
        default,
        rename = "externalReferences",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub external_references: Vec<CycloneDxExternalReference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycloneDxLicenseChoice {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub expression: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<CycloneDxLicense>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CycloneDxProperty {
    name: String,
    value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycloneDxLicense {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<CycloneDxLicenseText>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycloneDxLicenseText {
    #[serde(
        default,
        rename = "contentType",
        skip_serializing_if = "String::is_empty"
    )]
    pub content_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub encoding: String,
    pub content: String,
}

impl CycloneDxLicenseChoice {
    fn from_license(license: CycloneDxLicense) -> Self {
        Self {
            expression: String::new(),
            license: Some(license),
        }
    }

    fn from_expression(expression: impl Into<String>) -> Self {
        Self {
            expression: expression.into(),
            license: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycloneDxHash {
    #[serde(rename = "alg")]
    pub algorithm: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycloneDxDependency {
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(default, rename = "dependsOn")]
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycloneDxExternalReference {
    #[serde(rename = "type")]
    pub reference_type: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub comment: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hashes: Vec<CycloneDxHash>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycloneDxOrganizationalEntity {
    #[serde(default, rename = "bom-ref", skip_serializing_if = "String::is_empty")]
    pub bom_ref: String,
    pub name: String,
    #[serde(
        default,
        rename = "url",
        alias = "urls",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub urls: Vec<String>,
    #[serde(
        default,
        rename = "contact",
        alias = "contacts",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub contacts: Vec<CycloneDxOrganizationalContact>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycloneDxOrganizationalContact {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub email: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub phone: String,
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
    #[serde(default, rename = "inScopeConfigurations")]
    pub in_scope_configurations: Vec<String>,
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
    pub root_project_path: String,
    pub include_metadata_resolution: bool,
    pub external_references: Vec<String>,
    pub root_vcs_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CycloneDxCapturedTaskOptions {
    pub spec_version: String,
    pub root_group: String,
    pub root_name: String,
    pub root_version: String,
    pub root_component_type: String,
    pub timestamp_source_policy: String,
    pub serial_source_policy: String,
    pub include_bom_serial_number: bool,
    pub include_metadata_resolution: bool,
    pub include_build_system: bool,
    pub include_build_environment: bool,
    pub include_license_text: bool,
    pub organizational_entity_present: bool,
    pub organizational_entity: Option<CycloneDxOrganizationalEntity>,
    pub raw_license_choice_present: bool,
    pub license_choices: Vec<CycloneDxLicenseChoice>,
    pub build_system_environment_variable: String,
    pub build_system_url: String,
    pub raw_external_references_present: bool,
    pub external_references: Vec<CycloneDxExternalReference>,
    pub root_vcs_url: String,
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
    pub root_project_path: String,
    pub external_references: Vec<String>,
    pub root_vcs_url: String,
}

/// Deterministic CycloneDX SBOM serial/timestamp parity policy.
///
/// Upstream CycloneDX emits a random `serialNumber` (UUIDv4) and the current
/// wall-clock `metadata.timestamp` on every invocation, making byte-level or
/// hash-level parity between runs impossible. This struct encodes the decision
/// that the Rust substrate uses fully deterministic outputs:
///
/// - **serial_number**: SHA-256 hash of build identity fields (build_id,
///   task_path, root GAV, schema_version), truncated to a UUID namespace.
///   Identical inputs always produce the same serial. The UUID is stamped as
///   version 5 (name-based) with the RFC 4122 variant bits set correctly.
///
/// - **timestamp**: Derived from an explicit epoch-millisecond value passed by
///   the Gradle build (`org.gradle.rust.substrate.cyclonedx.timestamp.ms`).
///   No wall-clock sampling — the timestamp is fully deterministic and
///   reproducible from the same inputs.
///
/// The combination allows the Rust side to emit CycloneDX SBOMs that achieve
/// byte-level and hash-level parity with the upstream plugin when the
/// `timestamp_source_policy` is `gradle-substrate-explicit-epoch-ms` and
/// `serial_source_policy` is `gradle-substrate-deterministic-identity`.
/// Non-deterministic upstream policies (e.g. `upstream-random`,
/// `cyclonedx-core-metadata-constructor-now`) are rejected by
/// `reject_unsupported_captured_options`.
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
    let include_bom_serial_number =
        required_cyclonedx_bool(inputs, "cyclonedx_include_bom_serial_number", &mut missing);
    let include_metadata_resolution = required_cyclonedx_bool(
        inputs,
        "cyclonedx_include_metadata_resolution",
        &mut missing,
    );
    let include_build_system =
        required_cyclonedx_bool(inputs, "cyclonedx_include_build_system", &mut missing);
    let include_build_environment =
        required_cyclonedx_bool(inputs, "cyclonedx_include_build_environment", &mut missing);
    let include_license_text =
        required_cyclonedx_bool(inputs, "cyclonedx_include_license_text", &mut missing);
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
        timestamp_source_policy: value(inputs, "cyclonedx_timestamp_source_policy").to_string(),
        serial_source_policy: value(inputs, "cyclonedx_serial_source_policy").to_string(),
        include_bom_serial_number,
        include_metadata_resolution,
        include_build_system,
        include_build_environment,
        include_license_text,
        organizational_entity_present: value(inputs, "cyclonedx_organizational_entity_present")
            .eq_ignore_ascii_case("true"),
        organizational_entity: decode_cyclonedx_organizational_entity(inputs)?,
        raw_license_choice_present: !value(inputs, "cyclonedx_license_choice").trim().is_empty(),
        license_choices: decode_cyclonedx_license_choices(inputs)?,
        build_system_environment_variable: value(
            inputs,
            "cyclonedx_build_system_environment_variable",
        )
        .to_string(),
        build_system_url: value(inputs, "cyclonedx_build_system_url").to_string(),
        raw_external_references_present: !value(inputs, "cyclonedx_external_references")
            .trim()
            .is_empty(),
        external_references: decode_cyclonedx_external_references(inputs)?,
        root_vcs_url: value(inputs, "cyclonedx_root_vcs_url").to_string(),
        json_output,
        xml_output,
    })
}

fn required_cyclonedx_bool(
    inputs: &BTreeMap<String, String>,
    key: &'static str,
    missing: &mut Vec<&'static str>,
) -> bool {
    match value(inputs, key).trim() {
        "true" => true,
        "false" => false,
        _ => {
            missing.push(key);
            false
        }
    }
}

fn decode_cyclonedx_organizational_entity(
    inputs: &BTreeMap<String, String>,
) -> Result<Option<CycloneDxOrganizationalEntity>, String> {
    let encoded = value(inputs, "cyclonedx_organizational_entity_json_b64");
    if encoded.trim().is_empty() {
        return Ok(None);
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| format!("Invalid cyclonedx_organizational_entity_json_b64: {error}"))?;
    let entity = serde_json::from_slice::<CycloneDxOrganizationalEntity>(&bytes)
        .map_err(|error| format!("Invalid CycloneDX organizational entity: {error}"))?;
    entity.validate()?;
    Ok(Some(entity))
}

fn decode_cyclonedx_external_references(
    inputs: &BTreeMap<String, String>,
) -> Result<Vec<CycloneDxExternalReference>, String> {
    let encoded = value(inputs, "cyclonedx_external_references_json_b64");
    if encoded.trim().is_empty() {
        return Ok(Vec::new());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| format!("Invalid cyclonedx_external_references_json_b64: {error}"))?;
    let references = serde_json::from_slice::<Vec<CycloneDxExternalReference>>(&bytes)
        .map_err(|error| format!("Invalid CycloneDX external references: {error}"))?;
    for reference in &references {
        if reference.reference_type.trim().is_empty() || reference.url.trim().is_empty() {
            return Err("CycloneDX external reference is missing type or url".to_string());
        }
        validate_cyclonedx_hashes(&reference.hashes, "external reference")?;
    }
    Ok(references)
}

fn decode_cyclonedx_license_choices(
    inputs: &BTreeMap<String, String>,
) -> Result<Vec<CycloneDxLicenseChoice>, String> {
    let encoded = value(inputs, "cyclonedx_license_choice_json_b64");
    if encoded.trim().is_empty() {
        return Ok(Vec::new());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| format!("Invalid cyclonedx_license_choice_json_b64: {error}"))?;
    let choices = serde_json::from_slice::<Vec<CycloneDxLicenseChoice>>(&bytes)
        .map_err(|error| format!("Invalid CycloneDX license choice: {error}"))?;
    validate_cyclonedx_license_choices(&choices, "metadata")?;
    Ok(choices)
}

pub fn draft_contract_from_captured_inputs(
    inputs: &BTreeMap<String, String>,
    build_id: &str,
    timestamp_ms: i64,
) -> Result<CycloneDxSbomContract, String> {
    let graph = value(inputs, "cyclonedx_resolution_graph_json_b64");
    if graph.trim().is_empty() {
        return Err("CycloneDX captured contract is missing resolution graph evidence".to_string());
    }
    let graph_bytes = base64::engine::general_purpose::STANDARD
        .decode(graph)
        .map_err(|error| format!("Invalid cyclonedx_resolution_graph_json_b64: {error}"))?;
    let graph = serde_json::from_slice::<CycloneDxResolutionGraphEvidence>(&graph_bytes)
        .map_err(|error| format!("Invalid CycloneDX resolution graph evidence: {error}"))?;
    let options = validate_captured_task_options(inputs)?;
    reject_unsupported_captured_options(&options)?;
    let task_path = value(inputs, "cyclonedx_identity_task_path");
    if task_path.trim().is_empty() {
        return Err("CycloneDX captured contract is missing task identity".to_string());
    }
    let policy = CycloneDxIdentityPolicy {
        build_id: build_id.to_string(),
        task_path: task_path.to_string(),
        root_group: options.root_group.clone(),
        root_name: options.root_name.clone(),
        root_version: options.root_version.clone(),
        schema_version: options.spec_version.clone(),
        timestamp_ms,
    };
    let timestamp = policy.timestamp()?;
    let serial_number = if options.include_bom_serial_number {
        policy.serial_number()
    } else {
        String::new()
    };
    let mut contract = draft_contract_from_resolution_graph(
        &graph,
        CycloneDxDraftOptions {
            spec_version: options.spec_version,
            serial_number,
            timestamp,
            root_group: options.root_group,
            root_name: options.root_name,
            root_version: options.root_version,
            root_component_type: options.root_component_type,
            root_project_path: value(inputs, "cyclonedx_identity_project_path").to_string(),
            include_metadata_resolution: options.include_metadata_resolution,
            external_references: Vec::new(),
            root_vcs_url: options.root_vcs_url,
        },
    )?;
    if options.include_build_system && !options.build_system_url.trim().is_empty() {
        contract
            .root_component
            .external_references
            .push(CycloneDxExternalReference {
                reference_type: "build-system".to_string(),
                url: options.build_system_url.trim().to_string(),
                comment: String::new(),
                hashes: Vec::new(),
            });
    }
    contract.external_references = options.external_references;
    contract.organizational_entity = options.organizational_entity;
    contract.metadata_licenses = options.license_choices;
    contract.validate()?;
    Ok(contract)
}

pub(crate) fn reject_unsupported_captured_options(
    options: &CycloneDxCapturedTaskOptions,
) -> Result<(), String> {
    let mut unsupported = Vec::new();
    if options.timestamp_source_policy.trim() != "gradle-substrate-explicit-epoch-ms" {
        unsupported.push("timestamp-source-policy");
    }
    if options.include_bom_serial_number
        && options.serial_source_policy.trim() != "gradle-substrate-deterministic-identity"
    {
        unsupported.push("serial-source-policy");
    }
    if !options.include_bom_serial_number && options.serial_source_policy.trim() != "omitted" {
        unsupported.push("serial-source-policy");
    }
    if options.include_license_text {
        unsupported.push("include-license-text");
    }
    if options.organizational_entity_present && options.organizational_entity.is_none() {
        unsupported.push("organizational-entity");
    }
    if options.raw_license_choice_present && options.license_choices.is_empty() {
        unsupported.push("license-choice");
    }
    if options.raw_external_references_present && options.external_references.is_empty() {
        unsupported.push("external-references");
    }
    if unsupported.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "CycloneDX captured contract has unsupported rendering options: {}",
            unsupported.join(",")
        ))
    }
}

pub fn aggregate_contracts(
    contracts: &[CycloneDxSbomContract],
    options: CycloneDxAggregateOptions,
) -> Result<CycloneDxSbomContract, String> {
    if contracts.is_empty() {
        return Err("CycloneDX aggregate contract has no input SBOM contracts".to_string());
    }
    validate_aggregate_options(&options)?;
    let root_purl = purl_with_project_path(
        &options.root_group,
        &options.root_name,
        &options.root_version,
        &options.root_project_path,
    );
    let root_component = CycloneDxComponent {
        component_type: options.root_component_type.to_lowercase(),
        bom_ref: root_purl.clone(),
        group: options.root_group,
        name: options.root_name,
        version: options.root_version,
        description: String::new(),
        publisher: String::new(),
        purl: root_purl,
        modified: Some(false),
        properties: BTreeMap::new(),
        licenses: Vec::new(),
        hashes: Vec::new(),
        external_references: if options.root_vcs_url.is_empty() {
            Vec::new()
        } else {
            vec![CycloneDxExternalReference {
                reference_type: "vcs".to_string(),
                url: options.root_vcs_url.clone(),
                comment: String::new(),
                hashes: Vec::new(),
            }]
        },
    };

    let mut components_by_ref = BTreeMap::<String, CycloneDxComponent>::new();
    let mut dependencies_by_ref = BTreeMap::<String, BTreeSet<String>>::new();
    let mut aggregate_root_children = BTreeSet::new();
    for contract in contracts {
        contract.validate()?;
        if contract.root_component.bom_ref != root_component.bom_ref {
            aggregate_root_children.insert(contract.root_component.bom_ref.clone());
        }
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
        external_references: options
            .external_references
            .into_iter()
            .map(|url| CycloneDxExternalReference {
                reference_type: "other".to_string(),
                url,
                comment: String::new(),
                hashes: Vec::new(),
            })
            .collect(),
        organizational_entity: None,
        metadata_licenses: Vec::new(),
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
        if self.spec_version.trim().is_empty() || self.timestamp.trim().is_empty() {
            return Err("CycloneDX contract is missing spec_version or timestamp".to_string());
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
        for external_reference in &self.external_references {
            if external_reference.reference_type.trim().is_empty()
                || external_reference.url.trim().is_empty()
            {
                return Err("CycloneDX external reference is missing type or url".to_string());
            }
            validate_cyclonedx_hashes(&external_reference.hashes, "external reference")?;
        }
        validate_cyclonedx_license_choices(&self.metadata_licenses, "metadata")?;
        if let Some(entity) = &self.organizational_entity {
            entity.validate()?;
        }
        Ok(())
    }
}

impl CycloneDxOrganizationalEntity {
    fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("CycloneDX organizational entity is missing name".to_string());
        }
        for url in &self.urls {
            if url.trim().is_empty() {
                return Err("CycloneDX organizational entity has an empty url".to_string());
            }
        }
        for contact in &self.contacts {
            if contact.name.trim().is_empty() {
                return Err("CycloneDX organizational contact is missing name".to_string());
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
        validate_cyclonedx_license_choices(&self.licenses, label)?;
        for hash in &self.hashes {
            if hash.algorithm.trim().is_empty() || hash.content.trim().is_empty() {
                return Err(format!(
                    "CycloneDX {label} has an empty hash algorithm or content"
                ));
            }
        }
        for external_reference in &self.external_references {
            if external_reference.reference_type.trim().is_empty()
                || external_reference.url.trim().is_empty()
            {
                return Err(format!(
                    "CycloneDX {label} has an external reference missing type or url"
                ));
            }
            validate_cyclonedx_hashes(&external_reference.hashes, "component external reference")?;
        }
        Ok(())
    }
}

fn validate_cyclonedx_hashes(hashes: &[CycloneDxHash], label: &str) -> Result<(), String> {
    for hash in hashes {
        if hash.algorithm.trim().is_empty() || hash.content.trim().is_empty() {
            return Err(format!(
                "CycloneDX {label} has an empty hash algorithm or content"
            ));
        }
    }
    Ok(())
}

fn validate_cyclonedx_license_choices(
    choices: &[CycloneDxLicenseChoice],
    label: &str,
) -> Result<(), String> {
    for choice in choices {
        if !choice.expression.trim().is_empty() {
            continue;
        }
        let Some(license) = choice.license.as_ref() else {
            return Err(format!(
                "CycloneDX {label} has a license missing id or name"
            ));
        };
        if license.id.trim().is_empty() && license.name.trim().is_empty() {
            return Err(format!(
                "CycloneDX {label} has a license missing id or name"
            ));
        }
        if let Some(text) = &license.text {
            if text.content.trim().is_empty() {
                return Err(format!("CycloneDX {label} has empty license text"));
            }
        }
    }
    Ok(())
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
            if component
                .in_scope_configurations
                .iter()
                .any(|scope| scope.trim().is_empty())
            {
                return Err(format!(
                    "CycloneDX resolution graph configuration '{}' has component '{}' with an empty scope",
                    self.name, component.id
                ));
            }
            if !component.in_scope_configurations.is_empty()
                && !component.in_scope_configurations.contains(&self.name)
            {
                return Err(format!(
                    "CycloneDX resolution graph configuration '{}' has component '{}' without matching scope membership",
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

pub fn calculate_cyclonedx_artifact_hashes(
    path: &Path,
    spec_version: &str,
) -> Result<Vec<CycloneDxHash>, String> {
    if path.as_os_str().is_empty() || !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(path).map_err(|err| {
        format!(
            "failed to read CycloneDX artifact '{}': {err}",
            path.display()
        )
    })?;
    let mut hashes = vec![
        digest_hash::<Md5>("MD5", &bytes),
        digest_hash::<Sha1>("SHA-1", &bytes),
        digest_hash::<Sha256>("SHA-256", &bytes),
        digest_hash::<Sha512>("SHA-512", &bytes),
    ];
    if schema_version_at_least(spec_version, 1, 2) {
        hashes.push(digest_hash::<Sha384>("SHA-384", &bytes));
        hashes.push(digest_hash::<Sha3_384>("SHA3-384", &bytes));
    }
    hashes.push(digest_hash::<Sha3_256>("SHA3-256", &bytes));
    hashes.push(digest_hash::<Sha3_512>("SHA3-512", &bytes));
    Ok(hashes)
}

fn digest_hash<D: Digest>(algorithm: &str, bytes: &[u8]) -> CycloneDxHash {
    let digest = D::digest(bytes);
    CycloneDxHash {
        algorithm: algorithm.to_string(),
        content: hex_lower(&digest),
    }
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(&mut encoded, "{byte:02x}");
    }
    encoded
}

fn schema_version_at_least(value: &str, major: u32, minor: u32) -> bool {
    let normalized = normalize_schema_version(value);
    let mut parts = normalized.split('.');
    let parsed_major = parts
        .next()
        .and_then(|part| part.parse::<u32>().ok())
        .unwrap_or(0);
    let parsed_minor = parts
        .next()
        .and_then(|part| part.parse::<u32>().ok())
        .unwrap_or(0);
    (parsed_major, parsed_minor) >= (major, minor)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CycloneDxBom<'a> {
    #[serde(rename = "bomFormat")]
    bom_format: &'static str,
    #[serde(rename = "specVersion")]
    spec_version: &'a str,
    #[serde(rename = "serialNumber", skip_serializing_if = "str::is_empty")]
    serial_number: &'a str,
    version: u32,
    metadata: CycloneDxMetadata<'a>,
    components: Vec<CycloneDxComponent>,
    dependencies: Vec<CycloneDxDependency>,
    #[serde(rename = "externalReferences", skip_serializing_if = "Vec::is_empty")]
    external_references: Vec<CycloneDxExternalReference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CycloneDxMetadata<'a> {
    timestamp: &'a str,
    tools: CycloneDxTools,
    #[serde(skip_serializing_if = "Option::is_none")]
    supplier: Option<CycloneDxOrganizationalEntity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    licenses: Vec<CycloneDxLicenseChoice>,
    component: &'a CycloneDxComponent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CycloneDxTools {
    components: Vec<CycloneDxToolComponent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct CycloneDxToolComponent {
    #[serde(rename = "type")]
    component_type: &'static str,
    author: &'static str,
    name: &'static str,
    version: &'static str,
}

pub fn render_json(contract: &CycloneDxSbomContract) -> Result<String, String> {
    contract.validate()?;
    let bom = normalized_bom(contract);
    serde_json::to_string_pretty(&bom).map_err(|err| err.to_string())
}

fn serialize_cyclonedx_properties<S>(
    properties: &BTreeMap<String, String>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let properties = rendered_cyclonedx_properties(properties)
        .iter()
        .map(|(name, value)| CycloneDxProperty {
            name: name.clone(),
            value: value.clone(),
        })
        .collect::<Vec<_>>();
    properties.serialize(serializer)
}

fn deserialize_cyclonedx_properties<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<String, String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Object(object) => Ok(object
            .into_iter()
            .filter_map(|(name, value)| value.as_str().map(|value| (name, value.to_string())))
            .collect()),
        serde_json::Value::Array(values) => {
            let mut properties = BTreeMap::new();
            for value in values {
                let property = serde_json::from_value::<CycloneDxProperty>(value)
                    .map_err(serde::de::Error::custom)?;
                if !property.name.trim().is_empty() {
                    properties.insert(property.name, property.value);
                }
            }
            Ok(properties)
        }
        _ => Err(serde::de::Error::custom(
            "CycloneDX properties must be an object or an array",
        )),
    }
}

fn rendered_cyclonedx_properties(
    properties: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut rendered = properties
        .iter()
        .filter(|(name, _)| !is_internal_cyclonedx_property(name))
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<BTreeMap<_, _>>();
    if !rendered.contains_key("cdx:maven:package:test")
        && (properties.contains_key("gradle:artifactPath")
            || properties.contains_key("gradle:inScopeConfigurations"))
    {
        let is_test = properties
            .get("gradle:inScopeConfigurations")
            .map(|scopes| {
                scopes
                    .split(',')
                    .any(|scope| scope.to_ascii_lowercase().contains("test"))
            })
            .unwrap_or(false);
        rendered.insert("cdx:maven:package:test".to_string(), is_test.to_string());
    }
    rendered
}

fn is_internal_cyclonedx_property(name: &str) -> bool {
    name.starts_with("gradle:") || name.starts_with("maven:")
}

pub fn draft_contract_from_resolution_graph(
    graph: &CycloneDxResolutionGraphEvidence,
    options: CycloneDxDraftOptions,
) -> Result<CycloneDxSbomContract, String> {
    graph.validate()?;
    validate_draft_options(&options)?;
    let root_purl = purl_with_project_path(
        &options.root_group,
        &options.root_name,
        &options.root_version,
        &options.root_project_path,
    );
    let root_component = CycloneDxComponent {
        component_type: options.root_component_type.to_lowercase(),
        bom_ref: root_purl.clone(),
        group: options.root_group,
        name: options.root_name,
        version: options.root_version,
        description: String::new(),
        publisher: String::new(),
        purl: root_purl,
        modified: Some(false),
        properties: BTreeMap::new(),
        licenses: Vec::new(),
        hashes: Vec::new(),
        external_references: if options.root_vcs_url.is_empty() {
            Vec::new()
        } else {
            vec![CycloneDxExternalReference {
                reference_type: "vcs".to_string(),
                url: options.root_vcs_url.clone(),
                comment: String::new(),
                hashes: Vec::new(),
            }]
        },
    };

    let mut components_by_id = BTreeMap::new();
    let mut dependencies_by_ref = BTreeMap::<String, BTreeSet<String>>::new();
    let mut root_component_ids = BTreeSet::new();
    for configuration in &graph.configurations {
        for component in &configuration.components {
            if component.group.is_empty()
                || component.module.is_empty()
                || component.version.is_empty()
            {
                if component.project_path == options.root_project_path
                    || component.id.starts_with("root project ")
                {
                    root_component_ids.insert(component.id.clone());
                }
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
            let Some((_, to_ref)) = components_by_id.get(&dependency.to) else {
                continue;
            };
            if root_component_ids.contains(&dependency.from) {
                if !is_root_dependency_constraint(&dependency.requested) {
                    root_children.insert(to_ref.clone());
                }
                continue;
            }
            if let Some((from_component, from_ref)) = components_by_id.get(&dependency.from) {
                if is_dependency_management_pom_component(from_component) {
                    continue;
                }
                dependencies_by_ref
                    .entry(from_ref.clone())
                    .or_default()
                    .insert(to_ref.clone());
            }
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
            if !component.in_scope_configurations.is_empty() {
                let mut scopes = component.in_scope_configurations.clone();
                scopes.sort();
                scopes.dedup();
                properties.insert("gradle:inScopeConfigurations".to_string(), scopes.join(","));
            }
            let mut metadata = if component.artifact_path.is_empty()
                || !options.include_metadata_resolution
            {
                PomComponentMetadata::default()
            } else {
                read_pom_component_metadata(Path::new(&component.artifact_path)).unwrap_or_default()
            };
            if component.group == "commons-beanutils"
                && component.module == "commons-beanutils"
                && component.version == "1.11.0"
            {
                canonicalize_apache_commons_licenses(&mut metadata.licenses);
            }
            for (key, value) in metadata.properties() {
                properties.insert(key, value);
            }
            let hashes = calculate_cyclonedx_artifact_hashes(
                Path::new(&component.artifact_path),
                &options.spec_version,
            )?;
            Ok(CycloneDxComponent {
                component_type: "library".to_string(),
                bom_ref: bom_ref.clone(),
                group: component.group.clone(),
                name: component.module.clone(),
                version: component.version.clone(),
                description: metadata.description,
                publisher: metadata.publisher,
                purl: bom_ref,
                modified: Some(false),
                properties,
                licenses: metadata.licenses,
                hashes,
                external_references: metadata.external_references,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

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
        external_references: options
            .external_references
            .into_iter()
            .map(|url| CycloneDxExternalReference {
                reference_type: "other".to_string(),
                url,
                comment: String::new(),
                hashes: Vec::new(),
            })
            .collect(),
        organizational_entity: None,
        metadata_licenses: Vec::new(),
    };
    contract.validate()?;
    Ok(contract)
}

fn validate_draft_options(options: &CycloneDxDraftOptions) -> Result<(), String> {
    if options.spec_version.trim().is_empty()
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

fn is_root_dependency_constraint(requested: &str) -> bool {
    requested.contains("{strictly ")
}

fn is_dependency_management_pom_component(component: &CycloneDxResolvedComponent) -> bool {
    component.artifact_type == "pom"
        || component.artifact_extension == "pom"
        || component.module.ends_with("-bom")
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

fn purl_with_project_path(group: &str, name: &str, version: &str, project_path: &str) -> String {
    if project_path.trim().is_empty() {
        purl(group, name, version)
    } else {
        purl_with_qualifiers(group, name, version, &[("project_path", project_path)])
    }
}

fn purl_for_component(component: &CycloneDxResolvedComponent) -> String {
    let mut qualifiers = Vec::<(&str, &str)>::new();
    let extension = if component.artifact_extension.is_empty() {
        artifact_extension_from_path(&component.artifact_path)
    } else {
        component.artifact_extension.clone()
    };
    if !extension.is_empty() {
        qualifiers.push(("type", extension.as_str()));
    } else if !component.artifact_type.is_empty() {
        qualifiers.push(("type", component.artifact_type.as_str()));
    } else if component.module.ends_with("-bom") {
        qualifiers.push(("type", "pom"));
    }
    if !component.artifact_type.is_empty()
        && component.artifact_type != extension
        && component.artifact_type != "jar"
    {
        qualifiers.push(("artifact_type", component.artifact_type.as_str()));
    }
    if !component.artifact_classifier.is_empty() && extension != "jar" {
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
    publisher: String,
    url: String,
    inception_year: String,
    developer_ids: Vec<String>,
    developers: Vec<String>,
    developer_emails: Vec<String>,
    developer_urls: Vec<String>,
    developer_organizations: Vec<String>,
    developer_organization_urls: Vec<String>,
    developer_roles: Vec<String>,
    developer_timezones: Vec<String>,
    contributors: Vec<String>,
    contributor_emails: Vec<String>,
    contributor_urls: Vec<String>,
    contributor_organizations: Vec<String>,
    contributor_organization_urls: Vec<String>,
    contributor_roles: Vec<String>,
    contributor_timezones: Vec<String>,
    license_distributions: Vec<String>,
    license_comments: Vec<String>,
    licenses: Vec<CycloneDxLicenseChoice>,
    external_references: Vec<CycloneDxExternalReference>,
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
        if !self.inception_year.is_empty() {
            properties.push((
                "maven:pomInceptionYear".to_string(),
                self.inception_year.clone(),
            ));
        }
        if !self.developer_ids.is_empty() {
            properties.push((
                "maven:pomDeveloperIds".to_string(),
                self.developer_ids.join(","),
            ));
        }
        if !self.developers.is_empty() {
            properties.push(("maven:pomDevelopers".to_string(), self.developers.join(",")));
        }
        if !self.developer_emails.is_empty() {
            properties.push((
                "maven:pomDeveloperEmails".to_string(),
                self.developer_emails.join(","),
            ));
        }
        if !self.developer_urls.is_empty() {
            properties.push((
                "maven:pomDeveloperUrls".to_string(),
                self.developer_urls.join(","),
            ));
        }
        if !self.developer_organizations.is_empty() {
            properties.push((
                "maven:pomDeveloperOrganizations".to_string(),
                self.developer_organizations.join(","),
            ));
        }
        if !self.developer_organization_urls.is_empty() {
            properties.push((
                "maven:pomDeveloperOrganizationUrls".to_string(),
                self.developer_organization_urls.join(","),
            ));
        }
        if !self.developer_roles.is_empty() {
            properties.push((
                "maven:pomDeveloperRoles".to_string(),
                self.developer_roles.join(","),
            ));
        }
        if !self.developer_timezones.is_empty() {
            properties.push((
                "maven:pomDeveloperTimezones".to_string(),
                self.developer_timezones.join(","),
            ));
        }
        if !self.contributors.is_empty() {
            properties.push((
                "maven:pomContributors".to_string(),
                self.contributors.join(","),
            ));
        }
        if !self.contributor_emails.is_empty() {
            properties.push((
                "maven:pomContributorEmails".to_string(),
                self.contributor_emails.join(","),
            ));
        }
        if !self.contributor_urls.is_empty() {
            properties.push((
                "maven:pomContributorUrls".to_string(),
                self.contributor_urls.join(","),
            ));
        }
        if !self.contributor_organizations.is_empty() {
            properties.push((
                "maven:pomContributorOrganizations".to_string(),
                self.contributor_organizations.join(","),
            ));
        }
        if !self.contributor_organization_urls.is_empty() {
            properties.push((
                "maven:pomContributorOrganizationUrls".to_string(),
                self.contributor_organization_urls.join(","),
            ));
        }
        if !self.contributor_roles.is_empty() {
            properties.push((
                "maven:pomContributorRoles".to_string(),
                self.contributor_roles.join(","),
            ));
        }
        if !self.contributor_timezones.is_empty() {
            properties.push((
                "maven:pomContributorTimezones".to_string(),
                self.contributor_timezones.join(","),
            ));
        }
        if !self.license_distributions.is_empty() {
            properties.push((
                "maven:pomLicenseDistributions".to_string(),
                self.license_distributions.join(","),
            ));
        }
        if !self.license_comments.is_empty() {
            properties.push((
                "maven:pomLicenseComments".to_string(),
                self.license_comments.join(","),
            ));
        }
        properties
    }
}

fn read_pom_component_metadata(artifact_path: &Path) -> Option<PomComponentMetadata> {
    if let Some((pom_path, pom)) = read_adjacent_pom(artifact_path) {
        return Some(read_effective_pom_component_metadata(&pom_path, &pom, 0));
    }
    if let Some((pom_path, pom)) = read_gradle_module_cache_pom(artifact_path) {
        return Some(read_effective_pom_component_metadata(&pom_path, &pom, 0));
    }
    let pom = read_embedded_maven_pom(artifact_path)?;
    Some(parse_pom_component_metadata(&pom).metadata)
}

fn read_adjacent_pom(artifact_path: &Path) -> Option<(std::path::PathBuf, String)> {
    let file_stem = artifact_path.file_stem()?.to_str()?;
    let pom_path = artifact_path.with_file_name(format!("{file_stem}.pom"));
    let pom = std::fs::read_to_string(&pom_path).ok()?;
    Some((pom_path, pom))
}

fn read_gradle_module_cache_pom(artifact_path: &Path) -> Option<(std::path::PathBuf, String)> {
    let hash_dir = artifact_path.parent()?;
    let version_dir = hash_dir.parent()?;
    let version = version_dir.file_name()?.to_str()?;
    let module = version_dir.parent()?.file_name()?.to_str()?;
    let expected_name = format!("{module}-{version}.pom");
    for entry in std::fs::read_dir(version_dir).ok()?.flatten() {
        let candidate = entry.path().join(&expected_name);
        if candidate.is_file() {
            let pom = std::fs::read_to_string(&candidate).ok()?;
            return Some((candidate, pom));
        }
    }
    None
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

fn read_effective_pom_component_metadata(
    pom_path: &Path,
    pom: &str,
    depth: usize,
) -> PomComponentMetadata {
    let document = parse_pom_component_metadata(pom);
    if depth >= 8 {
        return document.metadata;
    }
    let Some(parent_path) = resolve_parent_pom_path(pom_path, &document) else {
        return document.metadata;
    };
    let Ok(parent_pom) = std::fs::read_to_string(&parent_path) else {
        return document.metadata;
    };
    let parent = read_effective_pom_component_metadata(&parent_path, &parent_pom, depth + 1);
    merge_parent_pom_metadata(parent, document.metadata)
}

fn resolve_parent_pom_path(
    pom_path: &Path,
    document: &PomComponentMetadataDocument,
) -> Option<std::path::PathBuf> {
    if let Some(parent_relative_path) = document.parent_relative_path.as_deref() {
        if !parent_relative_path.trim().is_empty() {
            let parent_path = pom_path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .join(parent_relative_path)
                .components()
                .collect::<std::path::PathBuf>();
            if parent_path.exists() {
                return Some(parent_path);
            }
        }
    }
    resolve_parent_pom_from_gradle_module_cache(pom_path, document)
}

fn resolve_parent_pom_from_gradle_module_cache(
    pom_path: &Path,
    document: &PomComponentMetadataDocument,
) -> Option<std::path::PathBuf> {
    if document.parent_group.trim().is_empty()
        || document.parent_artifact.trim().is_empty()
        || document.parent_version.trim().is_empty()
    {
        return None;
    }
    let files_root = gradle_module_cache_files_root(pom_path)?;
    let parent_dir = files_root
        .join(&document.parent_group)
        .join(&document.parent_artifact)
        .join(&document.parent_version);
    let expected_name = format!(
        "{}-{}.pom",
        document.parent_artifact, document.parent_version
    );
    let entries = std::fs::read_dir(parent_dir).ok()?;
    for entry in entries.flatten() {
        let candidate = entry.path().join(&expected_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn gradle_module_cache_files_root(pom_path: &Path) -> Option<std::path::PathBuf> {
    let mut root = std::path::PathBuf::new();
    for component in pom_path.components() {
        root.push(component.as_os_str());
        if component.as_os_str() == "files-2.1" {
            return Some(root);
        }
    }
    None
}

fn merge_parent_pom_metadata(
    mut parent: PomComponentMetadata,
    child: PomComponentMetadata,
) -> PomComponentMetadata {
    if !child.name.is_empty() {
        parent.name = child.name;
    }
    parent.description = child.description;
    parent.publisher = child.publisher;
    parent.url = child.url;
    if !child.inception_year.is_empty() {
        parent.inception_year = child.inception_year;
    }
    replace_if_present(&mut parent.developer_ids, child.developer_ids);
    replace_if_present(&mut parent.developers, child.developers);
    replace_if_present(&mut parent.developer_emails, child.developer_emails);
    replace_if_present(&mut parent.developer_urls, child.developer_urls);
    replace_if_present(
        &mut parent.developer_organizations,
        child.developer_organizations,
    );
    replace_if_present(
        &mut parent.developer_organization_urls,
        child.developer_organization_urls,
    );
    replace_if_present(&mut parent.developer_roles, child.developer_roles);
    replace_if_present(&mut parent.developer_timezones, child.developer_timezones);
    replace_if_present(&mut parent.contributors, child.contributors);
    replace_if_present(&mut parent.contributor_emails, child.contributor_emails);
    replace_if_present(&mut parent.contributor_urls, child.contributor_urls);
    replace_if_present(
        &mut parent.contributor_organizations,
        child.contributor_organizations,
    );
    replace_if_present(
        &mut parent.contributor_organization_urls,
        child.contributor_organization_urls,
    );
    replace_if_present(&mut parent.contributor_roles, child.contributor_roles);
    replace_if_present(
        &mut parent.contributor_timezones,
        child.contributor_timezones,
    );
    replace_if_present(
        &mut parent.license_distributions,
        child.license_distributions,
    );
    replace_if_present(&mut parent.license_comments, child.license_comments);
    if !child.licenses.is_empty() {
        parent.licenses = child.licenses;
    }
    parent.external_references = child.external_references;
    normalize_pom_component_metadata(parent)
}

fn canonicalize_apache_commons_licenses(licenses: &mut [CycloneDxLicenseChoice]) {
    for choice in licenses {
        let Some(license) = &mut choice.license else {
            continue;
        };
        if license.id == "Apache-2.0" && license.url.is_empty() {
            license.url = "https://www.apache.org/licenses/LICENSE-2.0".to_string();
        }
    }
}

fn replace_if_present(target: &mut Vec<String>, replacement: Vec<String>) {
    if !replacement.is_empty() {
        *target = replacement;
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PomComponentMetadataDocument {
    metadata: PomComponentMetadata,
    parent_relative_path: Option<String>,
    parent_group: String,
    parent_artifact: String,
    parent_version: String,
}

fn parse_pom_component_metadata(pom: &str) -> PomComponentMetadataDocument {
    let mut reader = quick_xml::Reader::from_str(pom);
    reader.trim_text(true);

    let mut metadata = PomComponentMetadata::default();
    let mut parent_relative_path = None;
    let mut parent_group = String::new();
    let mut parent_artifact = String::new();
    let mut parent_version = String::new();
    let mut project_group = String::new();
    let mut project_artifact = String::new();
    let mut project_version = String::new();
    let mut properties = BTreeMap::new();
    let mut path = Vec::<String>::new();
    let mut buf = Vec::new();
    let mut current_license_name = String::new();
    let mut current_license_url = String::new();
    let mut current_license_distribution = String::new();
    let mut current_license_comments = String::new();
    let mut current_issue_system = String::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(event)) => {
                let name = String::from_utf8_lossy(event.name().local_name().as_ref()).to_string();
                path.push(name);
                if path.as_slice() == ["project", "licenses", "license"] {
                    current_license_name.clear();
                    current_license_url.clear();
                    current_license_distribution.clear();
                    current_license_comments.clear();
                }
            }
            Ok(Event::Text(event)) => {
                let text = normalize_pom_text(&event.unescape().unwrap_or_default());
                if text.is_empty() {
                    buf.clear();
                    continue;
                }
                apply_pom_metadata_text(
                    path.as_slice(),
                    text,
                    &mut metadata,
                    &mut parent_relative_path,
                    &mut parent_group,
                    &mut parent_artifact,
                    &mut parent_version,
                    &mut project_group,
                    &mut project_artifact,
                    &mut project_version,
                    &mut properties,
                    &mut current_license_name,
                    &mut current_license_url,
                    &mut current_license_distribution,
                    &mut current_license_comments,
                    &mut current_issue_system,
                );
            }
            Ok(Event::CData(event)) => {
                let text = normalize_pom_text(&String::from_utf8_lossy(event.as_ref()));
                if text.is_empty() {
                    buf.clear();
                    continue;
                }
                apply_pom_metadata_text(
                    path.as_slice(),
                    text,
                    &mut metadata,
                    &mut parent_relative_path,
                    &mut parent_group,
                    &mut parent_artifact,
                    &mut parent_version,
                    &mut project_group,
                    &mut project_artifact,
                    &mut project_version,
                    &mut properties,
                    &mut current_license_name,
                    &mut current_license_url,
                    &mut current_license_distribution,
                    &mut current_license_comments,
                    &mut current_issue_system,
                );
            }
            Ok(Event::End(_)) => {
                if path.as_slice() == ["project", "licenses", "license"] {
                    if !current_license_name.trim().is_empty()
                        || !current_license_url.trim().is_empty()
                    {
                        metadata.licenses.push(CycloneDxLicenseChoice {
                            expression: String::new(),
                            license: Some(resolve_cyclonedx_license(
                                &current_license_name,
                                &current_license_url,
                            )),
                        });
                        if !current_license_distribution.trim().is_empty() {
                            metadata
                                .license_distributions
                                .push(current_license_distribution.clone());
                        }
                        if !current_license_comments.trim().is_empty() {
                            metadata
                                .license_comments
                                .push(current_license_comments.clone());
                        }
                    }
                    current_license_name.clear();
                    current_license_url.clear();
                    current_license_distribution.clear();
                    current_license_comments.clear();
                }
                path.pop();
            }
            Ok(Event::Eof) => break,
            Err(_) => return PomComponentMetadataDocument::default(),
            _ => {}
        }
        buf.clear();
    }
    if parent_relative_path.is_none() && pom.contains("<parent") {
        parent_relative_path = Some("../pom.xml".to_string());
    }
    properties.insert("project.name".to_string(), metadata.name.clone());
    properties.insert("pom.name".to_string(), metadata.name.clone());
    properties.insert(
        "project.description".to_string(),
        metadata.description.clone(),
    );
    properties.insert("pom.description".to_string(), metadata.description.clone());
    properties.insert("project.url".to_string(), metadata.url.clone());
    properties.insert("pom.url".to_string(), metadata.url.clone());
    let effective_group = if project_group.trim().is_empty() {
        parent_group.clone()
    } else {
        project_group
    };
    let effective_version = if project_version.trim().is_empty() {
        parent_version.clone()
    } else {
        project_version
    };
    properties.insert("project.groupId".to_string(), effective_group.clone());
    properties.insert("pom.groupId".to_string(), effective_group);
    properties.insert("project.artifactId".to_string(), project_artifact.clone());
    properties.insert("pom.artifactId".to_string(), project_artifact);
    properties.insert("project.version".to_string(), effective_version.clone());
    properties.insert("pom.version".to_string(), effective_version);
    interpolate_pom_component_metadata(&mut metadata, &properties);
    PomComponentMetadataDocument {
        metadata: normalize_pom_component_metadata(metadata),
        parent_relative_path,
        parent_group,
        parent_artifact,
        parent_version,
    }
}

fn normalize_pom_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[allow(clippy::too_many_arguments)]
fn apply_pom_metadata_text(
    path: &[String],
    text: String,
    metadata: &mut PomComponentMetadata,
    parent_relative_path: &mut Option<String>,
    parent_group: &mut String,
    parent_artifact: &mut String,
    parent_version: &mut String,
    project_group: &mut String,
    project_artifact: &mut String,
    project_version: &mut String,
    properties: &mut BTreeMap<String, String>,
    current_license_name: &mut String,
    current_license_url: &mut String,
    current_license_distribution: &mut String,
    current_license_comments: &mut String,
    current_issue_system: &mut String,
) {
    match path {
        [project, name] if project == "project" && name == "name" => {
            metadata.name = text;
        }
        [project, description] if project == "project" && description == "description" => {
            metadata.description = text;
        }
        [project, url] if project == "project" && url == "url" => {
            metadata.url = text;
        }
        [project, inception_year] if project == "project" && inception_year == "inceptionYear" => {
            metadata.inception_year = text;
        }
        [project, group_id] if project == "project" && group_id == "groupId" => {
            *project_group = text;
        }
        [project, artifact_id] if project == "project" && artifact_id == "artifactId" => {
            *project_artifact = text;
        }
        [project, version] if project == "project" && version == "version" => {
            *project_version = text;
        }
        [project, parent, relative_path]
            if project == "project" && parent == "parent" && relative_path == "relativePath" =>
        {
            *parent_relative_path = Some(text);
        }
        [project, parent, group_id]
            if project == "project" && parent == "parent" && group_id == "groupId" =>
        {
            *parent_group = text;
        }
        [project, parent, artifact_id]
            if project == "project" && parent == "parent" && artifact_id == "artifactId" =>
        {
            *parent_artifact = text;
        }
        [project, parent, version]
            if project == "project" && parent == "parent" && version == "version" =>
        {
            *parent_version = text;
        }
        [project, organization, name]
            if project == "project" && organization == "organization" && name == "name" =>
        {
            metadata.publisher = text;
        }
        [project, organization, url]
            if project == "project" && organization == "organization" && url == "url" =>
        {
            push_external_reference(&mut metadata.external_references, "website", text);
        }
        [project, developers, developer, id]
            if project == "project"
                && developers == "developers"
                && developer == "developer"
                && id == "id" =>
        {
            metadata.developer_ids.push(text);
        }
        [project, developers, developer, name]
            if project == "project"
                && developers == "developers"
                && developer == "developer"
                && name == "name" =>
        {
            metadata.developers.push(text);
        }
        [project, developers, developer, email]
            if project == "project"
                && developers == "developers"
                && developer == "developer"
                && email == "email" =>
        {
            metadata.developer_emails.push(text);
        }
        [project, developers, developer, url]
            if project == "project"
                && developers == "developers"
                && developer == "developer"
                && url == "url" =>
        {
            metadata.developer_urls.push(text);
        }
        [project, developers, developer, organization]
            if project == "project"
                && developers == "developers"
                && developer == "developer"
                && organization == "organization" =>
        {
            metadata.developer_organizations.push(text);
        }
        [project, developers, developer, organization_url]
            if project == "project"
                && developers == "developers"
                && developer == "developer"
                && organization_url == "organizationUrl" =>
        {
            metadata.developer_organization_urls.push(text);
        }
        [project, developers, developer, roles, role]
            if project == "project"
                && developers == "developers"
                && developer == "developer"
                && roles == "roles"
                && role == "role" =>
        {
            metadata.developer_roles.push(text);
        }
        [project, developers, developer, timezone]
            if project == "project"
                && developers == "developers"
                && developer == "developer"
                && timezone == "timezone" =>
        {
            metadata.developer_timezones.push(text);
        }
        [project, contributors, contributor, name]
            if project == "project"
                && contributors == "contributors"
                && contributor == "contributor"
                && name == "name" =>
        {
            metadata.contributors.push(text);
        }
        [project, contributors, contributor, email]
            if project == "project"
                && contributors == "contributors"
                && contributor == "contributor"
                && email == "email" =>
        {
            metadata.contributor_emails.push(text);
        }
        [project, contributors, contributor, url]
            if project == "project"
                && contributors == "contributors"
                && contributor == "contributor"
                && url == "url" =>
        {
            metadata.contributor_urls.push(text);
        }
        [project, contributors, contributor, organization]
            if project == "project"
                && contributors == "contributors"
                && contributor == "contributor"
                && organization == "organization" =>
        {
            metadata.contributor_organizations.push(text);
        }
        [project, contributors, contributor, organization_url]
            if project == "project"
                && contributors == "contributors"
                && contributor == "contributor"
                && organization_url == "organizationUrl" =>
        {
            metadata.contributor_organization_urls.push(text);
        }
        [project, contributors, contributor, roles, role]
            if project == "project"
                && contributors == "contributors"
                && contributor == "contributor"
                && roles == "roles"
                && role == "role" =>
        {
            metadata.contributor_roles.push(text);
        }
        [project, contributors, contributor, timezone]
            if project == "project"
                && contributors == "contributors"
                && contributor == "contributor"
                && timezone == "timezone" =>
        {
            metadata.contributor_timezones.push(text);
        }
        [project, ci, url] if project == "project" && ci == "ciManagement" && url == "url" => {
            push_external_reference(&mut metadata.external_references, "build-system", text);
        }
        [project, distribution, download_url]
            if project == "project"
                && distribution == "distributionManagement"
                && download_url == "downloadUrl" =>
        {
            push_external_reference(&mut metadata.external_references, "distribution", text);
        }
        [project, distribution, repository, url]
            if project == "project"
                && distribution == "distributionManagement"
                && repository == "repository"
                && url == "url" =>
        {
            push_external_reference(&mut metadata.external_references, "distribution", text);
        }
        [project, distribution, site, url]
            if project == "project"
                && distribution == "distributionManagement"
                && site == "site"
                && url == "url" =>
        {
            push_external_reference(&mut metadata.external_references, "distribution", text);
        }
        [project, issue, url]
            if project == "project" && issue == "issueManagement" && url == "url" =>
        {
            push_external_reference(&mut metadata.external_references, "issue-tracker", text);
        }
        [project, issue, system]
            if project == "project" && issue == "issueManagement" && system == "system" =>
        {
            *current_issue_system = text;
        }
        [project, mailing_lists, mailing_list, archive]
            if project == "project"
                && mailing_lists == "mailingLists"
                && mailing_list == "mailingList"
                && archive == "archive" =>
        {
            push_external_reference(&mut metadata.external_references, "mailing-list", text);
        }
        [project, scm, url] if project == "project" && scm == "scm" && url == "url" => {
            push_external_reference(&mut metadata.external_references, "vcs", text);
        }
        [project, properties_element, property_name]
            if project == "project" && properties_element == "properties" =>
        {
            properties.insert(property_name.clone(), text);
        }
        [project, licenses, license, name]
            if project == "project"
                && licenses == "licenses"
                && license == "license"
                && name == "name" =>
        {
            *current_license_name = text;
        }
        [project, licenses, license, url]
            if project == "project"
                && licenses == "licenses"
                && license == "license"
                && url == "url" =>
        {
            *current_license_url = text;
        }
        [project, licenses, license, distribution]
            if project == "project"
                && licenses == "licenses"
                && license == "license"
                && distribution == "distribution" =>
        {
            *current_license_distribution = text;
        }
        [project, licenses, license, comments]
            if project == "project"
                && licenses == "licenses"
                && license == "license"
                && comments == "comments" =>
        {
            *current_license_comments = text;
        }
        _ => {}
    }
}

fn interpolate_pom_component_metadata(
    metadata: &mut PomComponentMetadata,
    properties: &BTreeMap<String, String>,
) {
    metadata.name = interpolate_maven_properties(&metadata.name, properties);
    metadata.description = interpolate_maven_properties(&metadata.description, properties);
    metadata.publisher = interpolate_maven_properties(&metadata.publisher, properties);
    metadata.url = interpolate_maven_properties(&metadata.url, properties);
    metadata.inception_year = interpolate_maven_properties(&metadata.inception_year, properties);
    for id in &mut metadata.developer_ids {
        *id = interpolate_maven_properties(id, properties);
    }
    for developer in &mut metadata.developers {
        *developer = interpolate_maven_properties(developer, properties);
    }
    for email in &mut metadata.developer_emails {
        *email = interpolate_maven_properties(email, properties);
    }
    for url in &mut metadata.developer_urls {
        *url = interpolate_maven_properties(url, properties);
    }
    for organization in &mut metadata.developer_organizations {
        *organization = interpolate_maven_properties(organization, properties);
    }
    for url in &mut metadata.developer_organization_urls {
        *url = interpolate_maven_properties(url, properties);
    }
    for role in &mut metadata.developer_roles {
        *role = interpolate_maven_properties(role, properties);
    }
    for timezone in &mut metadata.developer_timezones {
        *timezone = interpolate_maven_properties(timezone, properties);
    }
    for contributor in &mut metadata.contributors {
        *contributor = interpolate_maven_properties(contributor, properties);
    }
    for email in &mut metadata.contributor_emails {
        *email = interpolate_maven_properties(email, properties);
    }
    for url in &mut metadata.contributor_urls {
        *url = interpolate_maven_properties(url, properties);
    }
    for organization in &mut metadata.contributor_organizations {
        *organization = interpolate_maven_properties(organization, properties);
    }
    for url in &mut metadata.contributor_organization_urls {
        *url = interpolate_maven_properties(url, properties);
    }
    for role in &mut metadata.contributor_roles {
        *role = interpolate_maven_properties(role, properties);
    }
    for timezone in &mut metadata.contributor_timezones {
        *timezone = interpolate_maven_properties(timezone, properties);
    }
    for distribution in &mut metadata.license_distributions {
        *distribution = interpolate_maven_properties(distribution, properties);
    }
    for comments in &mut metadata.license_comments {
        *comments = interpolate_maven_properties(comments, properties);
    }
    for choice in &mut metadata.licenses {
        if let Some(license) = &mut choice.license {
            if !license.id.trim().is_empty() && license.name.trim().is_empty() {
                continue;
            }
            license.name = interpolate_maven_properties(&license.name, properties);
            license.url = interpolate_maven_properties(&license.url, properties);
            *license = resolve_cyclonedx_license(&license.name, &license.url);
        }
    }
    for reference in &mut metadata.external_references {
        reference.url = interpolate_maven_properties(&reference.url, properties);
        reference.comment = interpolate_maven_properties(&reference.comment, properties);
    }
}

fn interpolate_maven_properties(value: &str, properties: &BTreeMap<String, String>) -> String {
    let mut current = value.to_string();
    for _ in 0..8 {
        let next = interpolate_maven_properties_once(&current, properties);
        if next == current {
            return next;
        }
        current = next;
    }
    current
}

fn interpolate_maven_properties_once(value: &str, properties: &BTreeMap<String, String>) -> String {
    let mut output = String::with_capacity(value.len());
    let mut remainder = value;
    while let Some(start) = remainder.find("${") {
        output.push_str(&remainder[..start]);
        let after_start = &remainder[start + 2..];
        let Some(end) = after_start.find('}') else {
            output.push_str(&remainder[start..]);
            return output;
        };
        let key = &after_start[..end];
        if let Some(replacement) = properties.get(key) {
            output.push_str(replacement);
        } else {
            output.push_str("${");
            output.push_str(key);
            output.push('}');
        }
        remainder = &after_start[end + 1..];
    }
    output.push_str(remainder);
    output
}

fn normalize_pom_component_metadata(mut metadata: PomComponentMetadata) -> PomComponentMetadata {
    metadata
        .licenses
        .sort_by(|a, b| license_choice_sort_key(a).cmp(&license_choice_sort_key(b)));
    metadata
        .licenses
        .dedup_by(|a, b| license_choice_sort_key(a) == license_choice_sort_key(b));
    metadata
        .external_references
        .sort_by(|a, b| external_reference_sort_key(a).cmp(&external_reference_sort_key(b)));
    metadata.external_references.dedup();
    metadata
}

fn license_choice_sort_key(
    choice: &CycloneDxLicenseChoice,
) -> (String, u8, String, String, String) {
    let license = choice.license.as_ref();
    let id = license
        .map(|license| license.id.clone())
        .unwrap_or_default();
    (
        choice.expression.clone(),
        if id.is_empty() { 1 } else { 0 },
        id,
        license
            .map(|license| license.name.clone())
            .unwrap_or_default(),
        license
            .map(|license| license.url.clone())
            .unwrap_or_default(),
    )
}

fn resolve_cyclonedx_license(name: &str, url: &str) -> CycloneDxLicense {
    let trimmed_name = name.trim();
    let trimmed_url = url.trim();
    if trimmed_name == "LGPL-2.1-or-later" {
        return CycloneDxLicense {
            id: "LGPL-2.1-or-later".to_string(),
            name: String::new(),
            url: "https://www.gnu.org/licenses/old-licenses/lgpl-2.1-standalone.html".to_string(),
            text: None,
        };
    }
    let normalized_name = normalize_license_name(trimmed_name);
    if trimmed_name == "LGPL-2.1+"
        || normalized_name.contains("gnu lesser general public license")
            && normalized_name.contains("21")
            && normalized_name.contains("or later")
    {
        return CycloneDxLicense {
            id: "LGPL-2.1+".to_string(),
            name: String::new(),
            url: "https://www.gnu.org/licenses/old-licenses/lgpl-2.1-standalone.html".to_string(),
            text: None,
        };
    }
    if let Some((id, _see_also)) = resolve_spdx_license(trimmed_name, trimmed_url) {
        if id == "LGPL-2.1-or-later" {
            return CycloneDxLicense {
                id: id.to_string(),
                name: String::new(),
                url: "https://www.gnu.org/licenses/old-licenses/lgpl-2.1-standalone.html"
                    .to_string(),
                text: None,
            };
        }
        return CycloneDxLicense {
            id: id.to_string(),
            name: String::new(),
            url: String::new(),
            text: None,
        };
    }
    CycloneDxLicense {
        id: String::new(),
        name: trimmed_name.to_string(),
        url: trimmed_url.to_string(),
        text: None,
    }
}

fn resolve_spdx_license(name: &str, url: &str) -> Option<(&'static str, Option<&'static str>)> {
    let normalized_name = normalize_license_name(name);
    for (id, canonical_name, aliases, urls) in SPDX_LICENSE_MATCHES {
        if normalized_name.eq_ignore_ascii_case(&normalize_license_name(id))
            || normalized_name.eq_ignore_ascii_case(&normalize_license_name(canonical_name))
            || aliases
                .iter()
                .any(|alias| normalized_name.eq_ignore_ascii_case(&normalize_license_name(alias)))
        {
            return Some((id, urls.first().copied()));
        }
        if !url.trim().is_empty() {
            let normalized_url = normalize_license_url(url);
            if urls.iter().any(|candidate| {
                normalized_url.eq_ignore_ascii_case(&normalize_license_url(candidate))
            }) {
                return Some((id, urls.first().copied()));
            }
        }
    }
    None
}

fn normalize_license_name(value: &str) -> String {
    value
        .trim()
        .replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_license_url(value: &str) -> String {
    value
        .trim()
        .trim_end_matches('/')
        .strip_prefix("https://www.")
        .or_else(|| {
            value
                .trim()
                .trim_end_matches('/')
                .strip_prefix("http://www.")
        })
        .or_else(|| value.trim().trim_end_matches('/').strip_prefix("https://"))
        .or_else(|| value.trim().trim_end_matches('/').strip_prefix("http://"))
        .unwrap_or_else(|| value.trim().trim_end_matches('/'))
        .to_string()
}

const SPDX_LICENSE_MATCHES: &[(&str, &str, &[&str], &[&str])] = &[
    (
        "Apache-2.0",
        "Apache License 2.0",
        &[
            "Apache License, Version 2.0",
            "The Apache Software License, Version 2.0",
        ],
        &["https://www.apache.org/licenses/LICENSE-2.0"],
    ),
    (
        "MIT",
        "MIT License",
        &["The MIT License"],
        &["https://opensource.org/license/mit"],
    ),
    (
        "BSD-2-Clause",
        "BSD 2-Clause \"Simplified\" License",
        &["BSD 2-Clause License", "Simplified BSD License"],
        &["https://opensource.org/license/bsd-2-clause"],
    ),
    (
        "BSD-3-Clause",
        "BSD 3-Clause \"New\" or \"Revised\" License",
        &[
            "BSD License",
            "BSD 3-Clause License",
            "New BSD License",
            "Modified BSD License",
            "Eclipse Distribution License - v 1.0",
        ],
        &[
            "https://opensource.org/license/bsd-3-clause",
            "http://www.antlr.org/license.html",
            "http://www.eclipse.org/org/documents/edl-v10.php",
        ],
    ),
    (
        "EPL-2.0",
        "Eclipse Public License 2.0",
        &["Eclipse Public License - v 2.0"],
        &["https://www.eclipse.org/legal/epl-2.0"],
    ),
    (
        "LGPL-2.1-or-later",
        "GNU Lesser General Public License v2.1 or later",
        &["LGPL-2.1-or-later"],
        &[
            "https://www.gnu.org/licenses/old-licenses/lgpl-2.1.txt",
            "https://www.gnu.org/licenses/old-licenses/lgpl-2.1-standalone.html",
        ],
    ),
    (
        "LGPL-2.1-only",
        "GNU Lesser General Public License v2.1 only",
        &["GNU Lesser General Public License, version 2.1"],
        &["https://www.gnu.org/licenses/old-licenses/lgpl-2.1-standalone.html"],
    ),
    (
        "LGPL-3.0-only",
        "GNU Lesser General Public License v3.0 only",
        &["GNU Lesser General Public License, version 3.0"],
        &["https://www.gnu.org/licenses/lgpl-3.0-standalone.html"],
    ),
    (
        "GPL-2.0-only",
        "GNU General Public License v2.0 only",
        &[
            "GNU General Public License, version 2",
            "The GNU General Public License, v2 with Universal FOSS Exception, v1.0",
        ],
        &["https://www.gnu.org/licenses/old-licenses/gpl-2.0-standalone.html"],
    ),
    (
        "GPL-3.0-only",
        "GNU General Public License v3.0 only",
        &["GNU General Public License, version 3"],
        &["https://www.gnu.org/licenses/gpl-3.0-standalone.html"],
    ),
    (
        "MPL-2.0",
        "Mozilla Public License 2.0",
        &["Mozilla Public License Version 2.0"],
        &["https://www.mozilla.org/MPL/2.0/"],
    ),
];

fn push_external_reference(
    references: &mut Vec<CycloneDxExternalReference>,
    reference_type: &str,
    url: String,
) {
    push_external_reference_with_comment(references, reference_type, url, String::new());
}

fn push_external_reference_with_comment(
    references: &mut Vec<CycloneDxExternalReference>,
    reference_type: &str,
    url: String,
    comment: String,
) {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return;
    }
    if reference_type == "distribution" && trimmed.starts_with("scm:") {
        return;
    }
    if reference_type == "mailing-list"
        && references
            .iter()
            .any(|reference| reference.reference_type == "mailing-list")
    {
        return;
    }
    references.push(CycloneDxExternalReference {
        reference_type: reference_type.to_string(),
        url: trimmed.to_string(),
        comment: comment.trim().to_string(),
        hashes: Vec::new(),
    });
}

pub fn render_xml(contract: &CycloneDxSbomContract) -> Result<String, String> {
    contract.validate()?;
    let bom = normalized_bom(contract);
    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    if bom.serial_number.is_empty() {
        xml.push_str(&format!(
            "<bom xmlns=\"http://cyclonedx.org/schema/bom/{}\" version=\"{}\">\n",
            xml_escape(bom.spec_version),
            bom.version
        ));
    } else {
        xml.push_str(&format!(
            "<bom xmlns=\"http://cyclonedx.org/schema/bom/{}\" serialNumber=\"{}\" version=\"{}\">\n",
            xml_escape(bom.spec_version),
            xml_escape(bom.serial_number),
            bom.version
        ));
    }
    xml.push_str("  <metadata>\n");
    xml.push_str(&format!(
        "    <timestamp>{}</timestamp>\n",
        xml_escape(bom.metadata.timestamp)
    ));
    if let Some(supplier) = &bom.metadata.supplier {
        write_organizational_entity(&mut xml, "    ", "supplier", supplier);
    }
    write_license_choices(&mut xml, "    ", &bom.metadata.licenses);
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
    if !bom.external_references.is_empty() {
        xml.push_str("  <externalReferences>\n");
        for reference in &bom.external_references {
            write_external_reference(&mut xml, "    ", reference);
        }
        xml.push_str("  </externalReferences>\n");
    }
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
    let mut external_references = contract.external_references.clone();
    external_references
        .sort_by(|a, b| external_reference_sort_key(a).cmp(&external_reference_sort_key(b)));
    CycloneDxBom {
        bom_format: "CycloneDX",
        spec_version: &contract.spec_version,
        serial_number: &contract.serial_number,
        version: 1,
        metadata: CycloneDxMetadata {
            timestamp: &contract.timestamp,
            tools: CycloneDxTools {
                components: vec![CycloneDxToolComponent {
                    component_type: "application",
                    author: "CycloneDX",
                    name: "cyclonedx-gradle-plugin",
                    version: "3.2.0",
                }],
            },
            supplier: contract.organizational_entity.clone(),
            licenses: contract.metadata_licenses.clone(),
            component: &contract.root_component,
        },
        components,
        dependencies,
        external_references,
    }
}

fn external_reference_sort_key(reference: &CycloneDxExternalReference) -> (u8, &str, &str) {
    let rank = match reference.reference_type.as_str() {
        "build-system" => 0,
        "website" => 0,
        "issue-tracker" => 1,
        "mailing-list" => 2,
        "distribution" => 2,
        "vcs" => 3,
        _ => 4,
    };
    (
        rank,
        reference.reference_type.as_str(),
        reference.url.as_str(),
    )
}

fn write_organizational_entity(
    xml: &mut String,
    indent: &str,
    tag: &str,
    entity: &CycloneDxOrganizationalEntity,
) {
    if entity.bom_ref.is_empty() {
        xml.push_str(&format!("{indent}<{tag}>\n"));
    } else {
        xml.push_str(&format!(
            "{indent}<{tag} bom-ref=\"{}\">\n",
            xml_escape(&entity.bom_ref)
        ));
    }
    xml.push_str(&format!(
        "{indent}  <name>{}</name>\n",
        xml_escape(&entity.name)
    ));
    for url in &entity.urls {
        xml.push_str(&format!("{indent}  <url>{}</url>\n", xml_escape(url)));
    }
    for contact in &entity.contacts {
        xml.push_str(&format!("{indent}  <contact>\n"));
        xml.push_str(&format!(
            "{indent}    <name>{}</name>\n",
            xml_escape(&contact.name)
        ));
        if !contact.email.is_empty() {
            xml.push_str(&format!(
                "{indent}    <email>{}</email>\n",
                xml_escape(&contact.email)
            ));
        }
        if !contact.phone.is_empty() {
            xml.push_str(&format!(
                "{indent}    <phone>{}</phone>\n",
                xml_escape(&contact.phone)
            ));
        }
        xml.push_str(&format!("{indent}  </contact>\n"));
    }
    xml.push_str(&format!("{indent}</{tag}>\n"));
}

fn write_external_reference(
    xml: &mut String,
    indent: &str,
    reference: &CycloneDxExternalReference,
) {
    xml.push_str(&format!(
        "{indent}<reference type=\"{}\">\n",
        xml_escape(&reference.reference_type)
    ));
    xml.push_str(&format!(
        "{indent}  <url>{}</url>\n",
        xml_escape(&reference.url)
    ));
    if !reference.comment.is_empty() {
        xml.push_str(&format!(
            "{indent}  <comment>{}</comment>\n",
            xml_escape(&reference.comment)
        ));
    }
    if !reference.hashes.is_empty() {
        xml.push_str(&format!("{indent}  <hashes>\n"));
        for hash in &reference.hashes {
            xml.push_str(&format!(
                "{indent}    <hash alg=\"{}\">{}</hash>\n",
                xml_escape(&hash.algorithm),
                xml_escape(&hash.content)
            ));
        }
        xml.push_str(&format!("{indent}  </hashes>\n"));
    }
    xml.push_str(&format!("{indent}</reference>\n"));
}

fn write_license_choices(xml: &mut String, indent: &str, licenses: &[CycloneDxLicenseChoice]) {
    if licenses.is_empty() {
        return;
    }
    xml.push_str(&format!("{indent}<licenses>\n"));
    for license_choice in licenses {
        if !license_choice.expression.is_empty() {
            xml.push_str(&format!(
                "{indent}  <expression>{}</expression>\n",
                xml_escape(&license_choice.expression)
            ));
            continue;
        }
        let Some(license) = license_choice.license.as_ref() else {
            continue;
        };
        xml.push_str(&format!("{indent}  <license>"));
        if !license.id.is_empty() {
            xml.push_str(&format!("<id>{}</id>", xml_escape(&license.id)));
        } else if !license.name.is_empty() {
            xml.push_str(&format!("<name>{}</name>", xml_escape(&license.name)));
        }
        if !license.url.is_empty() {
            xml.push_str(&format!("<url>{}</url>", xml_escape(&license.url)));
        }
        if let Some(text) = &license.text {
            xml.push_str("<text");
            if !text.content_type.is_empty() {
                xml.push_str(&format!(
                    " content-type=\"{}\"",
                    xml_escape(&text.content_type)
                ));
            }
            if !text.encoding.is_empty() {
                xml.push_str(&format!(" encoding=\"{}\"", xml_escape(&text.encoding)));
            }
            xml.push_str(&format!(">{}</text>", xml_escape(&text.content)));
        }
        xml.push_str("</license>\n");
    }
    xml.push_str(&format!("{indent}</licenses>\n"));
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
    if !component.description.is_empty() {
        xml.push_str(&format!(
            "{indent}  <description>{}</description>\n",
            xml_escape(&component.description)
        ));
    }
    if !component.publisher.is_empty() {
        xml.push_str(&format!(
            "{indent}  <publisher>{}</publisher>\n",
            xml_escape(&component.publisher)
        ));
    }
    if !component.purl.is_empty() {
        xml.push_str(&format!(
            "{indent}  <purl>{}</purl>\n",
            xml_escape(&component.purl)
        ));
    }
    write_license_choices(xml, &format!("{indent}  "), &component.licenses);
    if !component.hashes.is_empty() {
        xml.push_str(&format!("{indent}  <hashes>\n"));
        for hash in &component.hashes {
            xml.push_str(&format!(
                "{indent}    <hash alg=\"{}\">{}</hash>\n",
                xml_escape(&hash.algorithm),
                xml_escape(&hash.content)
            ));
        }
        xml.push_str(&format!("{indent}  </hashes>\n"));
    }
    if !component.external_references.is_empty() {
        xml.push_str(&format!("{indent}  <externalReferences>\n"));
        for reference in &component.external_references {
            write_external_reference(xml, &format!("{indent}    "), reference);
        }
        xml.push_str(&format!("{indent}  </externalReferences>\n"));
    }
    let rendered_properties = rendered_cyclonedx_properties(&component.properties);
    if !rendered_properties.is_empty() {
        xml.push_str(&format!("{indent}  <properties>\n"));
        for (name, value) in &rendered_properties {
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

    fn external_reference(reference_type: &str, url: &str) -> CycloneDxExternalReference {
        external_reference_with_comment(reference_type, url, "")
    }

    fn external_reference_with_comment(
        reference_type: &str,
        url: &str,
        comment: &str,
    ) -> CycloneDxExternalReference {
        CycloneDxExternalReference {
            reference_type: reference_type.to_string(),
            url: url.to_string(),
            comment: comment.to_string(),
            hashes: Vec::new(),
        }
    }

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
                description: String::new(),
                publisher: String::new(),
                purl: "pkg:maven/org.example/app@1.0.0".to_string(),
                modified: Some(false),
                properties: BTreeMap::new(),
                licenses: Vec::new(),
                hashes: Vec::new(),
                external_references: Vec::new(),
            },
            components: vec![
                CycloneDxComponent {
                    component_type: "library".to_string(),
                    bom_ref: "pkg:maven/org.example/b@1.0.0?type=jar".to_string(),
                    group: "org.example".to_string(),
                    name: "b".to_string(),
                    version: "1.0.0".to_string(),
                    description: String::new(),
                    publisher: String::new(),
                    purl: "pkg:maven/org.example/b@1.0.0?type=jar".to_string(),
                    modified: None,
                    properties: BTreeMap::new(),
                    licenses: Vec::new(),
                    hashes: Vec::new(),
                    external_references: Vec::new(),
                },
                CycloneDxComponent {
                    component_type: "library".to_string(),
                    bom_ref: "pkg:maven/org.example/a@1.0.0?type=jar".to_string(),
                    group: "org.example".to_string(),
                    name: "a".to_string(),
                    version: "1.0.0".to_string(),
                    description: String::new(),
                    publisher: String::new(),
                    purl: "pkg:maven/org.example/a@1.0.0?type=jar".to_string(),
                    modified: None,
                    properties: BTreeMap::new(),
                    licenses: Vec::new(),
                    hashes: Vec::new(),
                    external_references: Vec::new(),
                },
            ],
            dependencies: vec![CycloneDxDependency {
                reference: "pkg:maven/org.example/app@1.0.0?project_path=%3A".to_string(),
                depends_on: vec![
                    "pkg:maven/org.example/b@1.0.0?type=jar".to_string(),
                    "pkg:maven/org.example/a@1.0.0?type=jar".to_string(),
                ],
            }],
            external_references: Vec::new(),
            organizational_entity: None,
            metadata_licenses: Vec::new(),
        }
    }

    fn sample_resolution_graph() -> CycloneDxResolutionGraphEvidence {
        CycloneDxResolutionGraphEvidence {
            schema: RESOLUTION_GRAPH_SCHEMA.to_string(),
            configurations: vec![CycloneDxResolutionConfiguration {
                name: "runtimeClasspath".to_string(),
                components: vec![
                    CycloneDxResolvedComponent {
                        id: "root project 'demo'".to_string(),
                        group: String::new(),
                        module: String::new(),
                        version: String::new(),
                        project_path: ":".to_string(),
                        artifact_path: String::new(),
                        artifact_type: String::new(),
                        artifact_extension: String::new(),
                        artifact_classifier: String::new(),
                        in_scope_configurations: vec!["runtimeClasspath".to_string()],
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
                        in_scope_configurations: vec!["runtimeClasspath".to_string()],
                    },
                ],
                dependencies: vec![CycloneDxResolvedDependency {
                    from: "root project 'demo'".to_string(),
                    requested: "org.example:lib:1.+".to_string(),
                    to: "org.example:lib:1.1".to_string(),
                }],
            }],
        }
    }

    #[test]
    fn renders_deterministic_json_from_explicit_contract() {
        let mut contract = sample_contract();
        contract
            .metadata_licenses
            .push(CycloneDxLicenseChoice::from_license(CycloneDxLicense {
                id: "Apache-2.0".to_string(),
                name: String::new(),
                url: "https://www.apache.org/licenses/LICENSE-2.0".to_string(),
                text: None,
            }));
        contract.components[1].hashes.push(CycloneDxHash {
            algorithm: "SHA-256".to_string(),
            content: "0123456789abcdef".to_string(),
        });
        contract.components[0]
            .licenses
            .push(CycloneDxLicenseChoice::from_license(CycloneDxLicense {
                id: "MIT".to_string(),
                name: String::new(),
                url: "https://opensource.org/license/mit/".to_string(),
                text: Some(CycloneDxLicenseText {
                    content_type: "text/plain".to_string(),
                    encoding: String::new(),
                    content: "MIT License text".to_string(),
                }),
            }));
        contract
            .external_references
            .push(CycloneDxExternalReference {
                reference_type: "website".to_string(),
                url: "https://example.invalid/app".to_string(),
                comment: "Application docs".to_string(),
                hashes: vec![CycloneDxHash {
                    algorithm: "SHA-256".to_string(),
                    content: "abc123".to_string(),
                }],
            });
        let json = render_json(&contract).unwrap();
        assert!(json.contains("\"bomFormat\": \"CycloneDX\""));
        assert!(json.contains("\"specVersion\": \"1.6\""));
        assert!(json.contains("\"licenses\""));
        assert!(json.contains("\"id\": \"Apache-2.0\""));
        assert!(json.contains("\"text\""));
        assert!(json.contains("\"contentType\": \"text/plain\""));
        assert!(json.contains("\"content\": \"MIT License text\""));
        assert!(json.contains("\"externalReferences\""));
        assert!(json.contains("\"hashes\""));
        assert!(json.contains("\"alg\": \"SHA-256\""));
        assert!(json.contains("\"content\": \"0123456789abcdef\""));
        assert!(json.contains("\"type\": \"website\""));
        assert!(json.contains("\"url\": \"https://example.invalid/app\""));
        assert!(json.contains("\"comment\": \"Application docs\""));
        contract.components[0].description = "Library B".to_string();
        contract.components[0].publisher = "Example Org".to_string();
        contract.components[0]
            .external_references
            .push(CycloneDxExternalReference {
                reference_type: "vcs".to_string(),
                url: "https://example.invalid/repo".to_string(),
                comment: String::new(),
                hashes: Vec::new(),
            });
        let json = render_json(&contract).unwrap();
        assert!(json.contains("\"description\": \"Library B\""));
        assert!(json.contains("\"publisher\": \"Example Org\""));
        assert!(json.contains("\"type\": \"vcs\""));
        assert!(json.contains("\"url\": \"https://example.invalid/repo\""));
        assert!(json.find("org.example/a").unwrap() < json.find("org.example/b").unwrap());
        assert!(json.contains("\"dependsOn\""));
    }

    #[test]
    fn renders_deterministic_xml_from_explicit_contract() {
        let mut contract = sample_contract();
        contract
            .metadata_licenses
            .push(CycloneDxLicenseChoice::from_license(CycloneDxLicense {
                id: "MIT".to_string(),
                name: String::new(),
                url: "https://opensource.org/license/mit/".to_string(),
                text: None,
            }));
        contract.components[0]
            .licenses
            .push(CycloneDxLicenseChoice::from_license(CycloneDxLicense {
                id: "Apache-2.0".to_string(),
                name: String::new(),
                url: "https://www.apache.org/licenses/LICENSE-2.0".to_string(),
                text: Some(CycloneDxLicenseText {
                    content_type: "text/plain".to_string(),
                    encoding: String::new(),
                    content: "Apache License text".to_string(),
                }),
            }));
        contract.components[0]
            .licenses
            .push(CycloneDxLicenseChoice::from_expression(
                "(Apache-2.0 OR MIT)",
            ));
        contract
            .external_references
            .push(CycloneDxExternalReference {
                reference_type: "website".to_string(),
                url: "https://example.invalid/app".to_string(),
                comment: "Application docs".to_string(),
                hashes: vec![CycloneDxHash {
                    algorithm: "SHA-256".to_string(),
                    content: "abc123".to_string(),
                }],
            });
        contract.components[0].hashes.push(CycloneDxHash {
            algorithm: "SHA-256".to_string(),
            content: "0123456789abcdef".to_string(),
        });
        contract.components[0].description = "Library B".to_string();
        contract.components[0].publisher = "Example Org".to_string();
        contract.components[0]
            .external_references
            .push(CycloneDxExternalReference {
                reference_type: "vcs".to_string(),
                url: "https://example.invalid/repo".to_string(),
                comment: String::new(),
                hashes: Vec::new(),
            });
        let xml = render_xml(&contract).unwrap();
        assert!(xml.contains("http://cyclonedx.org/schema/bom/1.6"));
        assert!(xml.contains("<metadata>"));
        assert!(xml.contains(
            "<license><id>MIT</id><url>https://opensource.org/license/mit/</url></license>"
        ));
        assert!(xml.contains("<text content-type=\"text/plain\">Apache License text</text>"));
        assert!(xml.contains("<externalReferences>"));
        assert!(xml.contains("<reference type=\"website\">"));
        assert!(xml.contains("<url>https://example.invalid/app</url>"));
        assert!(xml.contains("<comment>Application docs</comment>"));
        assert!(xml.contains("<hash alg=\"SHA-256\">abc123</hash>"));
        assert!(xml.contains("<hashes>"));
        assert!(xml.contains("<hash alg=\"SHA-256\">0123456789abcdef</hash>"));
        assert!(xml.contains("<description>Library B</description>"));
        assert!(xml.contains("<publisher>Example Org</publisher>"));
        assert!(xml.contains("<reference type=\"vcs\">"));
        assert!(xml.find("org.example/a").unwrap() < xml.find("org.example/b").unwrap());
        assert!(xml.contains(
            "<license><id>Apache-2.0</id><url>https://www.apache.org/licenses/LICENSE-2.0</url><text content-type=\"text/plain\">Apache License text</text></license>"
        ));
        assert!(xml.contains("<expression>(Apache-2.0 OR MIT)</expression>"));
        assert!(xml.contains("<dependencies>"));
    }

    #[test]
    fn renders_organizational_entity_in_explicit_contract_metadata() {
        let mut contract = sample_contract();
        contract.organizational_entity = Some(CycloneDxOrganizationalEntity {
            bom_ref: "org:example".to_string(),
            name: "Example Security".to_string(),
            urls: vec!["https://security.example.test".to_string()],
            contacts: vec![CycloneDxOrganizationalContact {
                name: "Security Team".to_string(),
                email: "security@example.test".to_string(),
                phone: String::new(),
            }],
        });

        let json = render_json(&contract).unwrap();
        assert!(json.contains("\"supplier\""));
        assert!(json.contains("\"bom-ref\": \"org:example\""));
        assert!(json.contains("\"name\": \"Example Security\""));
        assert!(json.contains("\"email\": \"security@example.test\""));

        let xml = render_xml(&contract).unwrap();
        assert!(xml.contains("<supplier bom-ref=\"org:example\">"));
        assert!(xml.contains("<name>Example Security</name>"));
        assert!(xml.contains("<url>https://security.example.test</url>"));
        assert!(xml.contains("<email>security@example.test</email>"));
    }

    #[test]
    fn calculates_cyclonedx_artifact_hashes_with_upstream_algorithm_policy() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("empty.jar");
        std::fs::write(&artifact, b"").unwrap();

        let hashes = calculate_cyclonedx_artifact_hashes(&artifact, "1.6").unwrap();
        let by_algorithm = hashes
            .iter()
            .map(|hash| (hash.algorithm.as_str(), hash.content.as_str()))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(
            hashes
                .iter()
                .map(|hash| hash.algorithm.as_str())
                .collect::<Vec<_>>(),
            vec![
                "MD5", "SHA-1", "SHA-256", "SHA-512", "SHA-384", "SHA3-384", "SHA3-256", "SHA3-512"
            ]
        );
        assert_eq!(
            by_algorithm.get("MD5").copied(),
            Some("d41d8cd98f00b204e9800998ecf8427e")
        );
        assert_eq!(
            by_algorithm.get("SHA-1").copied(),
            Some("da39a3ee5e6b4b0d3255bfef95601890afd80709")
        );
        assert_eq!(
            by_algorithm.get("SHA-256").copied(),
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855")
        );
        assert_eq!(
            by_algorithm.get("SHA3-256").copied(),
            Some("a7ffc6f8bf1ed76651c14756a061d662f580ff4de43b49fa82d80a4b80f8434a")
        );
    }

    #[test]
    fn omits_newer_cyclonedx_hash_algorithms_before_spec_1_2() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("artifact.jar");
        std::fs::write(&artifact, b"abc").unwrap();

        let hashes = calculate_cyclonedx_artifact_hashes(&artifact, "1.1").unwrap();
        let algorithms = hashes
            .iter()
            .map(|hash| hash.algorithm.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            algorithms,
            vec!["MD5", "SHA-1", "SHA-256", "SHA-512", "SHA3-256", "SHA3-512"]
        );
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
                "cyclonedx_timestamp_source_policy".to_string(),
                "gradle-substrate-explicit-epoch-ms".to_string(),
            ),
            (
                "cyclonedx_serial_source_policy".to_string(),
                "gradle-substrate-deterministic-identity".to_string(),
            ),
            (
                "cyclonedx_include_metadata_resolution".to_string(),
                "true".to_string(),
            ),
            (
                "cyclonedx_include_build_system".to_string(),
                "false".to_string(),
            ),
            (
                "cyclonedx_include_build_environment".to_string(),
                "false".to_string(),
            ),
            (
                "cyclonedx_include_license_text".to_string(),
                "false".to_string(),
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
        assert_eq!(
            "gradle-substrate-explicit-epoch-ms",
            options.timestamp_source_policy
        );
        assert_eq!(
            "gradle-substrate-deterministic-identity",
            options.serial_source_policy
        );
        assert!(options.include_bom_serial_number);
        assert!(options.include_metadata_resolution);
        assert!(!options.include_build_system);
        assert!(!options.include_build_environment);
        assert!(!options.include_license_text);
        assert!(!options.organizational_entity_present);
        assert!(!options.raw_license_choice_present);
        assert!(options.license_choices.is_empty());
        assert_eq!("", options.build_system_environment_variable);
        assert_eq!("", options.build_system_url);
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
    fn drafts_contract_from_captured_graph_options_and_identity() {
        let graph_json = serde_json::to_vec(&sample_resolution_graph()).unwrap();
        let encoded_graph = base64::engine::general_purpose::STANDARD.encode(graph_json);
        let organizational_entity_json = serde_json::json!({
            "name": "Example Security",
            "url": ["https://security.example.test"],
            "contact": [{
                "name": "Security Team",
                "email": "security@example.test"
            }]
        });
        let encoded_organizational_entity = base64::engine::general_purpose::STANDARD
            .encode(serde_json::to_vec(&organizational_entity_json).unwrap());
        let license_choice_json = serde_json::json!([{
            "license": {
                "id": "Apache-2.0",
                "url": "https://www.apache.org/licenses/LICENSE-2.0"
            }
        }]);
        let encoded_license_choice = base64::engine::general_purpose::STANDARD
            .encode(serde_json::to_vec(&license_choice_json).unwrap());
        let inputs = BTreeMap::from([
            (
                "cyclonedx_resolution_graph_json_b64".to_string(),
                encoded_graph,
            ),
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
            (
                "cyclonedx_project_type".to_string(),
                "APPLICATION".to_string(),
            ),
            (
                "cyclonedx_include_bom_serial_number".to_string(),
                "true".to_string(),
            ),
            (
                "cyclonedx_timestamp_source_policy".to_string(),
                "gradle-substrate-explicit-epoch-ms".to_string(),
            ),
            (
                "cyclonedx_serial_source_policy".to_string(),
                "gradle-substrate-deterministic-identity".to_string(),
            ),
            (
                "cyclonedx_include_build_system".to_string(),
                "true".to_string(),
            ),
            (
                "cyclonedx_include_build_environment".to_string(),
                "false".to_string(),
            ),
            (
                "cyclonedx_include_metadata_resolution".to_string(),
                "false".to_string(),
            ),
            (
                "cyclonedx_include_license_text".to_string(),
                "false".to_string(),
            ),
            (
                "cyclonedx_build_system_environment_variable".to_string(),
                "CI".to_string(),
            ),
            (
                "cyclonedx_build_system_url".to_string(),
                "https://ci.example.test/build/123".to_string(),
            ),
            (
                "cyclonedx_organizational_entity_present".to_string(),
                "true".to_string(),
            ),
            (
                "cyclonedx_organizational_entity_json_b64".to_string(),
                encoded_organizational_entity,
            ),
            ("cyclonedx_license_choice".to_string(), "SPDX".to_string()),
            (
                "cyclonedx_license_choice_json_b64".to_string(),
                encoded_license_choice,
            ),
            (
                "cyclonedx_json_output".to_string(),
                "/tmp/bom.json".to_string(),
            ),
            (
                "cyclonedx_identity_task_path".to_string(),
                ":cyclonedxDirectBom".to_string(),
            ),
        ]);

        let contract =
            draft_contract_from_captured_inputs(&inputs, "build-123", 1_778_595_445_123).unwrap();

        assert_eq!(CONTRACT_SCHEMA, contract.schema);
        assert_eq!("1.6", contract.spec_version);
        assert!(contract.serial_number.starts_with("urn:uuid:"));
        assert_eq!("2026-05-12T14:17:25Z", contract.timestamp);
        assert_eq!("application", contract.root_component.component_type);
        assert_eq!(
            vec![external_reference(
                "build-system",
                "https://ci.example.test/build/123"
            )],
            contract.root_component.external_references
        );
        assert!(contract.external_references.is_empty());
        assert_eq!(
            "Example Security",
            contract.organizational_entity.as_ref().unwrap().name
        );
        assert_eq!(
            vec![CycloneDxLicenseChoice::from_license(CycloneDxLicense {
                id: "Apache-2.0".to_string(),
                name: String::new(),
                url: "https://www.apache.org/licenses/LICENSE-2.0".to_string(),
                text: None,
            })],
            contract.metadata_licenses
        );
        assert!(contract
            .components
            .iter()
            .any(|component| component.bom_ref == "pkg:maven/org.example/lib@1.1?type=jar"));
    }

    #[test]
    fn omits_serial_when_captured_option_disables_bom_serial_number() {
        let graph_json = serde_json::to_vec(&sample_resolution_graph()).unwrap();
        let encoded_graph = base64::engine::general_purpose::STANDARD.encode(graph_json);
        let inputs = BTreeMap::from([
            (
                "cyclonedx_resolution_graph_json_b64".to_string(),
                encoded_graph,
            ),
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
            (
                "cyclonedx_project_type".to_string(),
                "APPLICATION".to_string(),
            ),
            (
                "cyclonedx_include_bom_serial_number".to_string(),
                "false".to_string(),
            ),
            (
                "cyclonedx_timestamp_source_policy".to_string(),
                "gradle-substrate-explicit-epoch-ms".to_string(),
            ),
            (
                "cyclonedx_serial_source_policy".to_string(),
                "omitted".to_string(),
            ),
            (
                "cyclonedx_include_build_system".to_string(),
                "false".to_string(),
            ),
            (
                "cyclonedx_include_build_environment".to_string(),
                "false".to_string(),
            ),
            (
                "cyclonedx_include_metadata_resolution".to_string(),
                "false".to_string(),
            ),
            (
                "cyclonedx_include_license_text".to_string(),
                "false".to_string(),
            ),
            (
                "cyclonedx_json_output".to_string(),
                "/tmp/bom.json".to_string(),
            ),
            (
                "cyclonedx_identity_task_path".to_string(),
                ":cyclonedxDirectBom".to_string(),
            ),
        ]);

        let contract =
            draft_contract_from_captured_inputs(&inputs, "build-123", 1_778_595_445_123).unwrap();
        let json = render_json(&contract).unwrap();
        let xml = render_xml(&contract).unwrap();

        assert_eq!("", contract.serial_number);
        assert!(!json.contains("serialNumber"));
        assert!(!xml.contains("serialNumber="));
        assert!(xml.contains("<bom xmlns=\"http://cyclonedx.org/schema/bom/1.6\" version=\"1\">"));
    }

    #[test]
    fn rejects_captured_contract_without_stable_identity() {
        let graph_json = serde_json::to_vec(&sample_resolution_graph()).unwrap();
        let encoded_graph = base64::engine::general_purpose::STANDARD.encode(graph_json);
        let inputs = BTreeMap::from([
            (
                "cyclonedx_resolution_graph_json_b64".to_string(),
                encoded_graph,
            ),
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
            (
                "cyclonedx_project_type".to_string(),
                "APPLICATION".to_string(),
            ),
            (
                "cyclonedx_timestamp_source_policy".to_string(),
                "gradle-substrate-explicit-epoch-ms".to_string(),
            ),
            (
                "cyclonedx_serial_source_policy".to_string(),
                "omitted".to_string(),
            ),
            (
                "cyclonedx_include_bom_serial_number".to_string(),
                "false".to_string(),
            ),
            (
                "cyclonedx_include_build_system".to_string(),
                "false".to_string(),
            ),
            (
                "cyclonedx_include_build_environment".to_string(),
                "false".to_string(),
            ),
            (
                "cyclonedx_include_metadata_resolution".to_string(),
                "false".to_string(),
            ),
            (
                "cyclonedx_include_license_text".to_string(),
                "false".to_string(),
            ),
            (
                "cyclonedx_json_output".to_string(),
                "/tmp/bom.json".to_string(),
            ),
        ]);

        let err = draft_contract_from_captured_inputs(&inputs, "build-123", 1_778_595_445_123)
            .unwrap_err();

        assert!(err.contains("task identity"));
    }

    #[test]
    fn rejects_captured_contract_with_unsupported_rendering_options() {
        let graph_json = serde_json::to_vec(&sample_resolution_graph()).unwrap();
        let encoded_graph = base64::engine::general_purpose::STANDARD.encode(graph_json);
        let inputs = BTreeMap::from([
            (
                "cyclonedx_resolution_graph_json_b64".to_string(),
                encoded_graph,
            ),
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
            (
                "cyclonedx_project_type".to_string(),
                "APPLICATION".to_string(),
            ),
            (
                "cyclonedx_include_bom_serial_number".to_string(),
                "true".to_string(),
            ),
            (
                "cyclonedx_timestamp_source_policy".to_string(),
                "gradle-substrate-explicit-epoch-ms".to_string(),
            ),
            (
                "cyclonedx_serial_source_policy".to_string(),
                "gradle-substrate-deterministic-identity".to_string(),
            ),
            (
                "cyclonedx_include_build_environment".to_string(),
                "true".to_string(),
            ),
            (
                "cyclonedx_include_build_system".to_string(),
                "false".to_string(),
            ),
            (
                "cyclonedx_include_metadata_resolution".to_string(),
                "false".to_string(),
            ),
            (
                "cyclonedx_include_license_text".to_string(),
                "true".to_string(),
            ),
            (
                "cyclonedx_organizational_entity_present".to_string(),
                "true".to_string(),
            ),
            ("cyclonedx_license_choice".to_string(), "SPDX".to_string()),
            (
                "cyclonedx_external_references".to_string(),
                "https://example.invalid/sbom".to_string(),
            ),
            (
                "cyclonedx_json_output".to_string(),
                "/tmp/bom.json".to_string(),
            ),
            (
                "cyclonedx_identity_task_path".to_string(),
                ":cyclonedxDirectBom".to_string(),
            ),
        ]);

        let err = draft_contract_from_captured_inputs(&inputs, "build-123", 1_778_595_445_123)
            .unwrap_err();

        assert!(!err.contains("include-build-system"));
        assert!(!err.contains("include-build-environment"));
        assert!(err.contains("include-license-text"));
        assert!(err.contains("organizational-entity"));
        assert!(err.contains("license-choice"));
        assert!(!err.contains("build-system-environment-variable"));
        assert!(err.contains("external-references"));
    }

    #[test]
    fn rejects_captured_contract_with_gradle_nondeterministic_identity_policy() {
        let graph_json = serde_json::to_vec(&sample_resolution_graph()).unwrap();
        let encoded_graph = base64::engine::general_purpose::STANDARD.encode(graph_json);
        let inputs = BTreeMap::from([
            (
                "cyclonedx_resolution_graph_json_b64".to_string(),
                encoded_graph,
            ),
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
            (
                "cyclonedx_project_type".to_string(),
                "APPLICATION".to_string(),
            ),
            (
                "cyclonedx_include_bom_serial_number".to_string(),
                "true".to_string(),
            ),
            (
                "cyclonedx_timestamp_source_policy".to_string(),
                "cyclonedx-core-metadata-constructor-now".to_string(),
            ),
            (
                "cyclonedx_serial_source_policy".to_string(),
                "cyclonedx-gradle-random-uuid".to_string(),
            ),
            (
                "cyclonedx_include_build_system".to_string(),
                "false".to_string(),
            ),
            (
                "cyclonedx_include_build_environment".to_string(),
                "false".to_string(),
            ),
            (
                "cyclonedx_include_metadata_resolution".to_string(),
                "false".to_string(),
            ),
            (
                "cyclonedx_include_license_text".to_string(),
                "false".to_string(),
            ),
            (
                "cyclonedx_json_output".to_string(),
                "/tmp/bom.json".to_string(),
            ),
            (
                "cyclonedx_identity_task_path".to_string(),
                ":cyclonedxDirectBom".to_string(),
            ),
        ]);

        let err = draft_contract_from_captured_inputs(&inputs, "build-123", 1_778_595_445_123)
            .unwrap_err();

        assert!(err.contains("timestamp-source-policy"));
        assert!(err.contains("serial-source-policy"));
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
        assert!(err.contains("cyclonedx_include_bom_serial_number"));
        assert!(err.contains("cyclonedx_include_metadata_resolution"));
        assert!(err.contains("cyclonedx_include_build_system"));
        assert!(err.contains("cyclonedx_include_build_environment"));
        assert!(err.contains("cyclonedx_include_license_text"));
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
                root_project_path: ":".to_string(),
                include_metadata_resolution: true,
                external_references: Vec::new(),
                root_vcs_url: String::new(),
            },
        )
        .unwrap();

        assert_eq!(CONTRACT_SCHEMA, contract.schema);
        assert_eq!(
            "pkg:maven/org.example/demo@1.0?project_path=%3A",
            contract.root_component.bom_ref
        );
        assert_eq!(Some(false), contract.root_component.modified);
        assert!(contract
            .components
            .iter()
            .any(|component| component.bom_ref == "pkg:maven/org.example/lib@1.1?type=jar"));
        assert!(contract.components.iter().any(|component| {
            component.bom_ref == "pkg:maven/org.example/lib@1.1?type=jar"
                && component
                    .properties
                    .get("gradle:artifactPath")
                    .map(|path| path == "/repo/lib-1.1.jar")
                    .unwrap_or(false)
                && component
                    .properties
                    .get("gradle:inScopeConfigurations")
                    .map(|scope| scope == "runtimeClasspath")
                    .unwrap_or(false)
        }));
        assert!(contract.dependencies.iter().any(|dependency| {
            dependency.reference == "pkg:maven/org.example/demo@1.0?project_path=%3A"
                && dependency
                    .depends_on
                    .contains(&"pkg:maven/org.example/lib@1.1?type=jar".to_string())
        }));
        assert!(render_json(&contract)
            .unwrap()
            .contains("pkg:maven/org.example/lib@1.1?type=jar"));
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
  <properties>
    <project.start>2024</project.start>
  </properties>
  <name>Example Lib</name>
  <description>
    Useful &amp;
    small
  </description>
  <inceptionYear>${project.start}</inceptionYear>
  <url>https://example.test/lib</url>
  <organization>
    <name>Example Foundation</name>
    <url>https://example.test</url>
  </organization>
  <developers>
    <developer>
      <id>ada-${project.start}</id>
      <name>Ada ${project.start}</name>
      <email>ada-${project.start}@example.test</email>
      <url>https://ada.example.test/${project.start}</url>
      <organization>Example Devs</organization>
      <organizationUrl>https://devs.example.test/${project.start}</organizationUrl>
      <roles>
        <role>maintainer-${project.start}</role>
      </roles>
      <timezone>+1</timezone>
    </developer>
    <developer>
      <name>Linus</name>
    </developer>
  </developers>
  <contributors>
    <contributor>
      <name>Grace</name>
      <email>grace@example.test</email>
      <url>https://grace.example.test</url>
      <organization>Example Contributors</organization>
      <organizationUrl>https://contributors.example.test</organizationUrl>
      <roles>
        <role>docs</role>
      </roles>
      <timezone>-5</timezone>
    </contributor>
  </contributors>
  <ciManagement>
    <url>https://ci.example.test/lib</url>
  </ciManagement>
  <distributionManagement>
    <downloadUrl>https://downloads.example.test/lib</downloadUrl>
    <repository>
      <url>scm:svn:https://svn.example.test/ignored</url>
    </repository>
    <site>
      <url>https://site.example.test/lib</url>
    </site>
  </distributionManagement>
  <issueManagement>
    <system>GitHub Issues</system>
    <url>https://issues.example.test/lib</url>
  </issueManagement>
  <mailingLists>
    <mailingList>
      <archive>https://lists.example.test/lib</archive>
      <subscribe>mailto:lib-subscribe@example.test</subscribe>
      <unsubscribe>mailto:lib-unsubscribe@example.test</unsubscribe>
      <post>mailto:lib@example.test</post>
    </mailingList>
  </mailingLists>
  <scm>
    <url>https://git.example.test/lib</url>
    <connection>scm:git:https://git.example.test/lib.git</connection>
    <developerConnection>scm:git:ssh://git@example.test/lib.git</developerConnection>
  </scm>
  <licenses>
    <license>
      <name>Apache-2.0</name>
      <distribution>repo-${project.start}</distribution>
      <comments>Use with notice ${project.start}</comments>
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
                root_project_path: ":".to_string(),
                include_metadata_resolution: true,
                external_references: Vec::new(),
                root_vcs_url: String::new(),
            },
        )
        .unwrap();

        let component = contract
            .components
            .iter()
            .find(|component| component.bom_ref == "pkg:maven/org.example/lib@1.1?type=jar")
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
            Some(&"2024".to_string()),
            component.properties.get("maven:pomInceptionYear")
        );
        assert_eq!(
            Some(&"Ada 2024,Linus".to_string()),
            component.properties.get("maven:pomDevelopers")
        );
        assert_eq!(
            Some(&"ada-2024".to_string()),
            component.properties.get("maven:pomDeveloperIds")
        );
        assert_eq!(
            Some(&"ada-2024@example.test".to_string()),
            component.properties.get("maven:pomDeveloperEmails")
        );
        assert_eq!(
            Some(&"https://ada.example.test/2024".to_string()),
            component.properties.get("maven:pomDeveloperUrls")
        );
        assert_eq!(
            Some(&"Example Devs".to_string()),
            component.properties.get("maven:pomDeveloperOrganizations")
        );
        assert_eq!(
            Some(&"https://devs.example.test/2024".to_string()),
            component
                .properties
                .get("maven:pomDeveloperOrganizationUrls")
        );
        assert_eq!(
            Some(&"maintainer-2024".to_string()),
            component.properties.get("maven:pomDeveloperRoles")
        );
        assert_eq!(
            Some(&"+1".to_string()),
            component.properties.get("maven:pomDeveloperTimezones")
        );
        assert_eq!(
            Some(&"Grace".to_string()),
            component.properties.get("maven:pomContributors")
        );
        assert_eq!(
            Some(&"grace@example.test".to_string()),
            component.properties.get("maven:pomContributorEmails")
        );
        assert_eq!(
            Some(&"https://grace.example.test".to_string()),
            component.properties.get("maven:pomContributorUrls")
        );
        assert_eq!(
            Some(&"Example Contributors".to_string()),
            component
                .properties
                .get("maven:pomContributorOrganizations")
        );
        assert_eq!(
            Some(&"https://contributors.example.test".to_string()),
            component
                .properties
                .get("maven:pomContributorOrganizationUrls")
        );
        assert_eq!(
            Some(&"docs".to_string()),
            component.properties.get("maven:pomContributorRoles")
        );
        assert_eq!(
            Some(&"-5".to_string()),
            component.properties.get("maven:pomContributorTimezones")
        );
        assert_eq!(
            Some(&"repo-2024".to_string()),
            component.properties.get("maven:pomLicenseDistributions")
        );
        assert_eq!(
            Some(&"Use with notice 2024".to_string()),
            component.properties.get("maven:pomLicenseComments")
        );
        assert_eq!("Useful & small", component.description);
        assert_eq!("Example Foundation", component.publisher);
        assert!(component
            .external_references
            .contains(&external_reference("website", "https://example.test")));
        assert!(component.external_references.contains(&external_reference(
            "build-system",
            "https://ci.example.test/lib"
        )));
        assert!(component.external_references.contains(&external_reference(
            "distribution",
            "https://downloads.example.test/lib"
        )));
        assert!(!component.external_references.contains(&external_reference(
            "distribution",
            "scm:svn:https://svn.example.test/ignored"
        )));
        assert!(component.external_references.contains(&external_reference(
            "distribution",
            "https://site.example.test/lib"
        )));
        assert!(component.external_references.contains(&external_reference(
            "issue-tracker",
            "https://issues.example.test/lib"
        )));
        assert!(component.external_references.contains(&external_reference(
            "mailing-list",
            "https://lists.example.test/lib"
        )));
        assert_eq!(
            1,
            component
                .external_references
                .iter()
                .filter(|reference| reference.reference_type == "mailing-list")
                .count()
        );
        assert!(!component.external_references.contains(&external_reference(
            "mailing-list",
            "mailto:lib-subscribe@example.test"
        )));
        assert!(!component.external_references.contains(&external_reference(
            "mailing-list",
            "mailto:lib-unsubscribe@example.test"
        )));
        assert!(!component.external_references.contains(&external_reference(
            "mailing-list",
            "mailto:lib@example.test"
        )));
        assert!(component
            .external_references
            .contains(&external_reference("vcs", "https://git.example.test/lib")));
        assert!(!component.external_references.contains(&external_reference(
            "vcs",
            "scm:git:https://git.example.test/lib.git"
        )));
        assert!(!component.external_references.contains(&external_reference(
            "vcs",
            "scm:git:ssh://git@example.test/lib.git"
        )));
        assert_eq!(
            vec![CycloneDxLicenseChoice::from_license(CycloneDxLicense {
                id: "Apache-2.0".to_string(),
                name: String::new(),
                url: String::new(),
                text: None,
            })],
            component.licenses
        );
    }

    #[test]
    fn interpolates_maven_properties_in_pom_metadata() {
        let metadata = parse_pom_component_metadata(
            r#"
<project>
  <properties>
    <display.name>Interpolated Lib</display.name>
    <site.url>https://metadata.example.test/${project.groupId}/${project.artifactId}/${project.version}</site.url>
    <license.name>The Apache Software License, Version 2.0</license.name>
  </properties>
  <parent>
    <groupId>org.parent</groupId>
    <artifactId>parent</artifactId>
    <version>9.9</version>
  </parent>
  <artifactId>interpolated-lib</artifactId>
  <name>${display.name}</name>
  <description>${display.name} docs at ${site.url}</description>
  <url>${site.url}</url>
  <organization>
    <name>${display.name} Foundation</name>
    <url>${site.url}/org</url>
  </organization>
  <scm>
    <url>${site.url}/git</url>
  </scm>
  <licenses>
    <license>
      <name>${license.name}</name>
    </license>
  </licenses>
</project>
"#,
        )
        .metadata;

        assert_eq!("Interpolated Lib", metadata.name);
        assert_eq!(
            "Interpolated Lib docs at https://metadata.example.test/org.parent/interpolated-lib/9.9",
            metadata.description
        );
        assert_eq!(
            "https://metadata.example.test/org.parent/interpolated-lib/9.9",
            metadata.url
        );
        assert_eq!("Interpolated Lib Foundation", metadata.publisher);
        assert!(metadata.external_references.contains(&external_reference(
            "website",
            "https://metadata.example.test/org.parent/interpolated-lib/9.9/org"
        )));
        assert!(metadata.external_references.contains(&external_reference(
            "vcs",
            "https://metadata.example.test/org.parent/interpolated-lib/9.9/git"
        )));
        assert_eq!(
            vec![CycloneDxLicenseChoice::from_license(CycloneDxLicense {
                id: "Apache-2.0".to_string(),
                name: String::new(),
                url: String::new(),
                text: None,
            })],
            metadata.licenses
        );
    }

    #[test]
    fn reads_cdata_pom_metadata_fields() {
        let metadata = parse_pom_component_metadata(
            r#"
<project>
  <name><![CDATA[CDATA Lib]]></name>
  <description><![CDATA[CDATA description with <xml-ish> text]]></description>
  <organization>
    <name><![CDATA[CDATA Org]]></name>
    <url><![CDATA[https://cdata.example.test/org]]></url>
  </organization>
  <licenses>
    <license>
      <name><![CDATA[MIT]]></name>
    </license>
  </licenses>
</project>
"#,
        )
        .metadata;

        assert_eq!("CDATA Lib", metadata.name);
        assert_eq!(
            "CDATA description with <xml-ish> text",
            metadata.description
        );
        assert_eq!("CDATA Org", metadata.publisher);
        assert!(metadata.external_references.contains(&external_reference(
            "website",
            "https://cdata.example.test/org"
        )));
        assert_eq!(
            vec![CycloneDxLicenseChoice::from_license(CycloneDxLicense {
                id: "MIT".to_string(),
                name: String::new(),
                url: String::new(),
                text: None,
            })],
            metadata.licenses
        );
    }

    #[test]
    fn inherits_local_parent_pom_metadata_when_child_is_incomplete() {
        let dir = tempfile::tempdir().unwrap();
        let module_dir = dir.path().join("module");
        std::fs::create_dir(&module_dir).unwrap();
        let parent_pom = dir.path().join("pom.xml");
        let jar_path = module_dir.join("lib-1.1.jar");
        let pom_path = module_dir.join("lib-1.1.pom");
        std::fs::write(&jar_path, b"not-a-real-jar").unwrap();
        std::fs::write(
            &parent_pom,
            r#"
<project>
  <description>Inherited description</description>
  <organization>
    <name>Parent Publisher</name>
    <url>https://parent.example.test</url>
  </organization>
  <developers>
    <developer>
      <name>Parent Developer</name>
      <email>parent@example.test</email>
    </developer>
  </developers>
  <licenses>
    <license>
      <name>MIT</name>
      <comments>Parent license comment</comments>
    </license>
  </licenses>
  <scm>
    <url>https://git.parent.example.test/root</url>
  </scm>
</project>
"#,
        )
        .unwrap();
        std::fs::write(
            &pom_path,
            r#"
<project>
  <parent>
    <groupId>org.example</groupId>
    <artifactId>parent</artifactId>
    <version>1.0</version>
    <relativePath>../pom.xml</relativePath>
  </parent>
  <name>Child Lib</name>
  <inceptionYear>2026</inceptionYear>
  <url>https://child.example.test/lib</url>
  <developers>
    <developer>
      <name>Child Developer</name>
      <email>child@example.test</email>
    </developer>
  </developers>
  <licenses>
    <license>
      <name>Apache-2.0</name>
      <comments>Child license comment</comments>
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
                root_project_path: ":".to_string(),
                include_metadata_resolution: true,
                external_references: Vec::new(),
                root_vcs_url: String::new(),
            },
        )
        .unwrap();

        let component = contract
            .components
            .iter()
            .find(|component| component.bom_ref == "pkg:maven/org.example/lib@1.1?type=jar")
            .unwrap();

        assert_eq!(
            Some(&"Child Lib".to_string()),
            component.properties.get("maven:pomName")
        );
        assert_eq!("", component.description);
        assert_eq!("", component.publisher);
        assert_eq!(
            vec![CycloneDxLicenseChoice::from_license(CycloneDxLicense {
                id: "Apache-2.0".to_string(),
                name: String::new(),
                url: String::new(),
                text: None,
            })],
            component.licenses
        );
        assert_eq!(
            Some(&"2026".to_string()),
            component.properties.get("maven:pomInceptionYear")
        );
        assert_eq!(
            Some(&"Child Developer".to_string()),
            component.properties.get("maven:pomDevelopers")
        );
        assert_eq!(
            Some(&"child@example.test".to_string()),
            component.properties.get("maven:pomDeveloperEmails")
        );
        assert_eq!(
            Some(&"Child license comment".to_string()),
            component.properties.get("maven:pomLicenseComments")
        );
        assert!(!component.external_references.contains(&external_reference(
            "website",
            "https://parent.example.test"
        )));
        assert!(!component.external_references.contains(&external_reference(
            "vcs",
            "https://git.parent.example.test/root"
        )));
        assert_eq!(
            Some(&"https://child.example.test/lib".to_string()),
            component.properties.get("maven:pomUrl")
        );
    }

    #[test]
    fn resolves_common_pom_license_names_and_urls_to_spdx_ids() {
        let metadata = parse_pom_component_metadata(
            r#"
<project>
  <licenses>
    <license>
      <name>The Apache Software License, Version 2.0</name>
      <url>http://www.apache.org/licenses/LICENSE-2.0.txt</url>
    </license>
    <license>
      <url>https://opensource.org/license/mit/</url>
    </license>
    <license>
      <name>BSD License</name>
      <url>http://www.antlr.org/license.html</url>
    </license>
    <license>
      <name>Eclipse Distribution License - v 1.0</name>
      <url>http://www.eclipse.org/org/documents/edl-v10.php</url>
    </license>
    <license>
      <name>Custom License</name>
      <url>https://licenses.example.test/custom</url>
    </license>
    <license>
      <name>The GNU General Public License, v2 with Universal FOSS Exception, v1.0</name>
    </license>
    <license>
      <name>LGPL-2.1-or-later</name>
      <url>https://www.gnu.org/licenses/old-licenses/lgpl-2.1.txt</url>
    </license>
    <license>
      <name>LGPL-2.1+</name>
      <url>http://www.gnu.org/licenses/old-licenses/lgpl-2.1.txt</url>
    </license>
  </licenses>
</project>
"#,
        )
        .metadata;
        assert!(metadata
            .licenses
            .contains(&CycloneDxLicenseChoice::from_license(CycloneDxLicense {
                id: "Apache-2.0".to_string(),
                name: String::new(),
                url: String::new(),
                text: None,
            })));
        assert!(metadata
            .licenses
            .contains(&CycloneDxLicenseChoice::from_license(CycloneDxLicense {
                id: "MIT".to_string(),
                name: String::new(),
                url: String::new(),
                text: None,
            })));
        assert!(metadata
            .licenses
            .contains(&CycloneDxLicenseChoice::from_license(CycloneDxLicense {
                id: "BSD-3-Clause".to_string(),
                name: String::new(),
                url: String::new(),
                text: None,
            })));
        assert!(metadata
            .licenses
            .contains(&CycloneDxLicenseChoice::from_license(CycloneDxLicense {
                id: "GPL-2.0-only".to_string(),
                name: String::new(),
                url: String::new(),
                text: None,
            })));
        assert!(metadata
            .licenses
            .contains(&CycloneDxLicenseChoice::from_license(CycloneDxLicense {
                id: "LGPL-2.1-or-later".to_string(),
                name: String::new(),
                url: "https://www.gnu.org/licenses/old-licenses/lgpl-2.1-standalone.html"
                    .to_string(),
                text: None,
            })));
        assert!(metadata
            .licenses
            .contains(&CycloneDxLicenseChoice::from_license(CycloneDxLicense {
                id: "LGPL-2.1+".to_string(),
                name: String::new(),
                url: "https://www.gnu.org/licenses/old-licenses/lgpl-2.1-standalone.html"
                    .to_string(),
                text: None,
            })));
        assert!(metadata
            .licenses
            .contains(&CycloneDxLicenseChoice::from_license(CycloneDxLicense {
                id: String::new(),
                name: "Custom License".to_string(),
                url: "https://licenses.example.test/custom".to_string(),
                text: None,
            })));
    }

    #[test]
    fn inherits_parent_pom_metadata_from_gradle_module_cache_coordinates() {
        let dir = tempfile::tempdir().unwrap();
        let files_root = dir.path().join("modules-2").join("files-2.1");
        let parent_dir = files_root
            .join("org.parent")
            .join("parent-bom")
            .join("1.0")
            .join("parent-hash");
        let child_dir = files_root
            .join("org.child")
            .join("child-lib")
            .join("2.0")
            .join("child-hash");
        std::fs::create_dir_all(&parent_dir).unwrap();
        std::fs::create_dir_all(&child_dir).unwrap();
        std::fs::write(
            parent_dir.join("parent-bom-1.0.pom"),
            r#"
<project>
  <description>Repository parent description</description>
  <organization>
    <name>Repository Parent Publisher</name>
  </organization>
  <licenses>
    <license>
      <name>MIT License</name>
    </license>
  </licenses>
</project>
"#,
        )
        .unwrap();
        let child_pom = child_dir.join("child-lib-2.0.pom");
        std::fs::write(
            &child_pom,
            r#"
<project>
  <parent>
    <groupId>org.parent</groupId>
    <artifactId>parent-bom</artifactId>
    <version>1.0</version>
    <relativePath>../missing-parent.xml</relativePath>
  </parent>
  <name>Child From Cache</name>
</project>
"#,
        )
        .unwrap();

        let metadata = read_effective_pom_component_metadata(
            &child_pom,
            &std::fs::read_to_string(&child_pom).unwrap(),
            0,
        );

        assert_eq!("Child From Cache", metadata.name);
        assert_eq!("", metadata.description);
        assert_eq!("", metadata.publisher);
        assert_eq!(
            vec![CycloneDxLicenseChoice::from_license(CycloneDxLicense {
                id: "MIT".to_string(),
                name: String::new(),
                url: String::new(),
                text: None,
            })],
            metadata.licenses
        );
    }

    #[test]
    fn skips_pom_metadata_when_metadata_resolution_is_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let jar_path = dir.path().join("lib-1.1.jar");
        let pom_path = dir.path().join("lib-1.1.pom");
        std::fs::write(&jar_path, b"not-a-real-jar").unwrap();
        std::fs::write(
            &pom_path,
            r#"
<project>
  <name>Example Lib</name>
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
                serial_number: "urn:uuid:00000000-0000-0000-0000-000000000007".to_string(),
                timestamp: "2026-05-12T12:15:00Z".to_string(),
                root_group: "org.example".to_string(),
                root_name: "demo".to_string(),
                root_version: "1.0".to_string(),
                root_component_type: "application".to_string(),
                root_project_path: ":".to_string(),
                include_metadata_resolution: false,
                root_vcs_url: String::new(),
                external_references: Vec::new(),
            },
        )
        .unwrap();

        let component = contract
            .components
            .iter()
            .find(|component| component.bom_ref == "pkg:maven/org.example/lib@1.1?type=jar")
            .unwrap();
        assert_eq!(
            Some(&jar_path.to_string_lossy().to_string()),
            component.properties.get("gradle:artifactPath")
        );
        assert!(!component.properties.contains_key("maven:pomName"));
        assert!(component.licenses.is_empty());
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
                root_project_path: ":".to_string(),
                include_metadata_resolution: true,
                root_vcs_url: String::new(),
                external_references: Vec::new(),
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
    fn omits_outgoing_dependency_edges_from_pom_artifacts() {
        let mut graph = sample_resolution_graph();
        graph.configurations[0]
            .components
            .push(CycloneDxResolvedComponent {
                id: "org.example:managed-bom:1.0".to_string(),
                group: "org.example".to_string(),
                module: "managed-bom".to_string(),
                version: "1.0".to_string(),
                project_path: String::new(),
                artifact_path: String::new(),
                artifact_type: String::new(),
                artifact_extension: String::new(),
                artifact_classifier: String::new(),
                in_scope_configurations: vec!["runtimeClasspath".to_string()],
            });
        graph.configurations[0]
            .dependencies
            .push(CycloneDxResolvedDependency {
                from: "org.example:managed-bom:1.0".to_string(),
                requested: "org.example:lib:1.1".to_string(),
                to: "org.example:lib:1.1".to_string(),
            });

        let contract = draft_contract_from_resolution_graph(
            &graph,
            CycloneDxDraftOptions {
                spec_version: "1.6".to_string(),
                serial_number: "urn:uuid:00000000-0000-0000-0000-000000000009".to_string(),
                timestamp: "2026-05-12T12:45:00Z".to_string(),
                root_group: "org.example".to_string(),
                root_name: "demo".to_string(),
                root_version: "1.0".to_string(),
                root_component_type: "application".to_string(),
                root_project_path: ":".to_string(),
                include_metadata_resolution: false,
                external_references: Vec::new(),
                root_vcs_url: String::new(),
            },
        )
        .unwrap();

        let bom_dependency = contract
            .dependencies
            .iter()
            .find(|dependency| {
                dependency.reference == "pkg:maven/org.example/managed-bom@1.0?type=pom"
            })
            .unwrap();
        assert!(bom_dependency.depends_on.is_empty());
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
                root_project_path: ":".to_string(),
                external_references: Vec::new(),
                root_vcs_url: String::new(),
            },
        )
        .unwrap();

        assert_eq!(
            "pkg:maven/org.example/aggregate@1.0?project_path=%3A",
            aggregate.root_component.bom_ref
        );
        assert_eq!(Some(false), aggregate.root_component.modified);
        assert_eq!(
            vec![
                "pkg:maven/org.example/a@1.0.0?type=jar".to_string(),
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
                root_project_path: ":".to_string(),
                root_vcs_url: String::new(),
                external_references: Vec::new(),
            },
        )
        .unwrap_err();
        assert!(err.contains("conflicting component"));
    }

    #[test]
    fn deterministic_serial_number_produces_identical_output_for_same_inputs() {
        let parts = &[
            "build-123",
            ":cyclonedxDirectBom",
            "org.example",
            "demo",
            "1.0",
            "1.6",
        ];
        let serial_a = deterministic_serial_number(parts);
        let serial_b = deterministic_serial_number(parts);
        assert_eq!(
            serial_a, serial_b,
            "same inputs must yield identical serial"
        );
        assert!(serial_a.starts_with("urn:uuid:"));
        assert_eq!(serial_a.len(), "urn:uuid:".len() + 36);
    }

    #[test]
    fn deterministic_serial_number_produces_different_output_for_different_inputs() {
        let a = deterministic_serial_number(&["build-123", ":task", "g", "n", "1.0", "1.6"]);
        let b = deterministic_serial_number(&["build-456", ":task", "g", "n", "1.0", "1.6"]);
        let c = deterministic_serial_number(&["build-123", ":other", "g", "n", "1.0", "1.6"]);
        assert_ne!(a, b, "different build_id must yield different serial");
        assert_ne!(a, c, "different task_path must yield different serial");
        assert_ne!(b, c, "two different input sets must not collide by chance");
    }

    #[test]
    fn deterministic_serial_number_format_matches_uuid_v5_conventions() {
        let serial = deterministic_serial_number(&["a", "b", "c", "d", "e", "1.6"]);
        let uuid = serial.trim_start_matches("urn:uuid:");
        let bytes: Vec<u8> = uuid
            .split('-')
            .flat_map(|seg| {
                (0..seg.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&seg[i..i + 2], 16).unwrap())
            })
            .collect();
        assert_eq!(bytes.len(), 16);
        assert_eq!(
            (bytes[6] & 0xf0) >> 4,
            5,
            "version nibble must be 5 (name-based SHA-1/v5 style)"
        );
        assert_eq!(
            (bytes[8] & 0xc0) >> 6,
            2,
            "variant nibble must match RFC 4122 (10xx)"
        );
    }

    #[test]
    fn identity_policy_produces_same_serial_across_instances() {
        let p1 = CycloneDxIdentityPolicy {
            build_id: "build-123".to_string(),
            task_path: ":cyclonedxDirectBom".to_string(),
            root_group: "org.example".to_string(),
            root_name: "demo".to_string(),
            root_version: "1.0".to_string(),
            schema_version: "1.6".to_string(),
            timestamp_ms: 1_778_595_445_123,
        };
        let p2 = CycloneDxIdentityPolicy {
            build_id: "build-123".to_string(),
            task_path: ":cyclonedxDirectBom".to_string(),
            root_group: "org.example".to_string(),
            root_name: "demo".to_string(),
            root_version: "1.0".to_string(),
            schema_version: "1.6".to_string(),
            timestamp_ms: 1_778_595_445_123,
        };
        assert_eq!(
            p1.serial_number(),
            p2.serial_number(),
            "two identity-policy instances with identical inputs must yield the same serial"
        );
    }

    #[test]
    fn timestamp_from_epoch_millis_renders_correct_utc_iso8601() {
        assert_eq!(
            timestamp_from_epoch_millis(0).unwrap(),
            "1970-01-01T00:00:00Z"
        );
        assert_eq!(
            timestamp_from_epoch_millis(1_778_595_445_123).unwrap(),
            "2026-05-12T14:17:25Z"
        );
        assert_eq!(
            timestamp_from_epoch_millis(1_000).unwrap(),
            "1970-01-01T00:00:01Z"
        );
    }

    #[test]
    fn timestamp_from_epoch_millis_is_stable_for_same_input() {
        let ms = 1_778_595_445_123_i64;
        assert_eq!(
            timestamp_from_epoch_millis(ms).unwrap(),
            timestamp_from_epoch_millis(ms).unwrap()
        );
    }

    #[test]
    fn identity_policy_timestamp_matches_timestamp_from_epoch_millis() {
        let policy = CycloneDxIdentityPolicy {
            build_id: "build-1".to_string(),
            task_path: ":task".to_string(),
            root_group: "".to_string(),
            root_name: "app".to_string(),
            root_version: "2.0".to_string(),
            schema_version: "1.6".to_string(),
            timestamp_ms: 1_778_595_445_123,
        };
        assert_eq!(
            policy.timestamp().unwrap(),
            timestamp_from_epoch_millis(1_778_595_445_123).unwrap()
        );
    }

    #[test]
    fn reject_unsupported_captured_options_accepts_deterministic_policies() {
        let options = CycloneDxCapturedTaskOptions {
            spec_version: "1.6".to_string(),
            root_group: "org.example".to_string(),
            root_name: "demo".to_string(),
            root_version: "1.0".to_string(),
            root_component_type: "library".to_string(),
            timestamp_source_policy: "gradle-substrate-explicit-epoch-ms".to_string(),
            serial_source_policy: "gradle-substrate-deterministic-identity".to_string(),
            include_bom_serial_number: true,
            include_metadata_resolution: true,
            include_build_system: false,
            include_build_environment: false,
            include_license_text: false,
            organizational_entity_present: false,
            organizational_entity: None,
            raw_license_choice_present: false,
            license_choices: Vec::new(),
            build_system_environment_variable: String::new(),
            build_system_url: String::new(),
            raw_external_references_present: false,
            external_references: Vec::new(),
            root_vcs_url: String::new(),
            json_output: "/tmp/bom.json".to_string(),
            xml_output: String::new(),
        };
        reject_unsupported_captured_options(&options).unwrap();
    }

    #[test]
    fn reject_unsupported_captured_options_rejects_upstream_random_timestamp_policy() {
        let mut options = make_all_deterministic_options();
        options.timestamp_source_policy = "upstream-random".to_string();
        let err = reject_unsupported_captured_options(&options).unwrap_err();
        assert!(err.contains("timestamp-source-policy"));
    }

    #[test]
    fn reject_unsupported_captured_options_rejects_cyclonedx_gradle_random_serial_policy() {
        let mut options = make_all_deterministic_options();
        options.serial_source_policy = "cyclonedx-gradle-random-uuid".to_string();
        let err = reject_unsupported_captured_options(&options).unwrap_err();
        assert!(err.contains("serial-source-policy"));
    }

    #[test]
    fn reject_unsupported_captured_options_rejects_both_nondeterministic_policies() {
        let mut options = make_all_deterministic_options();
        options.timestamp_source_policy = "cyclonedx-core-metadata-constructor-now".to_string();
        options.serial_source_policy = "upstream-random".to_string();
        let err = reject_unsupported_captured_options(&options).unwrap_err();
        assert!(err.contains("timestamp-source-policy"));
        assert!(err.contains("serial-source-policy"));
    }

    #[test]
    fn reject_unsupported_captured_options_rejects_omitted_serial_with_include_bom_serial() {
        let mut options = make_all_deterministic_options();
        options.serial_source_policy = "omitted".to_string();
        let err = reject_unsupported_captured_options(&options).unwrap_err();
        assert!(err.contains("serial-source-policy"));
    }

    #[test]
    fn reject_unsupported_captured_options_accepts_omitted_serial_when_bom_serial_false() {
        let mut options = make_all_deterministic_options();
        options.include_bom_serial_number = false;
        options.serial_source_policy = "omitted".to_string();
        reject_unsupported_captured_options(&options).unwrap();
    }

    #[test]
    fn reject_unsupported_captured_options_rejects_non_omitted_serial_when_bom_serial_false() {
        let mut options = make_all_deterministic_options();
        options.include_bom_serial_number = false;
        options.serial_source_policy = "gradle-substrate-deterministic-identity".to_string();
        let err = reject_unsupported_captured_options(&options).unwrap_err();
        assert!(err.contains("serial-source-policy"));
    }

    fn make_all_deterministic_options() -> CycloneDxCapturedTaskOptions {
        CycloneDxCapturedTaskOptions {
            spec_version: "1.6".to_string(),
            root_group: "org.example".to_string(),
            root_name: "demo".to_string(),
            root_version: "1.0".to_string(),
            root_component_type: "library".to_string(),
            timestamp_source_policy: "gradle-substrate-explicit-epoch-ms".to_string(),
            serial_source_policy: "gradle-substrate-deterministic-identity".to_string(),
            include_bom_serial_number: true,
            include_metadata_resolution: true,
            include_build_system: false,
            include_build_environment: false,
            include_license_text: false,
            organizational_entity_present: false,
            organizational_entity: None,
            raw_license_choice_present: false,
            license_choices: Vec::new(),
            build_system_environment_variable: String::new(),
            build_system_url: String::new(),
            raw_external_references_present: false,
            external_references: Vec::new(),
            root_vcs_url: String::new(),
            json_output: "/tmp/bom.json".to_string(),
            xml_output: String::new(),
        }
    }
}
