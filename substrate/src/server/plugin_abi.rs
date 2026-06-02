use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::configuration_ir::{CanonicalConfigurationGraph, CanonicalPluginModel};

pub const NATIVE_PLUGIN_ABI_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativePluginContract {
    pub abi_version: u32,
    pub plugin_id: String,
    pub implementation_kind: String,
    pub capabilities: Vec<String>,
    pub model_contributions: Vec<NativePluginModelContribution>,
    pub task_registrations: Vec<NativePluginTaskRegistration>,
    pub dependency_requests: Vec<NativePluginDependencyRequest>,
    pub execution_handlers: Vec<NativePluginExecutionHandler>,
    pub diagnostics: Vec<NativePluginDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativePluginModelContribution {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativePluginTaskRegistration {
    pub task_name: String,
    pub task_type: String,
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativePluginDependencyRequest {
    pub configuration: String,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativePluginExecutionHandler {
    pub task_type: String,
    pub handler_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativePluginDiagnostic {
    pub severity: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NativePluginAdmission {
    Accepted { plugin_id: String },
    Rejected(NativePluginRejection),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativePluginRejection {
    pub plugin_id: String,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativePluginResolution {
    pub contracts: Vec<NativePluginContract>,
    pub rejections: Vec<NativePluginRejection>,
}

impl NativePluginRejection {
    pub fn message(&self) -> String {
        format!(
            "Rust native plugin ABI rejected '{}': {}",
            self.plugin_id,
            self.reasons.join("; ")
        )
    }
}

pub fn builtin_native_plugin_contract(plugin_id: &str) -> Option<NativePluginContract> {
    match plugin_id {
        "base" | "org.gradle.base" => Some(base_plugin_contract(plugin_id)),
        "java" | "org.gradle.java" => Some(java_plugin_contract(plugin_id, false)),
        "java-library" | "org.gradle.java-library" => Some(java_plugin_contract(plugin_id, true)),
        "application" | "org.gradle.application" => Some(application_plugin_contract(plugin_id)),
        _ => None,
    }
}

pub fn admit_native_plugin_contract(contract: &NativePluginContract) -> NativePluginAdmission {
    let mut reasons = Vec::new();
    if contract.abi_version != NATIVE_PLUGIN_ABI_VERSION {
        reasons.push(format!(
            "unsupported native plugin ABI version {} (expected {})",
            contract.abi_version, NATIVE_PLUGIN_ABI_VERSION
        ));
    }
    if contract.plugin_id.trim().is_empty() {
        reasons.push("plugin id is empty".to_string());
    }
    if contract.implementation_kind != "builtin-rust" {
        reasons.push(format!(
            "implementation kind '{}' is not executable by the Rust plugin ABI",
            contract.implementation_kind
        ));
    }
    if contract.capabilities.is_empty() {
        reasons.push("native plugin contract declares no capabilities".to_string());
    }
    if contract.task_registrations.is_empty() && contract.model_contributions.is_empty() {
        reasons.push("native plugin contract contributes no model or tasks".to_string());
    }
    let mut task_names = BTreeSet::new();
    for task in &contract.task_registrations {
        if task.task_name.trim().is_empty() {
            reasons.push(format!(
                "native plugin '{}' declares an empty task name",
                contract.plugin_id
            ));
        }
        if task.task_type.trim().is_empty() {
            reasons.push(format!(
                "native plugin '{}' task '{}' has no task type",
                contract.plugin_id, task.task_name
            ));
        }
        if !task_names.insert(task.task_name.as_str()) {
            reasons.push(format!(
                "native plugin '{}' declares duplicate task '{}'",
                contract.plugin_id, task.task_name
            ));
        }
    }

    if reasons.is_empty() {
        NativePluginAdmission::Accepted {
            plugin_id: contract.plugin_id.clone(),
        }
    } else {
        NativePluginAdmission::Rejected(NativePluginRejection {
            plugin_id: contract.plugin_id.clone(),
            reasons,
        })
    }
}

pub fn resolve_native_plugin_contracts(
    graph: &CanonicalConfigurationGraph,
) -> NativePluginResolution {
    let mut contracts = Vec::new();
    let mut rejections = Vec::new();
    for plugin in graph.plugins.iter().filter(|plugin| plugin.apply) {
        match contract_for_plugin(plugin) {
            Some(contract) => match admit_native_plugin_contract(&contract) {
                NativePluginAdmission::Accepted { .. } => contracts.push(contract),
                NativePluginAdmission::Rejected(rejection) => rejections.push(rejection),
            },
            None => rejections.push(NativePluginRejection {
                plugin_id: plugin.id.clone(),
                reasons: vec![format!(
                    "applied plugin '{}' on project '{}' has no Rust native plugin contract",
                    plugin.id, plugin.project_path
                )],
            }),
        }
    }
    for dependency in &graph.plugin_classpath {
        rejections.push(NativePluginRejection {
            plugin_id: dependency.notation.clone(),
            reasons: vec![format!(
                "plugin classpath dependency '{}' on project '{}' requires a native plugin ABI or JVM guest runtime",
                dependency.notation, dependency.project_path
            )],
        });
    }
    contracts.sort_unstable_by(|a, b| a.plugin_id.cmp(&b.plugin_id));
    rejections.sort_unstable_by(|a, b| a.plugin_id.cmp(&b.plugin_id));
    NativePluginResolution {
        contracts,
        rejections,
    }
}

fn contract_for_plugin(plugin: &CanonicalPluginModel) -> Option<NativePluginContract> {
    builtin_native_plugin_contract(&plugin.id)
}

fn base_plugin_contract(plugin_id: &str) -> NativePluginContract {
    NativePluginContract {
        abi_version: NATIVE_PLUGIN_ABI_VERSION,
        plugin_id: plugin_id.to_string(),
        implementation_kind: "builtin-rust".to_string(),
        capabilities: vec![
            "model:lifecycle".to_string(),
            "task:lifecycle".to_string(),
            "diagnostics".to_string(),
        ],
        model_contributions: vec![NativePluginModelContribution {
            name: "lifecycle".to_string(),
            value: "base".to_string(),
        }],
        task_registrations: vec![
            lifecycle_task("assemble", &[]),
            lifecycle_task("check", &[]),
            lifecycle_task("build", &["assemble", "check"]),
            lifecycle_task("clean", &[]),
        ],
        dependency_requests: Vec::new(),
        execution_handlers: vec![NativePluginExecutionHandler {
            task_type: "Lifecycle".to_string(),
            handler_id: "builtin:lifecycle".to_string(),
        }],
        diagnostics: Vec::new(),
    }
}

fn java_plugin_contract(plugin_id: &str, library: bool) -> NativePluginContract {
    let mut configurations = vec![
        dependency_request("implementation", "dependency-bucket"),
        dependency_request("compileOnly", "dependency-bucket"),
        dependency_request("runtimeOnly", "dependency-bucket"),
        dependency_request("testImplementation", "dependency-bucket"),
        dependency_request("testRuntimeOnly", "dependency-bucket"),
        dependency_request("compileClasspath", "resolvable"),
        dependency_request("runtimeClasspath", "resolvable"),
        dependency_request("testCompileClasspath", "resolvable"),
        dependency_request("testRuntimeClasspath", "resolvable"),
    ];
    if library {
        configurations.push(dependency_request("api", "dependency-bucket"));
    }
    NativePluginContract {
        abi_version: NATIVE_PLUGIN_ABI_VERSION,
        plugin_id: plugin_id.to_string(),
        implementation_kind: "builtin-rust".to_string(),
        capabilities: vec![
            "model:source-sets".to_string(),
            "model:java-toolchain".to_string(),
            "task:java-compile".to_string(),
            "task:resources".to_string(),
            "task:jar".to_string(),
            "task:test".to_string(),
            "dependency-configurations".to_string(),
        ],
        model_contributions: vec![
            NativePluginModelContribution {
                name: "sourceSet".to_string(),
                value: "main".to_string(),
            },
            NativePluginModelContribution {
                name: "sourceSet".to_string(),
                value: "test".to_string(),
            },
        ],
        task_registrations: vec![
            NativePluginTaskRegistration {
                task_name: "compileJava".to_string(),
                task_type: "JavaCompile".to_string(),
                depends_on: Vec::new(),
            },
            NativePluginTaskRegistration {
                task_name: "processResources".to_string(),
                task_type: "Copy".to_string(),
                depends_on: Vec::new(),
            },
            lifecycle_task("classes", &["compileJava", "processResources"]),
            NativePluginTaskRegistration {
                task_name: "jar".to_string(),
                task_type: "Jar".to_string(),
                depends_on: vec!["classes".to_string()],
            },
            NativePluginTaskRegistration {
                task_name: "test".to_string(),
                task_type: "Test".to_string(),
                depends_on: vec!["classes".to_string()],
            },
            lifecycle_task("check", &["test"]),
            lifecycle_task("assemble", &["jar"]),
            lifecycle_task("build", &["assemble", "check"]),
        ],
        dependency_requests: configurations,
        execution_handlers: vec![
            NativePluginExecutionHandler {
                task_type: "JavaCompile".to_string(),
                handler_id: "builtin:java-compile".to_string(),
            },
            NativePluginExecutionHandler {
                task_type: "Copy".to_string(),
                handler_id: "builtin:copy".to_string(),
            },
            NativePluginExecutionHandler {
                task_type: "Jar".to_string(),
                handler_id: "builtin:jar".to_string(),
            },
            NativePluginExecutionHandler {
                task_type: "Lifecycle".to_string(),
                handler_id: "builtin:lifecycle".to_string(),
            },
        ],
        diagnostics: Vec::new(),
    }
}

fn application_plugin_contract(plugin_id: &str) -> NativePluginContract {
    let mut contract = java_plugin_contract(plugin_id, false);
    contract.capabilities.push("model:application".to_string());
    contract
        .model_contributions
        .push(NativePluginModelContribution {
            name: "extension".to_string(),
            value: "application".to_string(),
        });
    contract.task_registrations.extend([
        NativePluginTaskRegistration {
            task_name: "run".to_string(),
            task_type: "JavaExec".to_string(),
            depends_on: vec!["classes".to_string()],
        },
        NativePluginTaskRegistration {
            task_name: "startScripts".to_string(),
            task_type: "CreateStartScripts".to_string(),
            depends_on: vec!["classes".to_string()],
        },
        lifecycle_task("installDist", &["jar", "startScripts"]),
        NativePluginTaskRegistration {
            task_name: "distZip".to_string(),
            task_type: "Zip".to_string(),
            depends_on: vec!["jar", "startScripts"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        },
        NativePluginTaskRegistration {
            task_name: "distTar".to_string(),
            task_type: "Tar".to_string(),
            depends_on: vec!["jar", "startScripts"]
                .into_iter()
                .map(str::to_string)
                .collect(),
        },
    ]);
    contract.execution_handlers.extend([
        NativePluginExecutionHandler {
            task_type: "JavaExec".to_string(),
            handler_id: "builtin:java-exec".to_string(),
        },
        NativePluginExecutionHandler {
            task_type: "CreateStartScripts".to_string(),
            handler_id: "builtin:start-scripts".to_string(),
        },
        NativePluginExecutionHandler {
            task_type: "Zip".to_string(),
            handler_id: "builtin:zip".to_string(),
        },
        NativePluginExecutionHandler {
            task_type: "Tar".to_string(),
            handler_id: "builtin:tar".to_string(),
        },
    ]);
    contract
}

fn lifecycle_task(task_name: &str, depends_on: &[&str]) -> NativePluginTaskRegistration {
    NativePluginTaskRegistration {
        task_name: task_name.to_string(),
        task_type: "Lifecycle".to_string(),
        depends_on: depends_on.iter().map(|task| task.to_string()).collect(),
    }
}

fn dependency_request(configuration: &str, role: &str) -> NativePluginDependencyRequest {
    NativePluginDependencyRequest {
        configuration: configuration.to_string(),
        role: role.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::configuration_ir::{
        CanonicalConfigurationGraph, CanonicalPluginClasspathDependency, CanonicalPluginModel,
        CONFIGURATION_GRAPH_SCHEMA_VERSION,
    };

    #[test]
    fn builtin_java_plugin_contract_is_admitted() {
        let contract = builtin_native_plugin_contract("java").unwrap();

        assert_eq!(
            admit_native_plugin_contract(&contract),
            NativePluginAdmission::Accepted {
                plugin_id: "java".to_string()
            }
        );
        assert!(contract
            .task_registrations
            .iter()
            .any(|task| task.task_name == "compileJava" && task.task_type == "JavaCompile"));
        assert!(contract
            .dependency_requests
            .iter()
            .any(|request| request.configuration == "runtimeClasspath"));
    }

    #[test]
    fn admission_rejects_unknown_abi_version_and_duplicate_tasks() {
        let mut contract = builtin_native_plugin_contract("base").unwrap();
        contract.abi_version = NATIVE_PLUGIN_ABI_VERSION + 1;
        contract
            .task_registrations
            .push(contract.task_registrations[0].clone());

        let NativePluginAdmission::Rejected(rejection) = admit_native_plugin_contract(&contract)
        else {
            panic!("expected ABI rejection");
        };

        let message = rejection.message();
        assert!(message.contains("unsupported native plugin ABI version"));
        assert!(message.contains("duplicate task"));
    }

    #[test]
    fn resolves_supported_and_unsupported_plugins_from_configuration_graph() {
        let graph = CanonicalConfigurationGraph {
            schema_version: CONFIGURATION_GRAPH_SCHEMA_VERSION,
            build_id: "build".to_string(),
            settings: None,
            environment: None,
            projects: Vec::new(),
            source_sets: Vec::new(),
            tasks: Vec::new(),
            plugins: vec![
                CanonicalPluginModel {
                    project_path: ":".to_string(),
                    id: "java".to_string(),
                    version: String::new(),
                    apply: true,
                    source: "project-script".to_string(),
                },
                CanonicalPluginModel {
                    project_path: ":".to_string(),
                    id: "com.example.custom".to_string(),
                    version: "1.0".to_string(),
                    apply: true,
                    source: "project-script".to_string(),
                },
            ],
            plugin_classpath: vec![CanonicalPluginClasspathDependency {
                project_path: ":".to_string(),
                notation: "com.example:plugin:1.0".to_string(),
                source: "buildscript-classpath".to_string(),
            }],
            dependency_configurations: Vec::new(),
            toolchains: Vec::new(),
            invalidation_inputs: Vec::new(),
            metadata: Default::default(),
        };

        let resolution = resolve_native_plugin_contracts(&graph);

        assert_eq!(resolution.contracts.len(), 1);
        assert_eq!(resolution.contracts[0].plugin_id, "java");
        assert_eq!(resolution.rejections.len(), 2);
        assert!(resolution
            .rejections
            .iter()
            .any(|rejection| rejection.plugin_id == "com.example.custom"));
        assert!(resolution
            .rejections
            .iter()
            .any(|rejection| rejection.plugin_id == "com.example:plugin:1.0"));
    }
}
