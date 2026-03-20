use dashmap::DashMap;
use tonic::{Request, Response, Status};

use crate::proto::{
    incremental_compilation_service_server::IncrementalCompilationService, CompilationUnit,
    GetIncrementalStateRequest, GetIncrementalStateResponse, GetRebuildSetRequest,
    GetRebuildSetResponse, IncrementalState, RebuildDecision, RecordCompilationRequest,
    RecordCompilationResponse, RegisterSourceSetRequest, RegisterSourceSetResponse,
    SourceSetDescriptor,
};

/// Tracked source set for incremental compilation.
struct SourceSet {
    descriptor: SourceSetDescriptor,
    classpath_hash: String,
    units: Vec<CompilationUnit>,
    total_compile_time_ms: i64,
    incremental_compiles: i64,
    full_compiles: i64,
}

/// Rust-native incremental compilation service.
/// Tracks source changes and computes rebuild decisions.
pub struct IncrementalCompilationServiceImpl {
    source_sets: DashMap<String, SourceSet>,     // source_set_id -> SourceSet
    build_source_sets: DashMap<String, Vec<String>>, // build_id -> [source_set_id]
}

impl IncrementalCompilationServiceImpl {
    pub fn new() -> Self {
        Self {
            source_sets: DashMap::new(),
            build_source_sets: DashMap::new(),
        }
    }
}

#[tonic::async_trait]
impl IncrementalCompilationService for IncrementalCompilationServiceImpl {
    async fn register_source_set(
        &self,
        request: Request<RegisterSourceSetRequest>,
    ) -> Result<Response<RegisterSourceSetResponse>, Status> {
        let req = request.into_inner();

        let descriptor = req
            .source_set
            .ok_or_else(|| Status::invalid_argument("SourceSetDescriptor is required"))?;

        let source_set_id = descriptor.source_set_id.clone();
        let build_id = req.build_id.clone();
        let classpath_hash = descriptor.classpath_hash.clone();

        self.source_sets.insert(
            source_set_id.clone(),
            SourceSet {
                descriptor,
                classpath_hash,
                units: Vec::new(),
                total_compile_time_ms: 0,
                incremental_compiles: 0,
                full_compiles: 0,
            },
        );

        self.build_source_sets
            .entry(build_id)
            .or_insert_with(Vec::new)
            .push(source_set_id);

        Ok(Response::new(RegisterSourceSetResponse { accepted: true }))
    }

    async fn record_compilation(
        &self,
        request: Request<RecordCompilationRequest>,
    ) -> Result<Response<RecordCompilationResponse>, Status> {
        let req = request.into_inner();

        let unit = req
            .unit
            .ok_or_else(|| Status::invalid_argument("CompilationUnit is required"))?;

        let source_set_id = unit.source_set_id.clone();
        let changed;

        if let Some(mut ss) = self.source_sets.get_mut(&source_set_id) {
            // Check if this is a recompilation (unit already exists)
            let existing = ss.units.iter().find(|u| u.source_file == unit.source_file);
            changed = existing.is_some();

            if let Some(existing) = ss.units.iter_mut().find(|u| u.source_file == unit.source_file) {
                *existing = unit.clone();
                ss.incremental_compiles += 1;
            } else {
                ss.units.push(unit);
                ss.full_compiles += 1;
            }
        } else {
            changed = false;
        }

        Ok(Response::new(RecordCompilationResponse {
            accepted: true,
            changed,
        }))
    }

    async fn get_rebuild_set(
        &self,
        request: Request<GetRebuildSetRequest>,
    ) -> Result<Response<GetRebuildSetResponse>, Status> {
        let req = request.into_inner();

        let mut decisions = Vec::new();
        let mut must_recompile_count = 0i32;
        let mut up_to_date_count = 0i32;
        let total_sources;

        if let Some(ss) = self.source_sets.get(&req.source_set_id) {
            total_sources = ss.units.len() as i32;

            for unit in &ss.units {
                let source_changed = req.changed_files.iter().any(|f| f == &unit.source_file);
                let dependency_changed = unit
                    .dependencies
                    .iter()
                    .any(|dep| req.changed_files.iter().any(|f| f == dep));

                let must_recompile = source_changed || dependency_changed;

                if must_recompile {
                    must_recompile_count += 1;
                    let reason = if source_changed {
                        "source_changed".to_string()
                    } else {
                        "dependency_changed".to_string()
                    };
                    decisions.push(RebuildDecision {
                        source_file: unit.source_file.clone(),
                        reason,
                        must_recompile: true,
                    });
                } else {
                    up_to_date_count += 1;
                }
            }
        } else {
            total_sources = 0;
        }

        Ok(Response::new(GetRebuildSetResponse {
            decisions,
            total_sources,
            must_recompile_count,
            up_to_date_count,
        }))
    }

    async fn get_incremental_state(
        &self,
        request: Request<GetIncrementalStateRequest>,
    ) -> Result<Response<GetIncrementalStateResponse>, Status> {
        let req = request.into_inner();

        if let Some(ss) = self.source_sets.get(&req.source_set_id) {
            Ok(Response::new(GetIncrementalStateResponse {
                state: Some(IncrementalState {
                    source_set_id: ss.descriptor.source_set_id.clone(),
                    total_compiled: ss.units.len() as i32,
                    incrementally_compiled: ss.incremental_compiles as i32,
                    fully_recompiled: ss.full_compiles as i32,
                    total_compile_time_ms: ss.total_compile_time_ms,
                    units: ss.units.clone(),
                }),
            }))
        } else {
            Ok(Response::new(GetIncrementalStateResponse { state: None }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_source_set(id: &str, name: &str) -> SourceSetDescriptor {
        SourceSetDescriptor {
            source_set_id: id.to_string(),
            name: name.to_string(),
            source_dirs: vec!["src/main/java".to_string()],
            output_dirs: vec!["build/classes".to_string()],
            classpath_hash: "abc123".to_string(),
        }
    }

    fn make_compilation_unit(source_set_id: &str, source: &str, deps: Vec<&str>) -> CompilationUnit {
        CompilationUnit {
            source_set_id: source_set_id.to_string(),
            source_file: source.to_string(),
            output_class: source.replace(".java", ".class"),
            source_hash: format!("hash-{}", source),
            class_hash: format!("class-hash-{}", source),
            dependencies: deps.iter().map(|s| s.to_string()).collect(),
            compile_duration_ms: 100,
        }
    }

    #[tokio::test]
    async fn test_register_and_record() {
        let svc = IncrementalCompilationServiceImpl::new();

        svc.register_source_set(Request::new(RegisterSourceSetRequest {
            build_id: "build-1".to_string(),
            source_set: Some(make_source_set("ss1", "main")),
        }))
        .await
        .unwrap();

        svc.record_compilation(Request::new(RecordCompilationRequest {
            build_id: "build-1".to_string(),
            unit: Some(make_compilation_unit("ss1", "A.java", vec![])),
        }))
        .await
        .unwrap();

        svc.record_compilation(Request::new(RecordCompilationRequest {
            build_id: "build-1".to_string(),
            unit: Some(make_compilation_unit("ss1", "B.java", vec!["A.java"])),
        }))
        .await
        .unwrap();

        let state = svc
            .get_incremental_state(Request::new(GetIncrementalStateRequest {
                build_id: "build-1".to_string(),
                source_set_id: "ss1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner()
            .state
            .unwrap();

        assert_eq!(state.total_compiled, 2);
        assert_eq!(state.fully_recompiled, 2);
    }

    #[tokio::test]
    async fn test_rebuild_set_no_changes() {
        let svc = IncrementalCompilationServiceImpl::new();

        svc.register_source_set(Request::new(RegisterSourceSetRequest {
            build_id: "build-2".to_string(),
            source_set: Some(make_source_set("ss2", "main")),
        }))
        .await
        .unwrap();

        svc.record_compilation(Request::new(RecordCompilationRequest {
            build_id: "build-2".to_string(),
            unit: Some(make_compilation_unit("ss2", "X.java", vec![])),
        }))
        .await
        .unwrap();

        let rebuild = svc
            .get_rebuild_set(Request::new(GetRebuildSetRequest {
                build_id: "build-2".to_string(),
                source_set_id: "ss2".to_string(),
                changed_files: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(rebuild.total_sources, 1);
        assert_eq!(rebuild.must_recompile_count, 0);
        assert_eq!(rebuild.up_to_date_count, 1);
    }

    #[tokio::test]
    async fn test_rebuild_set_source_changed() {
        let svc = IncrementalCompilationServiceImpl::new();

        svc.register_source_set(Request::new(RegisterSourceSetRequest {
            build_id: "build-3".to_string(),
            source_set: Some(make_source_set("ss3", "main")),
        }))
        .await
        .unwrap();

        svc.record_compilation(Request::new(RecordCompilationRequest {
            build_id: "build-3".to_string(),
            unit: Some(make_compilation_unit("ss3", "P.java", vec![])),
        }))
        .await
        .unwrap();

        svc.record_compilation(Request::new(RecordCompilationRequest {
            build_id: "build-3".to_string(),
            unit: Some(make_compilation_unit("ss3", "Q.java", vec!["P.java"])),
        }))
        .await
        .unwrap();

        // P.java changed — P must recompile, Q depends on P so Q must also recompile
        let rebuild = svc
            .get_rebuild_set(Request::new(GetRebuildSetRequest {
                build_id: "build-3".to_string(),
                source_set_id: "ss3".to_string(),
                changed_files: vec!["P.java".to_string()],
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(rebuild.must_recompile_count, 2);
        assert_eq!(rebuild.up_to_date_count, 0);
    }

    #[tokio::test]
    async fn test_recompilation_tracking() {
        let svc = IncrementalCompilationServiceImpl::new();

        svc.register_source_set(Request::new(RegisterSourceSetRequest {
            build_id: "build-4".to_string(),
            source_set: Some(make_source_set("ss4", "main")),
        }))
        .await
        .unwrap();

        // First compile
        let resp = svc
            .record_compilation(Request::new(RecordCompilationRequest {
                build_id: "build-4".to_string(),
                unit: Some(make_compilation_unit("ss4", "R.java", vec![])),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.changed); // first time, not a recompilation

        // Recompile
        let resp2 = svc
            .record_compilation(Request::new(RecordCompilationRequest {
                build_id: "build-4".to_string(),
                unit: Some(make_compilation_unit("ss4", "R.java", vec![])),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp2.changed); // second time, is a recompilation

        let state = svc
            .get_incremental_state(Request::new(GetIncrementalStateRequest {
                build_id: "build-4".to_string(),
                source_set_id: "ss4".to_string(),
            }))
            .await
            .unwrap()
            .into_inner()
            .state
            .unwrap();

        assert_eq!(state.incrementally_compiled, 1);
        assert_eq!(state.fully_recompiled, 1);
    }

    #[tokio::test]
    async fn test_unknown_source_set() {
        let svc = IncrementalCompilationServiceImpl::new();

        let rebuild = svc
            .get_rebuild_set(Request::new(GetRebuildSetRequest {
                build_id: "build-5".to_string(),
                source_set_id: "nonexistent".to_string(),
                changed_files: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(rebuild.total_sources, 0);
    }
}
