use std::collections::HashSet;

use super::dependency_solver::graph_builder;

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
            reasons.push(format!(
                "{} ({}) has no Rust executor",
                task.task_path, task.task_type
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
            reasons.push(format!(
                "dependency configuration '{}' uses unsupported feature '{}'",
                configuration.name, feature
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
    let Some(json) = context_json else {
        return None;
    };
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
        if input_properties
            .and_then(|props| props.get(key))
            .and_then(|v| v.as_str())
            == Some("true")
            || input_properties
                .and_then(|props| props.get(&format!("input.{key}")))
                .and_then(|v| v.as_str())
                == Some("true")
            || input_properties
                .and_then(|props| props.get(&format!("input_value.{key}")))
                .and_then(|v| v.as_str())
                == Some("true")
        {
            return Some(format!("unsupported contract marker '{}'", key));
        }
    }

    match task_type {
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
    use std::collections::HashSet;

    use super::{
        admit_build_plan, KernelAdmission, KernelBuildPlan, KernelDependencyConfiguration,
        KernelDependencyGraph, KernelDependencyRequest, KernelRepository, KernelTaskPlan,
    };

    fn native_types(types: &[&str]) -> HashSet<String> {
        types.iter().map(|ty| ty.to_string()).collect()
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
