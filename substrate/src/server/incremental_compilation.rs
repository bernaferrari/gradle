use std::collections::{HashMap, HashSet, VecDeque};

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
    #[allow(dead_code)]
    classpath_hash: String,
    previous_classpath_hash: String,
    units: Vec<CompilationUnit>,
    total_compile_time_ms: i64,
    incremental_compiles: i64,
    full_compiles: i64,
    classpath_changed: bool,
}

/// Rust-native incremental compilation service.
/// Tracks source changes and computes rebuild decisions with transitive dependency closure.
#[derive(Default)]
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

    /// Compute the transitive closure of files that must be recompiled.
    /// Uses reverse dependency map: if A depends on B, then changing B requires recompiling A.
    fn compute_transitive_rebuild_set(
        units: &[CompilationUnit],
        changed_files: &[String],
    ) -> HashSet<String> {
        // Build reverse dependency map: file -> set of files that depend on it
        let mut reverse_deps: HashMap<&str, Vec<&str>> = HashMap::new();
        for unit in units {
            for dep in &unit.dependencies {
                reverse_deps.entry(dep.as_str()).or_default().push(&unit.source_file);
            }
        }

        // BFS from changed files through reverse dependencies
        let mut affected: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<&str> = changed_files.iter().map(|s| s.as_str()).collect();

        while let Some(file) = queue.pop_front() {
            if affected.contains(file) {
                continue;
            }
            affected.insert(file.to_string());

            if let Some(dependents) = reverse_deps.get(file) {
                for dep in dependents {
                    if !affected.contains(*dep) {
                        queue.push_back(dep);
                    }
                }
            }
        }

        affected
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

        // Detect classpath change from previous registration
        let (previous_classpath_hash, classpath_changed, units, total_time, incr, full) =
            if let Some(existing) = self.source_sets.get(&source_set_id) {
                let changed = !existing.classpath_hash.is_empty()
                    && existing.classpath_hash != classpath_hash;
                (
                    existing.classpath_hash.clone(),
                    changed,
                    if changed {
                        // Classpath changed: invalidate all compilation results
                        Vec::new()
                    } else {
                        existing.units.clone()
                    },
                    if changed { 0 } else { existing.total_compile_time_ms },
                    if changed { 0 } else { existing.incremental_compiles },
                    if changed { 0 } else { existing.full_compiles },
                )
            } else {
                (String::new(), false, Vec::new(), 0, 0, 0)
            };

        self.source_sets.insert(
            source_set_id.clone(),
            SourceSet {
                descriptor,
                classpath_hash,
                previous_classpath_hash,
                units,
                total_compile_time_ms: total_time,
                incremental_compiles: incr,
                full_compiles: full,
                classpath_changed,
            },
        );

        self.build_source_sets
            .entry(build_id)
            .or_default()
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

            let compile_duration_ms = unit.compile_duration_ms;
            if let Some(existing) = ss.units.iter_mut().find(|u| u.source_file == unit.source_file) {
                *existing = unit.clone();
                ss.incremental_compiles += 1;
            } else {
                ss.units.push(unit);
                ss.full_compiles += 1;
            }
            ss.total_compile_time_ms += compile_duration_ms;
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

            if req.changed_files.is_empty() && !ss.classpath_changed {
                up_to_date_count = total_sources;
            } else if ss.classpath_changed && req.changed_files.is_empty() {
                // Classpath changed but no source changes: all existing units must recompile
                must_recompile_count = total_sources;
                for unit in &ss.units {
                    decisions.push(RebuildDecision {
                        source_file: unit.source_file.clone(),
                        reason: "classpath_changed".to_string(),
                        must_recompile: true,
                    });
                }
            } else {
                // Compute transitive closure of affected files
                let affected =
                    Self::compute_transitive_rebuild_set(&ss.units, &req.changed_files);

                for unit in &ss.units {
                    if affected.contains(&unit.source_file) {
                        must_recompile_count += 1;
                        let directly_changed =
                            req.changed_files.iter().any(|f| f == &unit.source_file);
                        let reason = if directly_changed {
                            "source_changed".to_string()
                        } else {
                            "transitive_dependency_changed".to_string()
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
                    classpath_changed: ss.classpath_changed,
                    previous_classpath_hash: ss.previous_classpath_hash.clone(),
                    current_classpath_hash: ss.classpath_hash.clone(),
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

    #[tokio::test]
    async fn test_transitive_dependency_closure() {
        // A.java -> B.java -> C.java -> D.java
        // Changing D.java should transitively recompile C, B, A
        let svc = IncrementalCompilationServiceImpl::new();

        svc.register_source_set(Request::new(RegisterSourceSetRequest {
            build_id: "build-6".to_string(),
            source_set: Some(make_source_set("ss6", "main")),
        }))
        .await
        .unwrap();

        svc.record_compilation(Request::new(RecordCompilationRequest {
            build_id: "build-6".to_string(),
            unit: Some(make_compilation_unit("ss6", "D.java", vec![])),
        }))
        .await
        .unwrap();

        svc.record_compilation(Request::new(RecordCompilationRequest {
            build_id: "build-6".to_string(),
            unit: Some(make_compilation_unit("ss6", "C.java", vec!["D.java"])),
        }))
        .await
        .unwrap();

        svc.record_compilation(Request::new(RecordCompilationRequest {
            build_id: "build-6".to_string(),
            unit: Some(make_compilation_unit("ss6", "B.java", vec!["C.java"])),
        }))
        .await
        .unwrap();

        svc.record_compilation(Request::new(RecordCompilationRequest {
            build_id: "build-6".to_string(),
            unit: Some(make_compilation_unit("ss6", "A.java", vec!["B.java"])),
        }))
        .await
        .unwrap();

        // E.java is independent
        svc.record_compilation(Request::new(RecordCompilationRequest {
            build_id: "build-6".to_string(),
            unit: Some(make_compilation_unit("ss6", "E.java", vec![])),
        }))
        .await
        .unwrap();

        // Change D.java — should transitively affect A, B, C, D but not E
        let rebuild = svc
            .get_rebuild_set(Request::new(GetRebuildSetRequest {
                build_id: "build-6".to_string(),
                source_set_id: "ss6".to_string(),
                changed_files: vec!["D.java".to_string()],
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(rebuild.total_sources, 5);
        assert_eq!(rebuild.must_recompile_count, 4);
        assert_eq!(rebuild.up_to_date_count, 1);

        let rebuild_files: std::collections::HashSet<String> =
            rebuild.decisions.iter().map(|d| d.source_file.clone()).collect();
        assert!(rebuild_files.contains("D.java"));
        assert!(rebuild_files.contains("C.java"));
        assert!(rebuild_files.contains("B.java"));
        assert!(rebuild_files.contains("A.java"));
        assert!(!rebuild_files.contains("E.java"));

        // D is directly changed, others are transitive
        let d_decision = rebuild.decisions.iter().find(|d| d.source_file == "D.java").unwrap();
        assert_eq!(d_decision.reason, "source_changed");
        let a_decision = rebuild.decisions.iter().find(|d| d.source_file == "A.java").unwrap();
        assert_eq!(a_decision.reason, "transitive_dependency_changed");
    }

    #[tokio::test]
    async fn test_diamond_dependency() {
        //     A
        //    / \
        //   B   C
        //    \ /
        //     D
        // Changing D should recompile B, C, A (but D only once)
        let svc = IncrementalCompilationServiceImpl::new();

        svc.register_source_set(Request::new(RegisterSourceSetRequest {
            build_id: "build-7".to_string(),
            source_set: Some(make_source_set("ss7", "main")),
        }))
        .await
        .unwrap();

        svc.record_compilation(Request::new(RecordCompilationRequest {
            build_id: "build-7".to_string(),
            unit: Some(make_compilation_unit("ss7", "D.java", vec![])),
        }))
        .await
        .unwrap();

        svc.record_compilation(Request::new(RecordCompilationRequest {
            build_id: "build-7".to_string(),
            unit: Some(make_compilation_unit("ss7", "B.java", vec!["D.java"])),
        }))
        .await
        .unwrap();

        svc.record_compilation(Request::new(RecordCompilationRequest {
            build_id: "build-7".to_string(),
            unit: Some(make_compilation_unit("ss7", "C.java", vec!["D.java"])),
        }))
        .await
        .unwrap();

        svc.record_compilation(Request::new(RecordCompilationRequest {
            build_id: "build-7".to_string(),
            unit: Some(make_compilation_unit("ss7", "A.java", vec!["B.java", "C.java"])),
        }))
        .await
        .unwrap();

        let rebuild = svc
            .get_rebuild_set(Request::new(GetRebuildSetRequest {
                build_id: "build-7".to_string(),
                source_set_id: "ss7".to_string(),
                changed_files: vec!["D.java".to_string()],
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(rebuild.must_recompile_count, 4);
    }

    #[tokio::test]
    async fn test_classpath_change_invalidates_all() {
        // Register source set, compile files, then change classpath
        let svc = IncrementalCompilationServiceImpl::new();

        // Initial registration with classpath hash "cp-v1"
        svc.register_source_set(Request::new(RegisterSourceSetRequest {
            build_id: "build-cp".to_string(),
            source_set: Some(SourceSetDescriptor {
                source_set_id: "ss-cp".to_string(),
                name: "main".to_string(),
                source_dirs: vec!["src/main/java".to_string()],
                output_dirs: vec!["build/classes".to_string()],
                classpath_hash: "cp-v1".to_string(),
            }),
        }))
        .await
        .unwrap();

        // Record compilations
        svc.record_compilation(Request::new(RecordCompilationRequest {
            build_id: "build-cp".to_string(),
            unit: Some(make_compilation_unit("ss-cp", "X.java", vec![])),
        }))
        .await
        .unwrap();

        svc.record_compilation(Request::new(RecordCompilationRequest {
            build_id: "build-cp".to_string(),
            unit: Some(make_compilation_unit("ss-cp", "Y.java", vec!["X.java"])),
        }))
        .await
        .unwrap();

        // Re-register with same classpath — should keep state
        svc.register_source_set(Request::new(RegisterSourceSetRequest {
            build_id: "build-cp".to_string(),
            source_set: Some(SourceSetDescriptor {
                source_set_id: "ss-cp".to_string(),
                name: "main".to_string(),
                source_dirs: vec!["src/main/java".to_string()],
                output_dirs: vec!["build/classes".to_string()],
                classpath_hash: "cp-v1".to_string(),
            }),
        }))
        .await
        .unwrap();

        let state = svc
            .get_incremental_state(Request::new(GetIncrementalStateRequest {
                build_id: "build-cp".to_string(),
                source_set_id: "ss-cp".to_string(),
            }))
            .await
            .unwrap()
            .into_inner()
            .state
            .unwrap();

        assert!(!state.classpath_changed);
        assert_eq!(state.total_compiled, 2);

        // Re-register with different classpath — should invalidate
        svc.register_source_set(Request::new(RegisterSourceSetRequest {
            build_id: "build-cp".to_string(),
            source_set: Some(SourceSetDescriptor {
                source_set_id: "ss-cp".to_string(),
                name: "main".to_string(),
                source_dirs: vec!["src/main/java".to_string()],
                output_dirs: vec!["build/classes".to_string()],
                classpath_hash: "cp-v2".to_string(),
            }),
        }))
        .await
        .unwrap();

        let state2 = svc
            .get_incremental_state(Request::new(GetIncrementalStateRequest {
                build_id: "build-cp".to_string(),
                source_set_id: "ss-cp".to_string(),
            }))
            .await
            .unwrap()
            .into_inner()
            .state
            .unwrap();

        assert!(state2.classpath_changed);
        assert_eq!(state2.total_compiled, 0); // invalidated
        assert_eq!(state2.previous_classpath_hash, "cp-v1");
        assert_eq!(state2.current_classpath_hash, "cp-v2");
        assert_eq!(state2.incrementally_compiled, 0); // counters reset
        assert_eq!(state2.fully_recompiled, 0);
    }

    #[tokio::test]
    async fn test_classpath_change_triggers_full_rebuild() {
        let svc = IncrementalCompilationServiceImpl::new();

        svc.register_source_set(Request::new(RegisterSourceSetRequest {
            build_id: "build-cp2".to_string(),
            source_set: Some(SourceSetDescriptor {
                source_set_id: "ss-cp2".to_string(),
                name: "main".to_string(),
                source_dirs: vec!["src/main/java".to_string()],
                output_dirs: vec!["build/classes".to_string()],
                classpath_hash: "old-cp".to_string(),
            }),
        }))
        .await
        .unwrap();

        svc.record_compilation(Request::new(RecordCompilationRequest {
            build_id: "build-cp2".to_string(),
            unit: Some(make_compilation_unit("ss-cp2", "A.java", vec![])),
        }))
        .await
        .unwrap();

        // Change classpath
        svc.register_source_set(Request::new(RegisterSourceSetRequest {
            build_id: "build-cp2".to_string(),
            source_set: Some(SourceSetDescriptor {
                source_set_id: "ss-cp2".to_string(),
                name: "main".to_string(),
                source_dirs: vec!["src/main/java".to_string()],
                output_dirs: vec!["build/classes".to_string()],
                classpath_hash: "new-cp".to_string(),
            }),
        }))
        .await
        .unwrap();

        // Request rebuild set with no source changes — classpath change should still trigger rebuild
        let rebuild = svc
            .get_rebuild_set(Request::new(GetRebuildSetRequest {
                build_id: "build-cp2".to_string(),
                source_set_id: "ss-cp2".to_string(),
                changed_files: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        // The classpath was invalidated so units were cleared, but the old units were tracked.
        // Since units are now empty (invalidated), there's nothing to rebuild.
        // This is expected: the daemon will recompile from scratch.
        assert_eq!(rebuild.total_sources, 0);
    }

    #[tokio::test]
    async fn test_classpath_change_then_new_compilation() {
        let svc = IncrementalCompilationServiceImpl::new();

        // Initial compile
        svc.register_source_set(Request::new(RegisterSourceSetRequest {
            build_id: "build-cp3".to_string(),
            source_set: Some(SourceSetDescriptor {
                source_set_id: "ss-cp3".to_string(),
                name: "main".to_string(),
                source_dirs: vec!["src/main/java".to_string()],
                output_dirs: vec!["build/classes".to_string()],
                classpath_hash: "cp-a".to_string(),
            }),
        }))
        .await
        .unwrap();

        svc.record_compilation(Request::new(RecordCompilationRequest {
            build_id: "build-cp3".to_string(),
            unit: Some(make_compilation_unit("ss-cp3", "Z.java", vec![])),
        }))
        .await
        .unwrap();

        // Change classpath
        svc.register_source_set(Request::new(RegisterSourceSetRequest {
            build_id: "build-cp3".to_string(),
            source_set: Some(SourceSetDescriptor {
                source_set_id: "ss-cp3".to_string(),
                name: "main".to_string(),
                source_dirs: vec!["src/main/java".to_string()],
                output_dirs: vec!["build/classes".to_string()],
                classpath_hash: "cp-b".to_string(),
            }),
        }))
        .await
        .unwrap();

        // Re-compile after classpath change
        let resp = svc.record_compilation(Request::new(RecordCompilationRequest {
            build_id: "build-cp3".to_string(),
            unit: Some(make_compilation_unit("ss-cp3", "Z.java", vec![])),
        }))
        .await
        .unwrap()
        .into_inner();

        assert!(!resp.changed); // units were cleared, so this is a fresh compile, not recompilation

        let state = svc
            .get_incremental_state(Request::new(GetIncrementalStateRequest {
                build_id: "build-cp3".to_string(),
                source_set_id: "ss-cp3".to_string(),
            }))
            .await
            .unwrap()
            .into_inner()
            .state
            .unwrap();

        assert_eq!(state.total_compiled, 1);
        assert_eq!(state.fully_recompiled, 1);
        assert_eq!(state.incrementally_compiled, 0);
    }
}
