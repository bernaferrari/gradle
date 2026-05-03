use std::collections::HashSet;

/// Whole-build plan admitted into the Rust execution kernel after JVM
/// configuration has produced a concrete task graph.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KernelBuildPlan {
    pub build_id: String,
    pub tasks: Vec<KernelTaskPlan>,
}

/// Task-level execution contract used for build-level admission only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KernelTaskPlan {
    pub task_path: String,
    pub task_type: String,
    pub execution_context_json: Option<String>,
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

    for task in &plan.tasks {
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
        if value
            .get("input_properties")
            .and_then(|v| v.as_object())
            .and_then(|props| props.get(key))
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

    use super::{admit_build_plan, KernelAdmission, KernelBuildPlan, KernelTaskPlan};

    fn native_types(types: &[&str]) -> HashSet<String> {
        types.iter().map(|ty| ty.to_string()).collect()
    }

    fn task(task_path: &str, task_type: &str, context: Option<String>) -> KernelTaskPlan {
        KernelTaskPlan {
            task_path: task_path.to_string(),
            task_type: task_type.to_string(),
            execution_context_json: context,
        }
    }

    #[test]
    fn admits_build_when_all_tasks_are_native_ready() {
        let plan = KernelBuildPlan {
            build_id: "build".to_string(),
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
}
