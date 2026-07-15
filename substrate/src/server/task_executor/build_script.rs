use crate::server::task_executor::{TaskExecutor, TaskInput, TaskResult};

use std::collections::BTreeMap;


// Reporters: tracing 'buildscript-lowering' + 'vfs-taskexec-cross' (consumed by kernel/scheduler/problem_reporting cross-slice).
// Richer contract: exec hooks for build script phases + java_compile deepen (generated_sources, annotationProcessing, classpath handling).

pub fn apply_vfs_delta_to_build_script(
    delta_child_summaries: &BTreeMap<String, String>,
    build_script_path: &std::path::Path,
) -> bool {
    if delta_child_summaries.is_empty() {
        return false;
    }
    let script_str = build_script_path.to_string_lossy().to_lowercase();
    for (changed_path, _hash) in delta_child_summaries.iter() {
        let changed_lower = changed_path.to_lowercase();
        if script_str.contains(&changed_lower)
            || changed_lower.contains("build.gradle")
            || changed_lower.contains("build.gradle.kts")
            || changed_lower.contains("src/main")
            || changed_lower.contains("buildscript")
            || changed_lower.contains("kotlin")
            || changed_lower.contains("java")
        {
            tracing::info!(target: "buildscript-lowering", vfs_taskexec_cross = true, script = %build_script_path.display(), changed = %changed_path, "VFS delta intersects build script / java compile inputs — re-execution likely required");
            return true;
        }
    }
    false
}

/// Build script / annotation processing task executor (richer lowering + VFS cross).
pub struct BuildScriptTaskExecutor;

impl Default for BuildScriptTaskExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl BuildScriptTaskExecutor {
    pub fn new() -> Self {
        Self
    }
}

#[tonic::async_trait]
impl TaskExecutor for BuildScriptTaskExecutor {
    fn task_type(&self) -> &str {
        "BuildScript"
    }

    async fn execute(&self, input: &TaskInput) -> TaskResult {
        let start = std::time::Instant::now();
        let mut result = TaskResult::default();

        // Richer contract scaffolding for build script phases (kotlinc/gradle-api fidelity, generated sources, annotationProcessing).
        tracing::info!(target: "buildscript-lowering", vfs_taskexec_cross = true, "BuildScriptTaskExecutor execute (shadow) with richer contracts + VFS delta prep");

        // Placeholder richer lowering (real impl would invoke substrate build script engine with VFS snapshot delta for incremental).
        result.success = true;
        result.duration_ms = start.elapsed().as_millis() as u64;
        result.files_processed = input.source_files.len() as u64;
        result
    }
}
