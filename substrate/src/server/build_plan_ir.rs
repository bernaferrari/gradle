use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::proto::{
    BuildPlan, BuildPlanDependency, BuildPlanEnvelope, BuildPlanProject, BuildPlanTask,
    BuildPlanToolchainRequest,
};

pub const BUILD_PLAN_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalBuildPlan {
    pub schema_version: u32,
    pub build_id: String,
    pub projects: Vec<CanonicalBuildPlanProject>,
    pub tasks: Vec<CanonicalBuildPlanTask>,
    pub dependencies: Vec<CanonicalBuildPlanDependency>,
    pub toolchains: Vec<CanonicalBuildPlanToolchainRequest>,
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalBuildPlanProject {
    pub path: String,
    pub name: String,
    pub project_dir: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalBuildPlanTask {
    pub path: String,
    pub project_path: String,
    pub implementation_id: String,
    pub depends_on: Vec<String>,
    pub inputs: BTreeMap<String, String>,
    pub outputs: Vec<String>,
    pub worker_isolation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalBuildPlanDependency {
    pub project_path: String,
    pub configuration: String,
    pub notation: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalBuildPlanToolchainRequest {
    pub language: String,
    pub version: String,
    pub vendor: String,
    pub implementation: String,
}

impl CanonicalBuildPlan {
    pub fn normalized(mut self) -> Self {
        self.projects.sort_by(|a, b| {
            (&a.path, &a.name, &a.project_dir).cmp(&(&b.path, &b.name, &b.project_dir))
        });

        for task in &mut self.tasks {
            task.depends_on.sort();
            task.outputs.sort();
        }
        self.tasks.sort_by(|a, b| {
            (&a.path, &a.project_path, &a.implementation_id).cmp(&(
                &b.path,
                &b.project_path,
                &b.implementation_id,
            ))
        });

        self.dependencies.sort_by(|a, b| {
            (&a.project_path, &a.configuration, &a.notation).cmp(&(
                &b.project_path,
                &b.configuration,
                &b.notation,
            ))
        });

        self.toolchains.sort_by(|a, b| {
            (&a.language, &a.version, &a.vendor, &a.implementation).cmp(&(
                &b.language,
                &b.version,
                &b.vendor,
                &b.implementation,
            ))
        });

        self
    }
}

pub fn validate_schema_version(plan: &CanonicalBuildPlan) -> Result<(), String> {
    if plan.schema_version != BUILD_PLAN_SCHEMA_VERSION {
        return Err(format!(
            "unsupported build plan schema version: {} (expected {})",
            plan.schema_version, BUILD_PLAN_SCHEMA_VERSION
        ));
    }
    Ok(())
}

pub fn canonical_json(plan: &CanonicalBuildPlan) -> Result<String, serde_json::Error> {
    serde_json::to_string(&plan.clone().normalized())
}

pub fn fingerprint_sha256_hex(plan: &CanonicalBuildPlan) -> Result<String, serde_json::Error> {
    let canonical = canonical_json(plan)?;
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn to_proto(plan: &CanonicalBuildPlan) -> BuildPlan {
    let normalized = plan.clone().normalized();
    BuildPlan {
        schema_version: normalized.schema_version,
        build_id: normalized.build_id,
        projects: normalized
            .projects
            .into_iter()
            .map(|p| BuildPlanProject {
                path: p.path,
                name: p.name,
                project_dir: p.project_dir,
            })
            .collect(),
        tasks: normalized
            .tasks
            .into_iter()
            .map(|t| BuildPlanTask {
                path: t.path,
                project_path: t.project_path,
                implementation_id: t.implementation_id,
                depends_on: t.depends_on,
                inputs: btree_to_hashmap(t.inputs),
                outputs: t.outputs,
                worker_isolation: t.worker_isolation,
            })
            .collect(),
        dependencies: normalized
            .dependencies
            .into_iter()
            .map(|d| BuildPlanDependency {
                project_path: d.project_path,
                configuration: d.configuration,
                notation: d.notation,
            })
            .collect(),
        toolchains: normalized
            .toolchains
            .into_iter()
            .map(|t| BuildPlanToolchainRequest {
                language: t.language,
                version: t.version,
                vendor: t.vendor,
                implementation: t.implementation,
            })
            .collect(),
        metadata: btree_to_hashmap(normalized.metadata),
    }
}

pub fn from_proto(plan: &BuildPlan) -> CanonicalBuildPlan {
    CanonicalBuildPlan {
        schema_version: plan.schema_version,
        build_id: plan.build_id.clone(),
        projects: plan
            .projects
            .iter()
            .map(|p| CanonicalBuildPlanProject {
                path: p.path.clone(),
                name: p.name.clone(),
                project_dir: p.project_dir.clone(),
            })
            .collect(),
        tasks: plan
            .tasks
            .iter()
            .map(|t| CanonicalBuildPlanTask {
                path: t.path.clone(),
                project_path: t.project_path.clone(),
                implementation_id: t.implementation_id.clone(),
                depends_on: t.depends_on.clone(),
                inputs: hashmap_to_btree(&t.inputs),
                outputs: t.outputs.clone(),
                worker_isolation: t.worker_isolation.clone(),
            })
            .collect(),
        dependencies: plan
            .dependencies
            .iter()
            .map(|d| CanonicalBuildPlanDependency {
                project_path: d.project_path.clone(),
                configuration: d.configuration.clone(),
                notation: d.notation.clone(),
            })
            .collect(),
        toolchains: plan
            .toolchains
            .iter()
            .map(|t| CanonicalBuildPlanToolchainRequest {
                language: t.language.clone(),
                version: t.version.clone(),
                vendor: t.vendor.clone(),
                implementation: t.implementation.clone(),
            })
            .collect(),
        metadata: hashmap_to_btree(&plan.metadata),
    }
    .normalized()
}

pub fn to_envelope(plan: &CanonicalBuildPlan) -> Result<BuildPlanEnvelope, serde_json::Error> {
    let proto_plan = to_proto(plan);
    let fingerprint = fingerprint_sha256_hex(plan)?;
    Ok(BuildPlanEnvelope {
        plan: Some(proto_plan),
        plan_fingerprint_sha256: fingerprint,
    })
}

fn btree_to_hashmap(map: BTreeMap<String, String>) -> HashMap<String, String> {
    map.into_iter().collect()
}

fn hashmap_to_btree(map: &HashMap<String, String>) -> BTreeMap<String, String> {
    map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_plan() -> CanonicalBuildPlan {
        CanonicalBuildPlan {
            schema_version: BUILD_PLAN_SCHEMA_VERSION,
            build_id: "build-123".to_string(),
            projects: vec![
                CanonicalBuildPlanProject {
                    path: ":app".to_string(),
                    name: "app".to_string(),
                    project_dir: "/repo/app".to_string(),
                },
                CanonicalBuildPlanProject {
                    path: ":".to_string(),
                    name: "root".to_string(),
                    project_dir: "/repo".to_string(),
                },
            ],
            tasks: vec![CanonicalBuildPlanTask {
                path: ":app:test".to_string(),
                project_path: ":app".to_string(),
                implementation_id: "org.gradle.api.tasks.testing.Test".to_string(),
                depends_on: vec![":app:classes".to_string(), ":app:testClasses".to_string()],
                inputs: BTreeMap::from([
                    ("testFramework".to_string(), "junit".to_string()),
                    ("forkEvery".to_string(), "0".to_string()),
                ]),
                outputs: vec![
                    "/repo/app/build/test-results".to_string(),
                    "/repo/app/build/reports/tests".to_string(),
                ],
                worker_isolation: "process".to_string(),
            }],
            dependencies: vec![CanonicalBuildPlanDependency {
                project_path: ":app".to_string(),
                configuration: "testRuntimeClasspath".to_string(),
                notation: "org.junit.jupiter:junit-jupiter:5.10.2".to_string(),
            }],
            toolchains: vec![CanonicalBuildPlanToolchainRequest {
                language: "java".to_string(),
                version: "21".to_string(),
                vendor: "temurin".to_string(),
                implementation: "jvm".to_string(),
            }],
            metadata: BTreeMap::from([
                ("gradleVersion".to_string(), "9.0.0".to_string()),
                ("requestedBy".to_string(), "unit-test".to_string()),
            ]),
        }
    }

    #[test]
    fn fingerprint_is_order_insensitive() {
        let plan_a = sample_plan();
        let mut plan_b = sample_plan();
        plan_b.projects.reverse();
        plan_b.tasks[0].depends_on.reverse();
        plan_b.tasks[0].outputs.reverse();

        let fp_a = fingerprint_sha256_hex(&plan_a).unwrap();
        let fp_b = fingerprint_sha256_hex(&plan_b).unwrap();

        assert_eq!(fp_a, fp_b);
    }

    #[test]
    fn schema_version_validation_rejects_unknown_version() {
        let mut plan = sample_plan();
        plan.schema_version = 999;
        let err = validate_schema_version(&plan).unwrap_err();
        assert!(err.contains("unsupported build plan schema version"));
    }

    #[test]
    fn proto_roundtrip_preserves_canonical_shape() {
        let plan = sample_plan();
        let proto = to_proto(&plan);
        let roundtrip = from_proto(&proto);
        assert_eq!(roundtrip, plan.normalized());
    }
}
