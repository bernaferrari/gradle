use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::client::jvm_host_bridge::JvmHostBridge;
use crate::proto::{GetBuildEnvironmentResponse, GetBuildModelResponse};

use super::build_plan_ir::{
    fingerprint_sha256_hex, validate_schema_version, CanonicalBuildPlan,
    CanonicalBuildPlanProject, CanonicalBuildPlanToolchainRequest, BUILD_PLAN_SCHEMA_VERSION,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BuildPlanShadowArtifact {
    pub plan: CanonicalBuildPlan,
    pub fingerprint_sha256: String,
    pub stored_at_ms: i64,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct BuildPlanShadowStore {
    root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildPlanShadowDiffReport {
    pub build_id: String,
    pub mismatches: Vec<String>,
}

impl BuildPlanShadowDiffReport {
    pub fn is_match(&self) -> bool {
        self.mismatches.is_empty()
    }
}

impl BuildPlanShadowStore {
    pub fn new(config_cache_dir: PathBuf) -> Self {
        let root = config_cache_dir.join("build-plan-shadow");
        std::fs::create_dir_all(&root).ok();
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn persist_plan(
        &self,
        plan: &CanonicalBuildPlan,
        source: &str,
    ) -> Result<PathBuf, Box<dyn std::error::Error + Send + Sync>> {
        validate_schema_version(plan)
            .map_err(|e| format!("build plan schema validation failed: {}", e))?;
        let fingerprint = fingerprint_sha256_hex(plan)?;

        let artifact = BuildPlanShadowArtifact {
            plan: plan.clone().normalized(),
            fingerprint_sha256: fingerprint,
            stored_at_ms: now_ms(),
            source: source.to_string(),
        };

        let path = self.artifact_path(&artifact.plan.build_id);
        let payload = serde_json::to_vec_pretty(&artifact)?;
        std::fs::write(&path, payload)?;
        Ok(path)
    }

    pub fn load_plan(
        &self,
        build_id: &str,
    ) -> Result<Option<BuildPlanShadowArtifact>, Box<dyn std::error::Error + Send + Sync>> {
        let path = self.artifact_path_for_build_id(build_id);
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path)?;
        let artifact: BuildPlanShadowArtifact = serde_json::from_slice(&bytes)?;
        Ok(Some(artifact))
    }

    pub fn artifact_path_for_build_id(&self, build_id: &str) -> PathBuf {
        self.root.join(keyed_artifact_filename(build_id))
    }

    fn artifact_path(&self, build_id: &str) -> PathBuf {
        self.artifact_path_for_build_id(build_id)
    }
}

pub async fn capture_and_persist_shadow_from_jvm(
    bridge: &JvmHostBridge,
    store: &BuildPlanShadowStore,
    build_id: &str,
) -> Result<Option<PathBuf>, Box<dyn std::error::Error + Send + Sync>> {
    let Some(model) = bridge.get_build_model(build_id).await? else {
        return Ok(None);
    };
    let env = bridge.get_build_environment().await?;

    let plan = canonical_plan_from_jvm(build_id, &model, env.as_ref());
    let path = store.persist_plan(&plan, "jvm-host-shadow")?;
    if let Some(artifact) = store.load_plan(build_id)? {
        let diff = diff_expected_vs_artifact(&plan, &artifact);
        if !diff.is_match() {
            return Err(format!(
                "shadow artifact persisted with mismatches: {}",
                diff.mismatches.join("; ")
            )
            .into());
        }
    }
    Ok(Some(path))
}

pub async fn verify_shadow_against_jvm(
    bridge: &JvmHostBridge,
    store: &BuildPlanShadowStore,
    build_id: &str,
) -> Result<BuildPlanShadowDiffReport, Box<dyn std::error::Error + Send + Sync>> {
    let Some(model) = bridge.get_build_model(build_id).await? else {
        return Ok(BuildPlanShadowDiffReport {
            build_id: build_id.to_string(),
            mismatches: vec!["JVM build model unavailable".to_string()],
        });
    };
    let env = bridge.get_build_environment().await?;
    let expected = canonical_plan_from_jvm(build_id, &model, env.as_ref());
    let Some(artifact) = store.load_plan(build_id)? else {
        return Ok(BuildPlanShadowDiffReport {
            build_id: build_id.to_string(),
            mismatches: vec!["missing shadow artifact".to_string()],
        });
    };

    Ok(diff_expected_vs_artifact(&expected, &artifact))
}

pub fn canonical_plan_from_jvm(
    build_id: &str,
    model: &GetBuildModelResponse,
    env: Option<&GetBuildEnvironmentResponse>,
) -> CanonicalBuildPlan {
    let mut projects: Vec<CanonicalBuildPlanProject> = model
        .projects
        .iter()
        .map(|p| CanonicalBuildPlanProject {
            path: p.path.clone(),
            name: p.name.clone(),
            project_dir: infer_project_dir(&p.build_file),
        })
        .collect();

    if projects.is_empty() {
        projects.push(CanonicalBuildPlanProject {
            path: ":".to_string(),
            name: "root".to_string(),
            project_dir: String::new(),
        });
    }

    let mut metadata = std::collections::BTreeMap::new();
    metadata.insert("source".to_string(), "jvm-host-shadow".to_string());
    metadata.insert("projectCount".to_string(), model.projects.len().to_string());
    if let Some(env) = env {
        if !env.gradle_version.is_empty() {
            metadata.insert("gradleVersion".to_string(), env.gradle_version.clone());
        }
        if !env.java_version.is_empty() {
            metadata.insert("javaVersion".to_string(), env.java_version.clone());
        }
    }

    let mut toolchains = Vec::new();
    if let Some(env) = env {
        if !env.java_version.is_empty() {
            toolchains.push(CanonicalBuildPlanToolchainRequest {
                language: "java".to_string(),
                version: normalize_java_version(&env.java_version),
                vendor: "jvm-host".to_string(),
                implementation: "jvm".to_string(),
            });
        }
    }

    CanonicalBuildPlan {
        schema_version: BUILD_PLAN_SCHEMA_VERSION,
        build_id: build_id.to_string(),
        projects,
        tasks: Vec::new(),
        dependencies: Vec::new(),
        toolchains,
        metadata,
    }
    .normalized()
}

fn infer_project_dir(build_file: &str) -> String {
    if build_file.is_empty() {
        return String::new();
    }
    let path = Path::new(build_file);
    path.parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn normalize_java_version(raw: &str) -> String {
    if let Some(major) = raw.split('.').next() {
        return major.to_string();
    }
    raw.to_string()
}

fn sanitize_key(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn keyed_artifact_filename(build_id: &str) -> String {
    format!(
        "{}-{}.json",
        sanitize_key(build_id),
        stable_short_hash(build_id)
    )
}

fn stable_short_hash(raw: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    let digest = hasher.finalize();
    super::cache::hex::encode(&digest[..8])
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn diff_expected_vs_artifact(
    expected: &CanonicalBuildPlan,
    artifact: &BuildPlanShadowArtifact,
) -> BuildPlanShadowDiffReport {
    let expected = expected.clone().normalized();
    let actual = artifact.plan.clone().normalized();
    let mut mismatches = Vec::new();

    if expected.build_id != actual.build_id {
        mismatches.push(format!(
            "build_id mismatch: expected '{}' got '{}'",
            expected.build_id, actual.build_id
        ));
    }
    if expected.schema_version != actual.schema_version {
        mismatches.push(format!(
            "schema_version mismatch: expected {} got {}",
            expected.schema_version, actual.schema_version
        ));
    }
    if expected.projects != actual.projects {
        // Field-level diff: identify specific project differences
        let expected_paths: std::collections::HashSet<&str> =
            expected.projects.iter().map(|p| p.path.as_str()).collect();
        let actual_paths: std::collections::HashSet<&str> =
            actual.projects.iter().map(|p| p.path.as_str()).collect();

        let only_expected: Vec<&str> = expected_paths.difference(&actual_paths).copied().collect();
        let only_actual: Vec<&str> = actual_paths.difference(&expected_paths).copied().collect();

        if !only_expected.is_empty() {
            mismatches.push(format!(
                "projects only in expected: [{}]",
                only_expected.join(", ")
            ));
        }
        if !only_actual.is_empty() {
            mismatches.push(format!(
                "projects only in actual: [{}]",
                only_actual.join(", ")
            ));
        }

        // Check for field-level differences in common projects
        for p in &expected.projects {
            if let Some(a) = actual.projects.iter().find(|a| a.path == p.path) {
                if p.name != a.name {
                    mismatches.push(format!(
                        "project '{}' name mismatch: expected '{}' got '{}'",
                        p.path, p.name, a.name
                    ));
                }
                if p.project_dir != a.project_dir {
                    mismatches.push(format!(
                        "project '{}' project_dir mismatch: expected '{}' got '{}'",
                        p.path, p.project_dir, a.project_dir
                    ));
                }
            }
        }

        if only_expected.is_empty() && only_actual.is_empty() && mismatches.iter().all(|m| !m.starts_with("project")) {
            mismatches.push(format!(
                "projects mismatch: expected {} entries got {}",
                expected.projects.len(),
                actual.projects.len()
            ));
        }
    }
    if expected.tasks != actual.tasks {
        let expected_tasks: std::collections::HashSet<&str> =
            expected.tasks.iter().map(|t| t.path.as_str()).collect();
        let actual_tasks: std::collections::HashSet<&str> =
            actual.tasks.iter().map(|t| t.path.as_str()).collect();

        let only_expected: Vec<&str> = expected_tasks.difference(&actual_tasks).copied().collect();
        let only_actual: Vec<&str> = actual_tasks.difference(&expected_tasks).copied().collect();

        if !only_expected.is_empty() {
            mismatches.push(format!(
                "tasks only in expected: [{}]",
                only_expected.join(", ")
            ));
        }
        if !only_actual.is_empty() {
            mismatches.push(format!(
                "tasks only in actual: [{}]",
                only_actual.join(", ")
            ));
        }
        if only_expected.is_empty() && only_actual.is_empty() {
            mismatches.push(format!(
                "tasks field-level mismatch: expected {} entries got {}",
                expected.tasks.len(),
                actual.tasks.len()
            ));
        }
    }
    if expected.dependencies != actual.dependencies {
        let expected_deps: std::collections::HashSet<String> = expected
            .dependencies
            .iter()
            .map(|d| format!("{}|{}|{}", d.project_path, d.configuration, d.notation))
            .collect();
        let actual_deps: std::collections::HashSet<String> = actual
            .dependencies
            .iter()
            .map(|d| format!("{}|{}|{}", d.project_path, d.configuration, d.notation))
            .collect();

        let only_expected: Vec<String> = expected_deps.iter()
            .filter(|s| !actual_deps.contains(*s))
            .cloned()
            .collect();
        let only_actual: Vec<String> = actual_deps.iter()
            .filter(|s| !expected_deps.contains(*s))
            .cloned()
            .collect();

        if !only_expected.is_empty() {
            mismatches.push(format!(
                "dependencies only in expected: [{}]",
                only_expected.join(", ")
            ));
        }
        if !only_actual.is_empty() {
            mismatches.push(format!(
                "dependencies only in actual: [{}]",
                only_actual.join(", ")
            ));
        }
        if only_expected.is_empty() && only_actual.is_empty() {
            mismatches.push(format!(
                "dependencies mismatch: expected {} entries got {}",
                expected.dependencies.len(),
                actual.dependencies.len()
            ));
        }
    }
    if expected.toolchains != actual.toolchains {
        let expected_tc: std::collections::HashSet<&str> = expected
            .toolchains
            .iter()
            .map(|t| t.language.as_str())
            .collect();
        let actual_tc: std::collections::HashSet<&str> = actual
            .toolchains
            .iter()
            .map(|t| t.language.as_str())
            .collect();

        let only_expected: Vec<&str> = expected_tc.difference(&actual_tc).copied().collect();
        let only_actual: Vec<&str> = actual_tc.difference(&expected_tc).copied().collect();

        if !only_expected.is_empty() {
            mismatches.push(format!(
                "toolchains only in expected: [{}]",
                only_expected.join(", ")
            ));
        }
        if !only_actual.is_empty() {
            mismatches.push(format!(
                "toolchains only in actual: [{}]",
                only_actual.join(", ")
            ));
        }
        if only_expected.is_empty() && only_actual.is_empty() {
            mismatches.push(format!(
                "toolchains field-level mismatch: expected {} entries got {}",
                expected.toolchains.len(),
                actual.toolchains.len()
            ));
        }
    }
    if expected.metadata != actual.metadata {
        let only_expected: Vec<String> = expected
            .metadata
            .keys()
            .filter(|k| !actual.metadata.contains_key(*k))
            .map(|k| k.to_string())
            .collect();
        let only_actual: Vec<String> = actual
            .metadata
            .keys()
            .filter(|k| !expected.metadata.contains_key(*k))
            .map(|k| k.to_string())
            .collect();
        let value_diffs: Vec<String> = expected
            .metadata
            .iter()
            .filter(|(k, v)| actual.metadata.get(*k) != Some(v))
            .map(|(k, v)| format!("{}: expected='{}' got='{}'", k, v, actual.metadata.get(k).unwrap_or(&String::new())))
            .collect();

        if !only_expected.is_empty() {
            mismatches.push(format!("metadata keys only in expected: [{}]", only_expected.join(", ")));
        }
        if !only_actual.is_empty() {
            mismatches.push(format!("metadata keys only in actual: [{}]", only_actual.join(", ")));
        }
        for d in &value_diffs {
            mismatches.push(format!("metadata {}", d));
        }
        if only_expected.is_empty() && only_actual.is_empty() && value_diffs.is_empty() {
            mismatches.push("metadata mismatch".to_string());
        }
    }

    match fingerprint_sha256_hex(&actual) {
        Ok(fp) if fp != artifact.fingerprint_sha256 => mismatches.push(format!(
            "fingerprint mismatch: expected '{}' got '{}'",
            fp, artifact.fingerprint_sha256
        )),
        Err(err) => mismatches.push(format!("fingerprint computation failed: {}", err)),
        _ => {}
    }

    BuildPlanShadowDiffReport {
        build_id: expected.build_id,
        mismatches,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_plan_from_jvm_extracts_projects_and_toolchain() {
        let model = GetBuildModelResponse {
            projects: vec![crate::proto::ProjectModel {
                path: ":app".to_string(),
                name: "app".to_string(),
                build_file: "/repo/app/build.gradle.kts".to_string(),
                subprojects: vec![],
            }],
        };
        let env = GetBuildEnvironmentResponse {
            java_version: "21.0.4".to_string(),
            java_home: String::new(),
            gradle_version: "9.0.0".to_string(),
            os_name: String::new(),
            os_arch: String::new(),
            available_processors: 0,
            max_memory_bytes: 0,
            system_properties: Default::default(),
        };

        let plan = canonical_plan_from_jvm("build-abc", &model, Some(&env));
        assert_eq!(plan.schema_version, BUILD_PLAN_SCHEMA_VERSION);
        assert_eq!(plan.build_id, "build-abc");
        assert_eq!(plan.projects.len(), 1);
        assert_eq!(plan.projects[0].project_dir, "/repo/app");
        assert_eq!(plan.toolchains.len(), 1);
        assert_eq!(plan.toolchains[0].version, "21");
    }

    #[test]
    fn persist_and_load_shadow_artifact() {
        let temp = tempfile::tempdir().unwrap();
        let store = BuildPlanShadowStore::new(temp.path().to_path_buf());
        let plan = CanonicalBuildPlan {
            schema_version: BUILD_PLAN_SCHEMA_VERSION,
            build_id: "build:1".to_string(),
            projects: vec![CanonicalBuildPlanProject {
                path: ":".to_string(),
                name: "root".to_string(),
                project_dir: "/repo".to_string(),
            }],
            tasks: Vec::new(),
            dependencies: Vec::new(),
            toolchains: Vec::new(),
            metadata: std::collections::BTreeMap::new(),
        };

        let path = store.persist_plan(&plan, "test").unwrap();
        assert!(path.exists());

        let loaded = store.load_plan("build:1").unwrap().unwrap();
        assert_eq!(loaded.plan.build_id, "build:1");
        assert_eq!(loaded.source, "test");
        assert!(!loaded.fingerprint_sha256.is_empty());
    }

    #[test]
    fn diff_report_detects_modified_artifact() {
        let expected = CanonicalBuildPlan {
            schema_version: BUILD_PLAN_SCHEMA_VERSION,
            build_id: "build-x".to_string(),
            projects: vec![CanonicalBuildPlanProject {
                path: ":".to_string(),
                name: "root".to_string(),
                project_dir: "/repo".to_string(),
            }],
            tasks: Vec::new(),
            dependencies: Vec::new(),
            toolchains: Vec::new(),
            metadata: std::collections::BTreeMap::new(),
        };

        let mut artifact = BuildPlanShadowArtifact {
            plan: expected.clone(),
            fingerprint_sha256: fingerprint_sha256_hex(&expected).unwrap(),
            stored_at_ms: 0,
            source: "test".to_string(),
        };
        artifact.plan.projects[0].name = "mutated".to_string();

        let report = diff_expected_vs_artifact(&expected, &artifact);
        assert!(!report.is_match());
        assert!(!report.mismatches.is_empty());
    }

    #[test]
    fn artifact_filename_is_collision_safe_for_similar_sanitized_keys() {
        let temp = tempfile::tempdir().unwrap();
        let store = BuildPlanShadowStore::new(temp.path().to_path_buf());
        let a = store.artifact_path_for_build_id("build/a");
        let b = store.artifact_path_for_build_id("build:a");
        assert_ne!(a, b);
    }
}
