use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::build_plan_ir::CanonicalBuildPlan;

pub const BUILD_GRAPH_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalBuildGraph {
    pub schema_version: u32,
    pub build_id: String,
    pub projects: Vec<CanonicalBuildGraphProject>,
    pub tasks: Vec<CanonicalBuildGraphTask>,
    pub edges: Vec<CanonicalBuildGraphEdge>,
    pub dependency_requests: Vec<CanonicalBuildGraphDependencyRequest>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalBuildGraphProject {
    pub path: String,
    pub name: String,
    pub project_dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalBuildGraphTask {
    pub path: String,
    pub project_path: String,
    pub implementation_id: String,
    pub action_kind: String,
    pub cacheability: String,
    pub worker_isolation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalBuildGraphEdge {
    pub from: String,
    pub to: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalBuildGraphDependencyRequest {
    pub project_path: String,
    pub configuration: String,
    pub notation: String,
    pub kind: String,
}

impl CanonicalBuildGraph {
    pub fn normalized(mut self) -> Self {
        self.normalize_mut();
        self
    }

    pub fn normalize_mut(&mut self) {
        self.projects.sort_unstable_by(|a, b| {
            (&a.path, &a.name, &a.project_dir).cmp(&(&b.path, &b.name, &b.project_dir))
        });
        self.tasks.sort_unstable_by(|a, b| {
            (&a.path, &a.project_path, &a.implementation_id).cmp(&(
                &b.path,
                &b.project_path,
                &b.implementation_id,
            ))
        });
        self.edges
            .sort_unstable_by(|a, b| (&a.from, &a.to, &a.kind).cmp(&(&b.from, &b.to, &b.kind)));
        self.dependency_requests.sort_unstable_by(|a, b| {
            (&a.project_path, &a.configuration, &a.kind, &a.notation).cmp(&(
                &b.project_path,
                &b.configuration,
                &b.kind,
                &b.notation,
            ))
        });
    }
}

pub fn from_build_plan(plan: &CanonicalBuildPlan) -> CanonicalBuildGraph {
    let mut graph =
        CanonicalBuildGraph {
            schema_version: BUILD_GRAPH_SCHEMA_VERSION,
            build_id: plan.build_id.clone(),
            projects: plan
                .projects
                .iter()
                .map(|project| CanonicalBuildGraphProject {
                    path: project.path.clone(),
                    name: project.name.clone(),
                    project_dir: project.project_dir.clone(),
                })
                .collect(),
            tasks: plan
                .tasks
                .iter()
                .map(|task| CanonicalBuildGraphTask {
                    path: task.path.clone(),
                    project_path: task.project_path.clone(),
                    implementation_id: task.implementation_id.clone(),
                    action_kind: task.action_kind.clone(),
                    cacheability: task.cacheability.clone(),
                    worker_isolation: task.worker_isolation.clone(),
                })
                .collect(),
            edges: plan
                .tasks
                .iter()
                .flat_map(|task| {
                    task.depends_on
                        .iter()
                        .map(|dependency| CanonicalBuildGraphEdge {
                            from: task.path.clone(),
                            to: dependency.clone(),
                            kind: "depends_on".to_string(),
                        })
                        .chain(task.should_run_after.iter().map(|dependency| {
                            CanonicalBuildGraphEdge {
                                from: task.path.clone(),
                                to: dependency.clone(),
                                kind: "should_run_after".to_string(),
                            }
                        }))
                        .chain(task.must_run_after.iter().map(|dependency| {
                            CanonicalBuildGraphEdge {
                                from: task.path.clone(),
                                to: dependency.clone(),
                                kind: "must_run_after".to_string(),
                            }
                        }))
                        .chain(
                            task.finalized_by
                                .iter()
                                .map(|finalizer| CanonicalBuildGraphEdge {
                                    from: task.path.clone(),
                                    to: finalizer.clone(),
                                    kind: "finalized_by".to_string(),
                                }),
                        )
                })
                .collect(),
            dependency_requests: plan
                .dependencies
                .iter()
                .map(|dependency| CanonicalBuildGraphDependencyRequest {
                    project_path: dependency.project_path.clone(),
                    configuration: dependency.configuration.clone(),
                    notation: dependency.notation.clone(),
                    kind: dependency.kind.clone(),
                })
                .collect(),
            metadata: BTreeMap::from([
                ("source".to_string(), "build-plan-ir".to_string()),
                (
                    "buildPlanSchemaVersion".to_string(),
                    plan.schema_version.to_string(),
                ),
            ]),
        };
    graph.normalize_mut();
    graph
}

pub fn validate_schema_version(graph: &CanonicalBuildGraph) -> Result<(), String> {
    if graph.schema_version != BUILD_GRAPH_SCHEMA_VERSION {
        return Err(format!(
            "unsupported build graph schema version: {} (expected {})",
            graph.schema_version, BUILD_GRAPH_SCHEMA_VERSION
        ));
    }
    Ok(())
}

pub fn validate_graph(graph: &CanonicalBuildGraph) -> Result<(), String> {
    validate_schema_version(graph)?;
    if graph.build_id.trim().is_empty() {
        return Err("build graph build_id must not be empty".to_string());
    }
    let project_paths = graph
        .projects
        .iter()
        .map(|project| project.path.as_str())
        .collect::<BTreeSet<_>>();
    for project in &graph.projects {
        if project.path.trim().is_empty() {
            return Err("build graph project path must not be empty".to_string());
        }
    }
    for task in &graph.tasks {
        if task.path.trim().is_empty() {
            return Err("build graph task path must not be empty".to_string());
        }
        if !project_paths.is_empty()
            && !task.project_path.is_empty()
            && !project_paths.contains(task.project_path.as_str())
        {
            return Err(format!(
                "build graph task '{}' references missing project '{}'",
                task.path, task.project_path
            ));
        }
    }
    for edge in &graph.edges {
        if edge.from.trim().is_empty() || edge.to.trim().is_empty() {
            return Err(format!(
                "build graph edge '{} -> {}' must have non-empty endpoints",
                edge.from, edge.to
            ));
        }
    }
    Ok(())
}

pub fn fingerprint_normalized(graph: &CanonicalBuildGraph) -> Result<String, serde_json::Error> {
    let canonical = serde_json::to_string(graph)?;
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    let digest = hasher.finalize();
    Ok(crate::server::cache::hex::encode(digest.as_ref()))
}

pub fn fingerprint_sha256_hex(graph: &CanonicalBuildGraph) -> Result<String, serde_json::Error> {
    let mut graph = graph.clone();
    graph.normalize_mut();
    fingerprint_normalized(&graph)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::build_plan_ir::{
        CanonicalBuildPlanDependency, CanonicalBuildPlanProject, CanonicalBuildPlanTask,
        BUILD_PLAN_SCHEMA_VERSION,
    };

    fn sample_plan() -> CanonicalBuildPlan {
        CanonicalBuildPlan {
            schema_version: BUILD_PLAN_SCHEMA_VERSION,
            build_id: "stable-root-test".to_string(),
            projects: vec![CanonicalBuildPlanProject {
                path: ":".to_string(),
                name: "root".to_string(),
                project_dir: "/repo".to_string(),
            }],
            tasks: vec![
                CanonicalBuildPlanTask {
                    path: ":classes".to_string(),
                    project_path: ":".to_string(),
                    implementation_id: "Classes".to_string(),
                    depends_on: vec![":compileJava".to_string()],
                    inputs: BTreeMap::new(),
                    outputs: Vec::new(),
                    worker_isolation: "none".to_string(),
                    should_run_after: Vec::new(),
                    must_run_after: Vec::new(),
                    finalized_by: Vec::new(),
                    cacheability: "cacheable".to_string(),
                    local_state: Vec::new(),
                    destroyables: Vec::new(),
                    action_kind: "lifecycle".to_string(),
                    input_specs: Vec::new(),
                    output_specs: Vec::new(),
                    environment_inputs: Vec::new(),
                    system_property_inputs: Vec::new(),
                    diagnostics: Vec::new(),
                },
                CanonicalBuildPlanTask {
                    path: ":compileJava".to_string(),
                    project_path: ":".to_string(),
                    implementation_id: "JavaCompile".to_string(),
                    depends_on: Vec::new(),
                    inputs: BTreeMap::new(),
                    outputs: Vec::new(),
                    worker_isolation: "process".to_string(),
                    should_run_after: Vec::new(),
                    must_run_after: Vec::new(),
                    finalized_by: Vec::new(),
                    cacheability: "cacheable".to_string(),
                    local_state: Vec::new(),
                    destroyables: Vec::new(),
                    action_kind: "java_compile".to_string(),
                    input_specs: Vec::new(),
                    output_specs: Vec::new(),
                    environment_inputs: Vec::new(),
                    system_property_inputs: Vec::new(),
                    diagnostics: Vec::new(),
                },
            ],
            dependencies: vec![CanonicalBuildPlanDependency {
                project_path: ":".to_string(),
                configuration: "compileClasspath".to_string(),
                notation: "org.example:lib:1".to_string(),
                kind: "external-module".to_string(),
                repositories: Vec::new(),
                unsupported_features: Vec::new(),
            }],
            toolchains: Vec::new(),
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn derives_task_edges_and_dependency_requests_from_build_plan() {
        let graph = from_build_plan(&sample_plan());

        assert_eq!(graph.build_id, "stable-root-test");
        assert_eq!(graph.projects.len(), 1);
        assert_eq!(graph.tasks.len(), 2);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].from, ":classes");
        assert_eq!(graph.edges[0].to, ":compileJava");
        assert_eq!(graph.dependency_requests.len(), 1);
        validate_graph(&graph).unwrap();
    }

    #[test]
    fn fingerprint_is_order_insensitive() {
        let plan = sample_plan();
        let mut reordered = plan.clone();
        reordered.tasks.reverse();

        assert_eq!(
            fingerprint_sha256_hex(&from_build_plan(&plan)).unwrap(),
            fingerprint_sha256_hex(&from_build_plan(&reordered)).unwrap()
        );
    }

    #[test]
    fn rejects_edges_with_empty_endpoints() {
        let mut graph = from_build_plan(&sample_plan());
        graph.edges.push(CanonicalBuildGraphEdge {
            from: ":classes".to_string(),
            to: String::new(),
            kind: "depends_on".to_string(),
        });

        let error = validate_graph(&graph).unwrap_err();

        assert!(error.contains("non-empty endpoints"));
    }
}
