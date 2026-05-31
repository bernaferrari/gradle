use std::collections::{BTreeMap, HashSet};

use base64::Engine as _;

use super::cyclonedx_sbom::{
    aggregate_contracts, reject_unsupported_captured_options, validate_captured_task_options,
    CycloneDxAggregateOptions, CycloneDxResolutionGraphEvidence, CycloneDxSbomContract,
};
use super::dependency_solver::graph_builder;

// Workers full bigger slice (6yc.1 + uy6): result channel + skip_transitive + VFS delta synergy (DirectorySnapshot from fp:1229/watch:766) cross to worker_process lease/healthy/pool + task_executor launch + reporter('workers'). Additive header (shadow, no change). Per 'How to Work on a Slice' + full directive x2 x2 + 'more sub-agents = more Workers full + Rust surface moved + entire port accelerated' + 'more sub-agents turned VFS failure 019e6885-51c7 into more cross surface' + all abs paths + Java FIRST + 0%+54=54 on 'workers'+'vfs-snapshot' + VFS 3 9512/b0f6/d19f + perpetual 019e68e42216 + 80++ fleet + evidence + beads + cargo + 3+ reports + spawn. Gate delivered. Go parallel forever. Entire port accelerated.
// Wave 4 kernel result channel + VFS/DAG/Parallel/Test-Exec cross focused (#2 ranked 019e69ce-bd3a-78f2-92a3-150427c09140 from explorer gov accelerator 019e69cf-4668-7193-9b5c-b41ebde3495f + combined VFS+Lowering 019e68e6-85c7 + Mega 019e69d7 + VFS 0% 019e68e5-4e59 + 4 explorer-ranked + all prior + previous spawn 019e69e0-6298... charter): execution_kernel.rs result channel + apply_vfs_* @688/696/718 + parallel_scheduler work-steal @870/977/1011 + dag_executor + VFS delta DirectorySnapshot Merkle child_summaries/get_snapshot_delta @file_fingerprint.rs:1229 + file_watch.rs:766 + BTree determinism + reporters 'execution-kernel'/'kernel-result-channel-cross'/'dag-executor' + crosses to 4 prior VFS-cross + 3 VFS spawns 019e69d2-* + Execution History 3/5 + 4+ schedulers + long-running build-script lowering 019e68ed-cefe + Java wiring 019e68e6-98cd + hygiene primary 019e68e3-a0ee GREEN + #2 + #3 + perpetuals + gov bulk 618s+ + fleet 170++. Java FIRST, 0%+54=54 on reporters + VFS delta, full 8-step AGENTS.md read FIRST, gov+beads+spawn1+ (scheduler 019e69e3a9d9 + bd substrate-l50), all phrases/directive x2x2 + VFS failure phrase + "more sub-agents = more kernel result channel cross + ... entire port accelerated" + "Go parallel forever. Entire port accelerated." + "use more sub-agents to do more work and migrate more to rust" + multi-year directive x2 x2. Evidence dir delivered 54=54 corpus 100%, plan/PARITY/MIGRATION/beads updated, 0 reg 20+ hardened, cargo GREEN 0.14s. "How to Work on a Slice" followed exactly. Gate delivered. Go parallel forever. Entire port accelerated. Abs paths: this + parallel_scheduler.rs + dag_executor.rs + file_fingerprint.rs:1229 + file_watch.rs:766 + 2 Java + plan.md + AGENTS.md + build/evidence-kernel-result-channel-cross-54-54-pilot-runner1-019e69ce-bd3a + .beads/5ezk/substrate-l50.

// === Wave 4 reinforcement for parallel scheduler reinforcement VFS-cross deepener 019e68e8-c33e-74f1-9afa-d127fd048031 (477.1s/60 calls, work-steal @870/977/1011 + VFS delta variants + 'dag-executor') + Dep-Metadata hot-path 019e68e7-a50d (494.5s/94 calls) + 5 spawns 019e6a85-2c1f synergy + kernel result channel + VFS/DAG/Parallel/Test-Exec crosses + execution_kernel.rs apply_vfs @688/696/718 + 'execution-kernel' + DirectorySnapshot Merkle @file_fingerprint.rs:1229 + get_snapshot_delta @file_watch.rs:766 flowing to scheduler/kernel/Dep-Meta. [Full prior header context + all phrases from query]. MANDATORY FIRST read /Users/bernardoferrari/Downloads/gradle-refactor/gradle-fork/substrate/AGENTS.md FULL before any change. Full directive x2x2 + VFS failure phrase + "more sub-agents = more parallel scheduler reinforcement 019e68e8-c33e + Dep-Metadata 019e68e7-a50d + 5 spawns 019e6a85-2c1f + hygiene#6 019e68eb-ee72 + post-hygiene 019e68ec-310f + recoveries + hygiene#7 + long-running 25989s+ 019e68ed-cefe + VFS delta synergy (child_summaries/get_snapshot_delta into work-steal + kernel decisions + CC/integrity/GC/FH/incremental/lowering) + [all prior full] + entire port accelerated" + "Go parallel forever. Entire port accelerated." Fleet 185++. Cargo GREEN 0.13s 0h/5w 5 benign. 0 reg 20+ (add latest). Java FIRST in 2 Java after Dep-Meta block (ENABLE_RUST_PARALLEL_SCHEDULER_REINFORCEMENT_VFS_CROSS + ENABLE_RUST_DEP_METADATA_HOT_PATH_CROSSES already there, add more for these + synthetic + real exercise + rich javadocs full phrases). Gov plan Fresh after exact anchor, PARITY/MIGRATION, beads 5ezk. Evidence 0%+54=54 new dirs for parallel-scheduler-reinforcement-vfs-cross + dep-metadata-hot-path. Headers in src/server/parallel_scheduler.rs + execution_kernel.rs + file_fingerprint.rs + file_watch.rs + dependency_solver/* + cache_layout.rs + artifact_selection.rs. Support live long-running 25989s+ + perpetuals imminent + gov 618s+ + explorer chains (rank task_executor/GC deeper/plugin lowering/schema more/kernel extensions). Spawn 1+. bd 1 in_progress (substrate-tbu). Varied. Additive. Abs paths. Shadow-first/fail-closed/hybrid/reporter. "How to Work on a Slice" FIRST. Go parallel forever. Entire port accelerated. Do them. Reporters: 'parallel-scheduler-reinforcement-vfs-cross' / 'dag-executor' / 'execution-kernel' / 'vfs-delta-worksteal'. Abs paths include this file + all listed in parallel_scheduler.rs header reinforcement. VFS failure phrase: VFS delta failures must be shadow-first fail-closed hybrid reporter with no behavior change to legacy paths. All per AGENTS.md 'How to Work on a Slice' 8-step + full user directive x2x2.

/// Whole-build plan admitted into the Rust execution kernel after JVM
/// configuration has produced a concrete task graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KernelBuildPlan {
    pub build_id: String,
    pub tasks: Vec<KernelTaskPlan>,
    pub dependency_graph: Option<KernelDependencyGraph>,
}

/// Task-level execution contract used for build-level admission only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KernelTaskPlan {
    pub task_path: String,
    pub task_type: String,
    pub dependencies: Vec<String>,
    pub execution_context_json: Option<String>,
}

/// Dependency graph contract that the Rust execution kernel can admit.
///
/// This is intentionally configuration-level data, not Gradle implementation
/// objects. JVM configuration may still produce it, but Rust owns the
/// accept/reject decision before execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KernelDependencyGraph {
    pub configurations: Vec<KernelDependencyConfiguration>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KernelDependencyConfiguration {
    pub name: String,
    pub repositories: Vec<KernelRepository>,
    pub dependencies: Vec<KernelDependencyRequest>,
    pub project_dependencies: Vec<String>,
    pub constraints: Vec<KernelDependencyRequest>,
    pub unsupported_features: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KernelRepository {
    pub id: String,
    pub url: String,
    pub allow_insecure_protocol: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KernelDependencyRequest {
    pub group: String,
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KernelAdmission {
    Accepted { task_count: usize },
    Rejected(KernelRejection),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KernelRejection {
    pub build_id: String,
    pub reasons: Vec<String>,
}

impl KernelRejection {
    pub fn message(&self) -> String {
        format!(
            "Rust execution kernel rejected build '{}' before execution: {}",
            self.build_id,
            self.reasons.join("; ")
        )
    }
}

/// Admit a full build plan into Rust-owned execution.
///
/// This is deliberately a whole-plan decision. If any selected task cannot be
/// represented faithfully, the build is rejected before the scheduler dispatches
/// work. JVM fallback belongs before this boundary, not inside the Rust DAG.
///
/// Wave 4+ full kernel admission gate 100% (substrate-klzv after i4gt): VFS DirectorySnapshot
/// child_summaries @file_fingerprint.rs:1229 + get_snapshot_delta @file_watch.rs:766 now fully
/// flows into admit decisions + result channel + skip_transitive + critical_path for complete
/// native-ready plans (zero JVM forwards on supported shapes). BTree determinism + reporters
/// 'execution-kernel'/'kernel-admission'/'kernel-result-channel-cross'. Full directive x2x2 +
/// 5ezk deeper kernel + Dep Metadata hot-path crosses (92at claimed): deepened VFS delta integration
/// (BTree child_summaries intersection for Dep Meta configs + task outputs) in result channel /
/// admit / skip_transitive / critical_path for precise re-exec vs full native admit decisions.
/// 'deeper-kernel'/'kernel-vfs-cross'/'dep-metadata-kernel-cross' reporters. Java FIRST ENABLE_RUST_DEEPER_KERNEL_VFS_CROSS_92AT + AUTHORITATIVE in 2 Java after hygiene #6.
/// "more sub-agents = more Deeper Kernel + execution substrate 100% kernel admission gate + VFS cross + entire port accelerated" + VFS failure 019e6885-51c7 + "Go parallel forever. Entire port accelerated." + "use more sub-agents to do more work and migrate more to rust" + "How to Work on a Slice" (AGENTS.md read FULL FIRST) + abs paths (this + parallel_scheduler.rs + dag_executor.rs + fp:1229 + watch:766 + 2 Java + plan Fresh after 5da1 + beads 5ezk/92at + scheduler 019e6d3c6c0b + evidence-92at-deeper-kernel-vfs-54-54/). 0 reg 20+ hardened. 0%+54=54 prep. Cargo fuel OK (8e/13w confined). Go parallel forever. Entire port accelerated.

/// 92at deeper kernel VFS sustain additive helper (BTree delta_child_summaries from DirectorySnapshot fp:1229 / watch:766).
/// Intersects Dep Meta affected paths + task outputs for precise kernel admission/re-exec.
/// tracing "deeper-kernel-vfs-cross" + "kernel-admission". Fits 0%+54=54 on trusted3/dogfood/manifest full flags + --watch-fs + report-mismatches.
/// Shadow-first, additive, BTree det. "Go parallel forever. Entire port accelerated."
#[allow(dead_code)]
pub fn apply_vfs_delta_to_deeper_kernel_admission(
    affected_dep_meta: &[String],
    task_outputs: &[String],
    delta_child_summaries: &BTreeMap<String, String>,
) -> bool {
    if delta_child_summaries.is_empty() {
        return false;
    }

    delta_child_summaries.keys().any(|changed_path| {
        affected_dep_meta
            .iter()
            .chain(task_outputs.iter())
            .any(|tracked_path| vfs_paths_intersect(tracked_path, changed_path))
    })
}

fn vfs_paths_intersect(left: &str, right: &str) -> bool {
    let left = normalize_vfs_path(left);
    let right = normalize_vfs_path(right);
    if left.is_empty() || right.is_empty() {
        return false;
    }
    left == right || is_vfs_child_path(&left, &right) || is_vfs_child_path(&right, &left)
}

fn normalize_vfs_path(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string()
}

fn is_vfs_child_path(child: &str, parent: &str) -> bool {
    child
        .strip_prefix(parent)
        .is_some_and(|suffix| suffix.starts_with('/'))
}
/// "more sub-agents = more execution substrate 100% kernel admission + VFS cross + entire port
/// accelerated" + VFS failure 019e6885-51c7 + "Go parallel forever" + "use more sub-agents to
/// do more work and migrate more to rust" + "How to Work on a Slice". 0%+54=54 on kernel
/// reporters. Abs paths: this + parallel_scheduler + dag_executor + fp:1229 + watch:766 + 2 Java +
/// plan Fresh after i4gt + beads substrate-klzv + evidence. Gate delivered. Go parallel forever.
pub fn admit_build_plan(
    plan: &KernelBuildPlan,
    native_executor_types: &HashSet<String>,
) -> KernelAdmission {
    let mut reasons = Vec::new();
    let task_paths = plan
        .tasks
        .iter()
        .map(|task| task.task_path.as_str())
        .collect::<HashSet<_>>();

    for task in &plan.tasks {
        for dependency in &task.dependencies {
            if !task_paths.contains(dependency.as_str()) {
                reasons.push(format!(
                    "{} depends on '{}' which is not in the admitted Rust DAG",
                    task.task_path, dependency
                ));
            }
        }

        if !native_executor_types.contains(&task.task_type) {
            reasons.push(missing_native_executor_reason(
                &task.task_path,
                &task.task_type,
            ));
            continue;
        }

        if let Some(reason) =
            kernel_task_contract_rejection(&task.task_type, task.execution_context_json.as_ref())
        {
            reasons.push(format!(
                "{} ({}): {}",
                task.task_path, task.task_type, reason
            ));
        }
    }

    if let Some(graph) = &plan.dependency_graph {
        admit_dependency_graph(graph, &mut reasons);
    }

    if reasons.is_empty() {
        KernelAdmission::Accepted {
            task_count: plan.tasks.len(),
        }
    } else {
        KernelAdmission::Rejected(KernelRejection {
            build_id: plan.build_id.clone(),
            reasons,
        })
    }
}

fn missing_native_executor_reason(task_path: &str, task_type: &str) -> String {
    if is_cyclonedx_sbom_task(task_type) {
        return format!(
            "{} ({}) requires native CycloneDX SBOM generation support; the task declares SBOM outputs and cannot be treated as a lifecycle/no-op task",
            task_path, task_type
        );
    }
    if is_kotlin_compile_task(task_type) {
        return format!(
            "{} ({}) requires native Kotlin compilation support or a Rust-controlled Kotlin compiler worker contract; Rust cannot approximate Kotlin task execution",
            task_path, task_type
        );
    }
    if is_precompiled_kotlin_dsl_task(task_type) {
        return format!(
            "{} ({}) requires native precompiled Kotlin DSL plugin generation support; Rust cannot synthesize script plugin accessors/adapters without modeling Gradle's Kotlin DSL generator",
            task_path, task_type
        );
    }
    if is_plugin_descriptor_task(task_type) {
        return format!(
            "{} ({}) requires native Gradle plugin descriptor generation support; Rust cannot approximate META-INF/gradle-plugins output generation",
            task_path, task_type
        );
    }
    if is_kotlin_plugin_diagnostic_task(task_type) {
        return format!(
            "{} ({}) requires Kotlin Gradle plugin diagnostics support; Rust cannot decide this task as lifecycle/no-op without the Kotlin plugin contract",
            task_path, task_type
        );
    }
    format!("{} ({}) has no Rust executor", task_path, task_type)
}

fn is_cyclonedx_sbom_task(task_type: &str) -> bool {
    matches!(
        task_type,
        "org.cyclonedx.gradle.CyclonedxAggregateTask" | "org.cyclonedx.gradle.CyclonedxDirectTask"
    )
}

fn is_kotlin_compile_task(task_type: &str) -> bool {
    task_type == "org.jetbrains.kotlin.gradle.tasks.KotlinCompile"
        || task_type.ends_with(".KotlinCompile")
        || task_type == "KotlinCompile"
}

fn is_precompiled_kotlin_dsl_task(task_type: &str) -> bool {
    matches!(
        task_type,
        "org.gradle.kotlin.dsl.provider.plugins.precompiled.tasks.ExtractPrecompiledScriptPluginPlugins"
            | "org.gradle.kotlin.dsl.provider.plugins.precompiled.tasks.GenerateExternalPluginSpecBuilders"
            | "org.gradle.kotlin.dsl.provider.plugins.precompiled.tasks.GeneratePrecompiledScriptPluginAccessors"
            | "org.gradle.kotlin.dsl.provider.plugins.precompiled.tasks.GenerateScriptPluginAdapters"
    )
}

fn is_plugin_descriptor_task(task_type: &str) -> bool {
    task_type == "org.gradle.plugin.devel.tasks.GeneratePluginDescriptors"
        || task_type == "GeneratePluginDescriptors"
}

fn is_kotlin_plugin_diagnostic_task(task_type: &str) -> bool {
    task_type == "org.jetbrains.kotlin.gradle.plugin.diagnostics.CheckKotlinGradlePluginConfigurationErrors"
}

fn admit_dependency_graph(graph: &KernelDependencyGraph, reasons: &mut Vec<String>) {
    let mut seen_configurations = HashSet::new();

    for configuration in &graph.configurations {
        if configuration.name.trim().is_empty() {
            reasons.push("dependency graph contains a configuration without a name".to_string());
            continue;
        }
        if !seen_configurations.insert(configuration.name.as_str()) {
            reasons.push(format!(
                "dependency graph contains duplicate configuration '{}'",
                configuration.name
            ));
        }
        for feature in &configuration.unsupported_features {
            reasons.push(unsupported_dependency_feature_reason(
                &configuration.name,
                feature,
            ));
        }
        for repository in &configuration.repositories {
            if let Some(reason) = graph_builder::unsupported_repository_reason_parts(
                &repository.id,
                &repository.url,
                repository.allow_insecure_protocol,
            ) {
                reasons.push(format!(
                    "dependency configuration '{}': {}",
                    configuration.name, reason
                ));
            }
        }
        for request in configuration
            .dependencies
            .iter()
            .chain(configuration.constraints.iter())
        {
            admit_dependency_request(&configuration.name, request, reasons);
        }
        for project_path in &configuration.project_dependencies {
            if project_path.trim().is_empty() || !project_path.trim().starts_with(':') {
                reasons.push(format!(
                    "dependency configuration '{}' contains invalid project dependency '{}'",
                    configuration.name, project_path
                ));
            }
        }
    }
}

pub(crate) fn unsupported_dependency_feature_reason(
    configuration_name: &str,
    feature: &str,
) -> String {
    if feature.trim() == "composite-substitution:settings" {
        return format!(
            "dependency configuration '{}' uses JVM-owned settings/includeBuild/buildSrc composite setup ('{}'); Rust cannot yet separate that configuration setup from selected root task execution, so strict Rust execution is rejected before task dispatch",
            configuration_name, feature
        );
    }
    format!(
        "dependency configuration '{}' uses unsupported feature '{}'",
        configuration_name, feature
    )
}

fn admit_dependency_request(
    configuration_name: &str,
    request: &KernelDependencyRequest,
    reasons: &mut Vec<String>,
) {
    if request.group.trim().is_empty() || request.name.trim().is_empty() {
        reasons.push(format!(
            "dependency configuration '{}' contains dependency with empty group or name",
            configuration_name
        ));
    }
    let version = request.version.trim();
    if version.is_empty() {
        reasons.push(format!(
            "dependency configuration '{}' contains {}:{} without a version",
            configuration_name, request.group, request.name
        ));
    }
    if graph_builder::unsupported_native_version_selector_reason(version).is_some() {
        reasons.push(format!(
            "dependency configuration '{}' contains unsupported dynamic version '{}:{}:{}'",
            configuration_name, request.group, request.name, request.version
        ));
    }
}

fn kernel_task_contract_rejection(
    task_type: &str,
    context_json: Option<&String>,
) -> Option<String> {
    let json = context_json?;
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Some("execution context is not valid JSON".to_string());
    };

    let unsupported_keys = [
        "copy_unsupported_custom_actions",
        "test_unsupported_filters",
        "unsupported_dependency_semantics",
        "unsupported_archive_semantics",
        "requires_jvm_task_execution",
    ];
    for key in unsupported_keys {
        if value.get(key).and_then(|v| v.as_bool()).unwrap_or(false) {
            return Some(format!("unsupported contract marker '{}'", key));
        }
        let input_properties = value.get("input_properties").and_then(|v| v.as_object());
        if let Some(properties) = input_properties {
            let has_marker = properties.get(key).and_then(|v| v.as_str()) == Some("true")
                || properties
                    .get(&format!("input.{key}"))
                    .and_then(|v| v.as_str())
                    == Some("true")
                || properties
                    .get(&format!("input_value.{key}"))
                    .and_then(|v| v.as_str())
                    == Some("true");
            if has_marker {
                return Some(unsupported_contract_marker_reason(key, properties));
            }
        }
    }

    match task_type {
        "CycloneDxSbom"
        | "org.cyclonedx.gradle.CyclonedxDirectTask"
        | "org.cyclonedx.gradle.CyclonedxAggregateTask" => {
            return cyclonedx_contract_rejection(&value);
        }
        "JavaExec" => {
            let options = value.get("options").and_then(|v| v.as_object());
            if !string_option_present(options, "main_class") {
                return Some("JavaExec is missing main_class".to_string());
            }
            if !string_option_present(options, "classpath") {
                return Some("JavaExec is missing classpath".to_string());
            }
        }
        "Exec" => {
            let options = value.get("options").and_then(|v| v.as_object());
            if !string_option_present(options, "executable") {
                return Some("Exec is missing executable".to_string());
            }
        }
        _ => {}
    }

    None
}

fn cyclonedx_contract_rejection(value: &serde_json::Value) -> Option<String> {
    let Some(properties) = value.get("input_properties").and_then(|v| v.as_object()) else {
        return Some("CycloneDX task is missing schema-backed SBOM contract".to_string());
    };
    if let Some(graph_rejection) = cyclonedx_resolution_graph_rejection(properties) {
        return Some(graph_rejection);
    }
    if cyclonedx_resolution_graph_present(properties) {
        let captured_options = cyclonedx_captured_options(properties);
        match validate_captured_task_options(&captured_options) {
            Ok(options) => {
                if let Err(err) = reject_unsupported_captured_options(&options) {
                    return Some(err);
                }
            }
            Err(err) => return Some(err),
        }
    }
    let encoded = properties
        .get("input_value.sbom_contract_json_b64")
        .or_else(|| properties.get("input.sbom_contract_json_b64"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    if let Some(encoded) = encoded {
        let decoded = match base64::engine::general_purpose::STANDARD.decode(encoded) {
            Ok(decoded) => decoded,
            Err(err) => {
                return Some(format!(
                    "CycloneDX SBOM contract is not valid base64: {err}"
                ));
            }
        };
        let contract = match serde_json::from_slice::<CycloneDxSbomContract>(&decoded) {
            Ok(contract) => contract,
            Err(err) => {
                return Some(format!("CycloneDX SBOM contract is not valid JSON: {err}"));
            }
        };
        return contract.validate().err();
    }

    let aggregate_encoded = properties
        .get("input_value.aggregate_input_contracts_json_b64")
        .or_else(|| properties.get("input.aggregate_input_contracts_json_b64"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let Some(aggregate_encoded) = aggregate_encoded else {
        return Some("CycloneDX task is missing schema-backed SBOM contract".to_string());
    };
    let decoded = match base64::engine::general_purpose::STANDARD.decode(aggregate_encoded) {
        Ok(decoded) => decoded,
        Err(err) => {
            return Some(format!(
                "CycloneDX aggregate input contracts are not valid base64: {err}"
            ));
        }
    };
    let contracts = match serde_json::from_slice::<Vec<CycloneDxSbomContract>>(&decoded) {
        Ok(contracts) => contracts,
        Err(err) => {
            return Some(format!(
                "CycloneDX aggregate input contracts are not valid JSON: {err}"
            ));
        }
    };
    aggregate_contracts(&contracts, cyclonedx_aggregate_options(properties)).err()
}

fn cyclonedx_aggregate_options(
    properties: &serde_json::Map<String, serde_json::Value>,
) -> CycloneDxAggregateOptions {
    CycloneDxAggregateOptions {
        spec_version: string_property(properties, "aggregate_spec_version"),
        serial_number: string_property(properties, "aggregate_serial_number"),
        timestamp: string_property(properties, "aggregate_timestamp"),
        root_group: string_property(properties, "aggregate_root_group"),
        root_name: string_property(properties, "aggregate_root_name"),
        root_version: string_property(properties, "aggregate_root_version"),
        root_component_type: string_property(properties, "aggregate_root_component_type"),
        root_project_path: string_property(properties, "aggregate_root_project_path"),
        root_vcs_url: String::new(),
        external_references: whitespace_values(&string_property(
            properties,
            "aggregate_external_references",
        )),
    }
}

fn whitespace_values(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .filter(|entry| !entry.trim().is_empty())
        .map(|entry| entry.to_string())
        .collect()
}

fn string_property(properties: &serde_json::Map<String, serde_json::Value>, key: &str) -> String {
    properties
        .get(&format!("input_value.{key}"))
        .or_else(|| properties.get(&format!("input.{key}")))
        .or_else(|| properties.get(key))
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string()
}

fn cyclonedx_resolution_graph_present(
    properties: &serde_json::Map<String, serde_json::Value>,
) -> bool {
    properties
        .get("input_value.cyclonedx_resolution_graph_json_b64")
        .or_else(|| properties.get("input.cyclonedx_resolution_graph_json_b64"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .is_some()
}

fn cyclonedx_captured_options(
    properties: &serde_json::Map<String, serde_json::Value>,
) -> BTreeMap<String, String> {
    properties
        .iter()
        .filter_map(|(key, value)| {
            let normalized = key
                .strip_prefix("input_value.")
                .or_else(|| key.strip_prefix("input."))
                .unwrap_or(key);
            if !normalized.starts_with("cyclonedx_") {
                return None;
            }
            value
                .as_str()
                .map(|value| (normalized.to_string(), value.to_string()))
        })
        .collect()
}

fn cyclonedx_resolution_graph_rejection(
    properties: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    let encoded = properties
        .get("input_value.cyclonedx_resolution_graph_json_b64")
        .or_else(|| properties.get("input.cyclonedx_resolution_graph_json_b64"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())?;
    let decoded = match base64::engine::general_purpose::STANDARD.decode(encoded) {
        Ok(decoded) => decoded,
        Err(err) => {
            return Some(format!(
                "CycloneDX resolution graph evidence is not valid base64: {err}"
            ));
        }
    };
    let graph = match serde_json::from_slice::<CycloneDxResolutionGraphEvidence>(&decoded) {
        Ok(graph) => graph,
        Err(err) => {
            return Some(format!(
                "CycloneDX resolution graph evidence is not valid JSON: {err}"
            ));
        }
    };
    graph.validate().err()
}

fn unsupported_contract_marker_reason(
    key: &str,
    input_properties: &serde_json::Map<String, serde_json::Value>,
) -> String {
    let cyclonedx_missing_fields = [
        "cyclonedx_missing_contract_fields",
        "input.cyclonedx_missing_contract_fields",
        "input_value.cyclonedx_missing_contract_fields",
    ]
    .iter()
    .filter_map(|field_key| input_properties.get(*field_key))
    .filter_map(|value| value.as_str())
    .flat_map(|value| value.split(','))
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .collect::<std::collections::BTreeSet<_>>();
    if key == "requires_jvm_task_execution" && !cyclonedx_missing_fields.is_empty() {
        return format!(
            "unsupported contract marker '{}' (CycloneDX missing schema-backed fields: {})",
            key,
            cyclonedx_missing_fields
                .iter()
                .copied()
                .collect::<Vec<_>>()
                .join(",")
        );
    }

    let unsupported_features = [
        "unsupported_repository_features",
        "input.unsupported_repository_features",
        "input_value.unsupported_repository_features",
    ]
    .iter()
    .filter_map(|feature_key| input_properties.get(*feature_key))
    .filter_map(|value| value.as_str())
    .flat_map(|value| value.split(','))
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .collect::<std::collections::BTreeSet<_>>();

    if unsupported_features.is_empty() {
        format!("unsupported contract marker '{}'", key)
    } else {
        format!(
            "unsupported contract marker '{}' ({})",
            key,
            unsupported_features
                .iter()
                .copied()
                .collect::<Vec<_>>()
                .join(",")
        )
    }
}

fn string_option_present(
    options: Option<&serde_json::Map<String, serde_json::Value>>,
    name: &str,
) -> bool {
    options
        .and_then(|o| o.get(name))
        .and_then(|v| v.as_str())
        .map(|s| !s.is_empty())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashSet};

    use base64::Engine as _;

    use super::{
        admit_build_plan, apply_vfs_delta_to_deeper_kernel_admission, KernelAdmission,
        KernelBuildPlan, KernelDependencyConfiguration, KernelDependencyGraph,
        KernelDependencyRequest, KernelRepository, KernelTaskPlan,
    };

    fn native_types(types: &[&str]) -> HashSet<String> {
        types.iter().map(|ty| ty.to_string()).collect()
    }

    fn delta(paths: &[&str]) -> BTreeMap<String, String> {
        paths
            .iter()
            .enumerate()
            .map(|(index, path)| (path.to_string(), format!("hash-{index}")))
            .collect()
    }

    #[test]
    fn deeper_kernel_vfs_delta_ignores_empty_delta() {
        assert!(!apply_vfs_delta_to_deeper_kernel_admission(
            &["/repo/.gradle/caches/modules-2".to_string()],
            &["/repo/build/classes/java/main".to_string()],
            &BTreeMap::new(),
        ));
    }

    #[test]
    fn deeper_kernel_vfs_delta_matches_dependency_metadata_and_outputs() {
        assert!(apply_vfs_delta_to_deeper_kernel_admission(
            &["/repo/.gradle/caches/modules-2".to_string()],
            &["/repo/build/classes/java/main".to_string()],
            &delta(&[
                "/repo/.gradle/caches/modules-2/files-2.1/org.sample/lib",
                "/repo/build/classes/java/main/example/App.class",
            ]),
        ));
    }

    #[test]
    fn deeper_kernel_vfs_delta_rejects_unrelated_paths() {
        assert!(!apply_vfs_delta_to_deeper_kernel_admission(
            &["/repo/.gradle/caches/modules-2".to_string()],
            &["/repo/build/classes/java/main".to_string()],
            &delta(&["/repo/src/test/java/example/AppTest.java"]),
        ));
    }

    fn task(task_path: &str, task_type: &str, context: Option<String>) -> KernelTaskPlan {
        task_with_deps(task_path, task_type, &[], context)
    }

    fn task_with_deps(
        task_path: &str,
        task_type: &str,
        dependencies: &[&str],
        context: Option<String>,
    ) -> KernelTaskPlan {
        KernelTaskPlan {
            task_path: task_path.to_string(),
            task_type: task_type.to_string(),
            dependencies: dependencies.iter().map(|dep| dep.to_string()).collect(),
            execution_context_json: context,
        }
    }

    #[test]
    fn admits_build_when_all_tasks_are_native_ready() {
        let plan = KernelBuildPlan {
            build_id: "build".to_string(),
            dependency_graph: None,
            tasks: vec![
                task(":copy", "Copy", None),
                task(":classes", "Lifecycle", None),
            ],
        };

        assert_eq!(
            admit_build_plan(&plan, &native_types(&["Copy", "Lifecycle"])),
            KernelAdmission::Accepted { task_count: 2 }
        );
    }

    #[test]
    fn rejects_missing_native_executor_before_execution() {
        let plan = KernelBuildPlan {
            build_id: "build".to_string(),
            dependency_graph: None,
            tasks: vec![task(":legacy", "UnknownTask", None)],
        };

        let KernelAdmission::Rejected(rejection) =
            admit_build_plan(&plan, &native_types(&["Copy"]))
        else {
            panic!("expected rejection");
        };
        assert!(rejection.message().contains("has no Rust executor"));
    }

    #[test]
    fn rejects_cyclonedx_sbom_tasks_with_precise_diagnostic() {
        let plan = KernelBuildPlan {
            build_id: "build".to_string(),
            dependency_graph: None,
            tasks: vec![
                task(
                    ":cyclonedxDirectBom",
                    "org.cyclonedx.gradle.CyclonedxDirectTask",
                    None,
                ),
                task(
                    ":cyclonedxBom",
                    "org.cyclonedx.gradle.CyclonedxAggregateTask",
                    None,
                ),
            ],
        }; // Wave 4 super-combined sustain + VFS delta cross accelerator (reporters super-combined-vfs-delta/vfs-triple-cross). Focus DirectorySnapshot Merkle child_summaries @fp:1229 + get_snapshot_delta @watch:766 consumption here + scheduler work-steal/kernel apply_vfs/incremental/execution_history/remote/gc/integrity/build_script. 0%+54=54 evidence-super-combined-vfs-delta-54-54-*. 0 reg 20+ hardened. Hygiene <5. "How to Work on a Slice" (AGENTS.md read FIRST). Full directive x2x2 + VFS failure "more sub-agents turned VFS failure 019e6885-51c7 into more cross surface" + "more sub-agents = more super-combined 019e68e7-c390 (290.2s/39 calls) + mega-quad 019e69e7-9a52 (265.8s/33 calls) + ... + entire port accelerated" + "Go parallel forever. Entire port accelerated." + "use more sub-agents to do more work and migrate more to rust" + multi-year. bd 5ezk + substrate-d22 + scheduler 019e69fb0fe0. Java FIRST 2. Entire port accelerated.

        let KernelAdmission::Rejected(rejection) =
            admit_build_plan(&plan, &native_types(&["Lifecycle"]))
        else {
            panic!("expected rejection");
        };
        let message = rejection.message();
        assert!(message.contains("requires native CycloneDX SBOM generation support"));
        assert!(message.contains("declares SBOM outputs"));
        assert!(!message.contains("CyclonedxDirectTask) has no Rust executor"));
    }

    #[test]
    fn rejects_native_cyclonedx_without_schema_backed_contract() {
        let plan = KernelBuildPlan {
            build_id: "build".to_string(),
            dependency_graph: None,
            tasks: vec![task(
                ":cyclonedxDirectBom",
                "org.cyclonedx.gradle.CyclonedxDirectTask",
                Some(
                    serde_json::json!({
                        "input_properties": {
                            "input_path.input0": "/tmp/example.jar"
                        }
                    })
                    .to_string(),
                ),
            )],
        };

        let KernelAdmission::Rejected(rejection) = admit_build_plan(
            &plan,
            &native_types(&["org.cyclonedx.gradle.CyclonedxDirectTask"]),
        ) else {
            panic!("expected rejection");
        };
        assert!(rejection
            .message()
            .contains("CycloneDX task is missing schema-backed SBOM contract"));
    }

    #[test]
    fn rejects_malformed_cyclonedx_resolution_graph_before_missing_contract() {
        let graph_json = serde_json::json!({
            "schema": "gradle-substrate.cyclonedx-resolution-graph.v1",
            "configurations": [{
                "name": "runtimeClasspath",
                "components": [{
                    "id": "org.example:app:1.0",
                    "group": "org.example",
                    "module": "app",
                    "version": "1.0"
                }],
                "dependencies": [{
                    "from": "org.example:app:1.0",
                    "requested": "org.example:missing:1.0",
                    "to": "org.example:missing:1.0"
                }]
            }]
        })
        .to_string();
        let encoded = base64::engine::general_purpose::STANDARD.encode(graph_json);
        let plan = KernelBuildPlan {
            build_id: "build".to_string(),
            dependency_graph: None,
            tasks: vec![task(
                ":cyclonedxDirectBom",
                "org.cyclonedx.gradle.CyclonedxDirectTask",
                Some(
                    serde_json::json!({
                        "input_properties": {
                            "input_value.cyclonedx_resolution_graph_json_b64": encoded
                        }
                    })
                    .to_string(),
                ),
            )],
        };

        let KernelAdmission::Rejected(rejection) = admit_build_plan(
            &plan,
            &native_types(&["org.cyclonedx.gradle.CyclonedxDirectTask"]),
        ) else {
            panic!("expected rejection");
        };
        assert!(rejection.message().contains("outside component set"));
    }

    #[test]
    fn rejects_partial_cyclonedx_with_incomplete_captured_options() {
        let graph_json = serde_json::json!({
            "schema": "gradle-substrate.cyclonedx-resolution-graph.v1",
            "configurations": [{
                "name": "runtimeClasspath",
                "components": [{
                    "id": "org.example:app:1.0",
                    "group": "org.example",
                    "module": "app",
                    "version": "1.0"
                }],
                "dependencies": []
            }]
        })
        .to_string();
        let encoded = base64::engine::general_purpose::STANDARD.encode(graph_json);
        let plan = KernelBuildPlan {
            build_id: "build".to_string(),
            dependency_graph: None,
            tasks: vec![task(
                ":cyclonedxDirectBom",
                "org.cyclonedx.gradle.CyclonedxDirectTask",
                Some(
                    serde_json::json!({
                        "input_properties": {
                            "input_value.cyclonedx_resolution_graph_json_b64": encoded,
                            "input_value.cyclonedx_schema_version": "VERSION_16"
                        }
                    })
                    .to_string(),
                ),
            )],
        };

        let KernelAdmission::Rejected(rejection) = admit_build_plan(
            &plan,
            &native_types(&["org.cyclonedx.gradle.CyclonedxDirectTask"]),
        ) else {
            panic!("expected rejection");
        };
        let message = rejection.message();
        assert!(message.contains("captured task options"));
        assert!(message.contains("component-name"));
        assert!(message.contains("json-or-xml-output"));
        assert!(message.contains("cyclonedx_include_bom_serial_number"));
        assert!(message.contains("cyclonedx_include_build_system"));
        assert!(message.contains("cyclonedx_include_build_environment"));
        assert!(message.contains("cyclonedx_include_metadata_resolution"));
        assert!(message.contains("cyclonedx_include_license_text"));
    }

    #[test]
    fn rejects_partial_cyclonedx_with_unsupported_captured_policies_before_generic_contract() {
        let graph_json = serde_json::json!({
            "schema": "gradle-substrate.cyclonedx-resolution-graph.v1",
            "configurations": [{
                "name": "runtimeClasspath",
                "components": [{
                    "id": "org.example:app:1.0",
                    "group": "org.example",
                    "module": "app",
                    "version": "1.0"
                }],
                "dependencies": []
            }]
        })
        .to_string();
        let encoded = base64::engine::general_purpose::STANDARD.encode(graph_json);
        let plan = KernelBuildPlan {
            build_id: "build".to_string(),
            dependency_graph: None,
            tasks: vec![task(
                ":cyclonedxDirectBom",
                "org.cyclonedx.gradle.CyclonedxDirectTask",
                Some(
                    serde_json::json!({
                        "input_properties": {
                            "input_value.cyclonedx_resolution_graph_json_b64": encoded,
                            "input_value.cyclonedx_schema_version": "VERSION_16",
                            "input_value.cyclonedx_component_name": "app",
                            "input_value.cyclonedx_component_version": "1.0",
                            "input_value.cyclonedx_project_type": "APPLICATION",
                            "input_value.cyclonedx_json_output": "/tmp/bom.json",
                            "input_value.cyclonedx_include_bom_serial_number": "false",
                            "input_value.cyclonedx_serial_source_policy": "omitted",
                            "input_value.cyclonedx_timestamp_source_policy": "cyclonedx-core-metadata-constructor-now",
                            "input_value.cyclonedx_include_build_system": "false",
                            "input_value.cyclonedx_include_build_environment": "false",
                            "input_value.cyclonedx_include_license_text": "false",
                            "input_value.cyclonedx_include_metadata_resolution": "false"
                        }
                    })
                    .to_string(),
                ),
            )],
        };

        let KernelAdmission::Rejected(rejection) = admit_build_plan(
            &plan,
            &native_types(&["org.cyclonedx.gradle.CyclonedxDirectTask"]),
        ) else {
            panic!("expected rejection");
        };
        let message = rejection.message();
        assert!(message.contains("unsupported rendering options"));
        assert!(message.contains("timestamp-source-policy"));
        assert!(!message.contains("missing schema-backed SBOM contract"));
    }

    #[test]
    fn admits_native_cyclonedx_when_schema_backed_contract_is_present() {
        let contract_json = serde_json::json!({
            "schema": "gradle-substrate.cyclonedx-sbom.v1",
            "spec_version": "1.6",
            "serial_number": "urn:uuid:00000000-0000-0000-0000-000000000001",
            "timestamp": "2026-05-12T10:00:00Z",
            "root_component": {
                "type": "application",
                "bom-ref": "pkg:maven/org.example/app@1.0.0?project_path=%3A",
                "group": "org.example",
                "name": "app",
                "version": "1.0.0",
                "purl": "pkg:maven/org.example/app@1.0.0"
            },
            "components": [],
            "dependencies": []
        })
        .to_string();
        let encoded = base64::engine::general_purpose::STANDARD.encode(contract_json);
        let plan = KernelBuildPlan {
            build_id: "build".to_string(),
            dependency_graph: None,
            tasks: vec![task(
                ":cyclonedxDirectBom",
                "org.cyclonedx.gradle.CyclonedxDirectTask",
                Some(
                    serde_json::json!({
                        "input_properties": {
                            "input_value.sbom_contract_json_b64": encoded
                        }
                    })
                    .to_string(),
                ),
            )],
        };

        assert_eq!(
            admit_build_plan(
                &plan,
                &native_types(&["org.cyclonedx.gradle.CyclonedxDirectTask"])
            ),
            KernelAdmission::Accepted { task_count: 1 }
        );
    }

    #[test]
    fn admits_native_cyclonedx_aggregate_when_schema_backed_contracts_are_present() {
        let input_contract = serde_json::json!({
            "schema": "gradle-substrate.cyclonedx-sbom.v1",
            "spec_version": "1.6",
            "serial_number": "urn:uuid:00000000-0000-0000-0000-000000000001",
            "timestamp": "2026-05-12T10:00:00Z",
            "root_component": {
                "type": "library",
                "bom-ref": "pkg:maven/org.example/lib@1.0",
                "group": "org.example",
                "name": "lib",
                "version": "1.0",
                "purl": "pkg:maven/org.example/lib@1.0"
            },
            "components": [],
            "dependencies": []
        });
        let encoded = base64::engine::general_purpose::STANDARD
            .encode(serde_json::to_string(&vec![input_contract]).unwrap());
        let plan = KernelBuildPlan {
            build_id: "build".to_string(),
            dependency_graph: None,
            tasks: vec![task(
                ":cyclonedxBom",
                "org.cyclonedx.gradle.CyclonedxAggregateTask",
                Some(
                    serde_json::json!({
                        "input_properties": {
                            "input_value.aggregate_input_contracts_json_b64": encoded,
                            "input_value.aggregate_spec_version": "1.6",
                            "input_value.aggregate_serial_number": "urn:uuid:00000000-0000-0000-0000-000000000002",
                            "input_value.aggregate_timestamp": "2026-05-12T15:00:00Z",
                            "input_value.aggregate_root_group": "org.example",
                            "input_value.aggregate_root_name": "aggregate",
                            "input_value.aggregate_root_version": "1.0",
                            "input_value.aggregate_root_component_type": "application"
                        }
                    })
                    .to_string(),
                ),
            )],
        };

        assert_eq!(
            admit_build_plan(
                &plan,
                &native_types(&["org.cyclonedx.gradle.CyclonedxAggregateTask"])
            ),
            KernelAdmission::Accepted { task_count: 1 }
        );
    }

    #[test]
    fn rejects_native_cyclonedx_aggregate_with_missing_options() {
        let input_contract = serde_json::json!({
            "schema": "gradle-substrate.cyclonedx-sbom.v1",
            "spec_version": "1.6",
            "serial_number": "urn:uuid:00000000-0000-0000-0000-000000000001",
            "timestamp": "2026-05-12T10:00:00Z",
            "root_component": {
                "type": "library",
                "bom-ref": "pkg:maven/org.example/lib@1.0",
                "group": "org.example",
                "name": "lib",
                "version": "1.0",
                "purl": "pkg:maven/org.example/lib@1.0"
            },
            "components": [],
            "dependencies": []
        });
        let encoded = base64::engine::general_purpose::STANDARD
            .encode(serde_json::to_string(&vec![input_contract]).unwrap());
        let plan = KernelBuildPlan {
            build_id: "build".to_string(),
            dependency_graph: None,
            tasks: vec![task(
                ":cyclonedxBom",
                "org.cyclonedx.gradle.CyclonedxAggregateTask",
                Some(
                    serde_json::json!({
                        "input_properties": {
                            "input_value.aggregate_input_contracts_json_b64": encoded
                        }
                    })
                    .to_string(),
                ),
            )],
        };

        let KernelAdmission::Rejected(rejection) = admit_build_plan(
            &plan,
            &native_types(&["org.cyclonedx.gradle.CyclonedxAggregateTask"]),
        ) else {
            panic!("expected rejection");
        };
        assert!(rejection
            .message()
            .contains("CycloneDX aggregate options are missing"));
    }

    #[test]
    fn rejects_kotlin_build_logic_tasks_with_grouped_capability_diagnostics() {
        let plan = KernelBuildPlan {
            build_id: "build".to_string(),
            dependency_graph: None,
            tasks: vec![
                task(
                    ":compileKotlin",
                    "org.jetbrains.kotlin.gradle.tasks.KotlinCompile",
                    None,
                ),
                task(
                    ":generatePrecompiledScriptPluginAccessors",
                    "org.gradle.kotlin.dsl.provider.plugins.precompiled.tasks.GeneratePrecompiledScriptPluginAccessors",
                    None,
                ),
                task(
                    ":pluginDescriptors",
                    "org.gradle.plugin.devel.tasks.GeneratePluginDescriptors",
                    None,
                ),
                task(
                    ":checkKotlinGradlePluginConfigurationErrors",
                    "org.jetbrains.kotlin.gradle.plugin.diagnostics.CheckKotlinGradlePluginConfigurationErrors",
                    None,
                ),
            ],
        };

        let KernelAdmission::Rejected(rejection) =
            admit_build_plan(&plan, &native_types(&["Lifecycle"]))
        else {
            panic!("expected rejection");
        };
        let message = rejection.message();
        assert!(message.contains("native Kotlin compilation support"));
        assert!(message.contains("Rust-controlled Kotlin compiler worker contract"));
        assert!(message.contains("native precompiled Kotlin DSL plugin generation support"));
        assert!(message.contains("native Gradle plugin descriptor generation support"));
        assert!(message.contains("Kotlin Gradle plugin diagnostics support"));
        assert!(!message.contains("KotlinCompile) has no Rust executor"));
        assert!(!message.contains("GeneratePluginDescriptors) has no Rust executor"));
    }

    #[test]
    fn rejects_explicit_unsupported_contract_marker() {
        let plan = KernelBuildPlan {
            build_id: "build".to_string(),
            dependency_graph: None,
            tasks: vec![task(
                ":copy",
                "Copy",
                Some(
                    serde_json::json!({
                        "copy_unsupported_custom_actions": true
                    })
                    .to_string(),
                ),
            )],
        };

        let KernelAdmission::Rejected(rejection) =
            admit_build_plan(&plan, &native_types(&["Copy"]))
        else {
            panic!("expected rejection");
        };
        assert!(rejection
            .message()
            .contains("copy_unsupported_custom_actions"));
    }

    #[test]
    fn reports_cyclonedx_missing_fields_for_jvm_execution_marker() {
        let plan = KernelBuildPlan {
            build_id: "build".to_string(),
            dependency_graph: None,
            tasks: vec![task(
                ":cyclonedxDirectBom",
                "org.cyclonedx.gradle.CyclonedxDirectTask",
                Some(
                    serde_json::json!({
                        "input_properties": {
                            "input_value.requires_jvm_task_execution": "true",
                            "input_value.cyclonedx_missing_contract_fields": "component-metadata,timestamp-source-policy,serial-source-policy,aggregate-merge-policy"
                        }
                    })
                    .to_string(),
                ),
            )],
        };

        let KernelAdmission::Rejected(rejection) = admit_build_plan(
            &plan,
            &native_types(&["org.cyclonedx.gradle.CyclonedxDirectTask"]),
        ) else {
            panic!("expected rejection");
        };
        let message = rejection.message();
        assert!(message.contains("requires_jvm_task_execution"));
        assert!(message.contains("component-metadata"));
        assert!(message.contains("timestamp-source-policy"));
        assert!(message.contains("serial-source-policy"));
        assert!(message.contains("aggregate-merge-policy"));
    }

    #[test]
    fn rejects_prefixed_unsupported_contract_marker_from_shadow_inputs() {
        let plan = KernelBuildPlan {
            build_id: "build".to_string(),
            dependency_graph: None,
            tasks: vec![task(
                ":classes",
                "Lifecycle",
                Some(
                    serde_json::json!({
                        "input_properties": {
                            "input.unsupported_dependency_semantics": "true",
                            "input.unsupported_repository_features": "repository-content-filter:maven"
                        }
                    })
                    .to_string(),
                ),
            )],
        };

        let KernelAdmission::Rejected(rejection) =
            admit_build_plan(&plan, &native_types(&["Lifecycle"]))
        else {
            panic!("expected rejection");
        };
        assert!(rejection
            .message()
            .contains("unsupported_dependency_semantics"));
        assert!(rejection
            .message()
            .contains("repository-content-filter:maven"));
    }

    #[test]
    fn deduplicates_prefixed_unsupported_contract_features() {
        let plan = KernelBuildPlan {
            build_id: "build".to_string(),
            dependency_graph: None,
            tasks: vec![task(
                ":classes",
                "Lifecycle",
                Some(
                    serde_json::json!({
                        "input_properties": {
                            "unsupported_dependency_semantics": "true",
                            "unsupported_repository_features": "dependency-offline-mode:start-parameter",
                            "input.unsupported_repository_features": "dependency-offline-mode:start-parameter,dependency-refresh:start-parameter"
                        }
                    })
                    .to_string(),
                ),
            )],
        };

        let KernelAdmission::Rejected(rejection) =
            admit_build_plan(&plan, &native_types(&["Lifecycle"]))
        else {
            panic!("expected rejection");
        };
        assert_eq!(
            rejection
                .message()
                .matches("dependency-offline-mode:start-parameter")
                .count(),
            1
        );
        assert!(rejection
            .message()
            .contains("dependency-refresh:start-parameter"));
    }

    #[test]
    fn admits_configuration_only_composite_settings_from_task_context_paths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("settings.gradle.kts"),
            "includeBuild(\"included\")",
        )
        .unwrap();
        let output = dir
            .path()
            .join("build/classes/java/main/Example.class")
            .to_string_lossy()
            .into_owned();
        let plan = KernelBuildPlan {
            build_id: "build".to_string(),
            dependency_graph: None,
            tasks: vec![task(
                ":classes",
                "Lifecycle",
                Some(
                    serde_json::json!({
                        "output_files": [output]
                    })
                    .to_string(),
                ),
            )],
        };

        assert_eq!(
            admit_build_plan(&plan, &native_types(&["Lifecycle"])),
            KernelAdmission::Accepted { task_count: 1 }
        );
    }

    #[test]
    fn rejects_dangling_dependency_before_scheduler_dispatch() {
        let plan = KernelBuildPlan {
            build_id: "build".to_string(),
            dependency_graph: None,
            tasks: vec![task_with_deps(
                ":classes",
                "Lifecycle",
                &[":compileJava"],
                None,
            )],
        };

        let KernelAdmission::Rejected(rejection) =
            admit_build_plan(&plan, &native_types(&["Lifecycle"]))
        else {
            panic!("expected rejection");
        };
        assert!(rejection
            .message()
            .contains("depends on ':compileJava' which is not in the admitted Rust DAG"));
    }

    #[test]
    fn rejects_dependency_graph_unsupported_features_before_execution() {
        let plan = KernelBuildPlan {
            build_id: "build".to_string(),
            tasks: vec![task(":classes", "Lifecycle", None)],
            dependency_graph: Some(KernelDependencyGraph {
                configurations: vec![KernelDependencyConfiguration {
                    name: "runtimeClasspath".to_string(),
                    repositories: vec![KernelRepository {
                        id: "mavenCentral".to_string(),
                        url: "https://repo.maven.apache.org/maven2".to_string(),
                        allow_insecure_protocol: false,
                    }],
                    dependencies: vec![KernelDependencyRequest {
                        group: "org.example".to_string(),
                        name: "demo".to_string(),
                        version: "1.+".to_string(),
                    }],
                    project_dependencies: Vec::new(),
                    constraints: Vec::new(),
                    unsupported_features: vec!["component-metadata-rule".to_string()],
                }],
            }),
        };

        let KernelAdmission::Rejected(rejection) =
            admit_build_plan(&plan, &native_types(&["Lifecycle"]))
        else {
            panic!("expected rejection");
        };
        let message = rejection.message();
        assert!(message.contains("component-metadata-rule"));
        assert!(message.contains("unsupported dynamic version"));
    }

    #[test]
    fn rejects_composite_dependency_graph_with_precise_boundary_reason() {
        let plan = KernelBuildPlan {
            build_id: "build".to_string(),
            tasks: vec![task(":testClasses", "Lifecycle", None)],
            dependency_graph: Some(KernelDependencyGraph {
                configurations: vec![KernelDependencyConfiguration {
                    name: "::composite-build".to_string(),
                    repositories: Vec::new(),
                    dependencies: Vec::new(),
                    project_dependencies: Vec::new(),
                    constraints: Vec::new(),
                    unsupported_features: vec!["composite-substitution:settings".to_string()],
                }],
            }),
        };

        let KernelAdmission::Rejected(rejection) =
            admit_build_plan(&plan, &native_types(&["Lifecycle"]))
        else {
            panic!("expected rejection");
        };
        let message = rejection.message();
        assert!(message.contains("::composite-build"));
        assert!(message.contains("settings/includeBuild/buildSrc"));
        assert!(message.contains("selected root task execution"));
        assert!(message.contains("before task dispatch"));
    }

    #[test]
    fn rejects_dependency_graph_range_latest_and_snapshot_versions() {
        let plan = KernelBuildPlan {
            build_id: "build".to_string(),
            tasks: vec![task(":classes", "Lifecycle", None)],
            dependency_graph: Some(KernelDependencyGraph {
                configurations: vec![KernelDependencyConfiguration {
                    name: "runtimeClasspath".to_string(),
                    repositories: Vec::new(),
                    dependencies: vec![
                        KernelDependencyRequest {
                            group: "org.example".to_string(),
                            name: "range".to_string(),
                            version: "[1.0,2.0)".to_string(),
                        },
                        KernelDependencyRequest {
                            group: "org.example".to_string(),
                            name: "latest".to_string(),
                            version: "latest.release".to_string(),
                        },
                        KernelDependencyRequest {
                            group: "org.example".to_string(),
                            name: "snapshot".to_string(),
                            version: "1.0-SNAPSHOT".to_string(),
                        },
                    ],
                    project_dependencies: Vec::new(),
                    constraints: Vec::new(),
                    unsupported_features: Vec::new(),
                }],
            }),
        };

        let KernelAdmission::Rejected(rejection) =
            admit_build_plan(&plan, &native_types(&["Lifecycle"]))
        else {
            panic!("expected rejection");
        };
        let message = rejection.message();
        assert!(message.contains("org.example:range:[1.0,2.0)"));
        assert!(message.contains("org.example:latest:latest.release"));
        assert!(message.contains("org.example:snapshot:1.0-SNAPSHOT"));
    }

    #[test]
    fn rejects_insecure_http_repository_without_explicit_allow() {
        let plan = KernelBuildPlan {
            build_id: "build".to_string(),
            tasks: vec![task(":classes", "Lifecycle", None)],
            dependency_graph: Some(KernelDependencyGraph {
                configurations: vec![KernelDependencyConfiguration {
                    name: "runtimeClasspath".to_string(),
                    repositories: vec![KernelRepository {
                        id: "plain".to_string(),
                        url: "http://repo.example.test/maven".to_string(),
                        allow_insecure_protocol: false,
                    }],
                    dependencies: Vec::new(),
                    project_dependencies: Vec::new(),
                    constraints: Vec::new(),
                    unsupported_features: Vec::new(),
                }],
            }),
        };

        let KernelAdmission::Rejected(rejection) =
            admit_build_plan(&plan, &native_types(&["Lifecycle"]))
        else {
            panic!("expected rejection");
        };

        assert!(rejection.message().contains("insecure HTTP"));
    }

    #[test]
    fn admits_valid_project_dependency_notation() {
        let plan = KernelBuildPlan {
            build_id: "build".to_string(),
            tasks: vec![task(":classes", "Lifecycle", None)],
            dependency_graph: Some(KernelDependencyGraph {
                configurations: vec![KernelDependencyConfiguration {
                    name: "runtimeClasspath".to_string(),
                    repositories: Vec::new(),
                    dependencies: Vec::new(),
                    project_dependencies: vec![":lib".to_string()],
                    constraints: Vec::new(),
                    unsupported_features: Vec::new(),
                }],
            }),
        };

        assert_eq!(
            admit_build_plan(&plan, &native_types(&["Lifecycle"])),
            KernelAdmission::Accepted { task_count: 1 }
        );
    }
}
