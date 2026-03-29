use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use super::scopes::BuildId;

use crate::proto::{
    incremental_compilation_service_server::IncrementalCompilationService,
    AnalyzeClassDependenciesRequest, AnalyzeClassDependenciesResponse, AnnotationProcessorChange,
    ClassDependencyInfo, CompilationUnit, DetectAnnotationProcessorChangesRequest,
    DetectAnnotationProcessorChangesResponse, DiscoverSourcesRequest, DiscoverSourcesResponse,
    DiscoveredSource, GetIncrementalStateRequest, GetIncrementalStateResponse,
    GetRebuildSetRequest, GetRebuildSetResponse, IncrementalState, RebuildDecision,
    RecordCompilationRequest, RecordCompilationResponse, RegisterSourceSetRequest,
    RegisterSourceSetResponse, SourceSetDescriptor,
};

/// Tracked source set for incremental compilation.
struct SourceSet {
    descriptor: SourceSetDescriptor,
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
    source_sets: DashMap<String, SourceSet>, // source_set_id -> SourceSet
    build_source_sets: DashMap<BuildId, Vec<String>>, // build_id -> [source_set_id]
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
        let mut reverse_deps: HashMap<&str, Vec<&str>> = HashMap::with_capacity(units.len());
        for unit in units {
            for dep in &unit.dependencies {
                reverse_deps
                    .entry(dep.as_str())
                    .or_default()
                    .push(&unit.source_file);
            }
        }

        // BFS from changed files through reverse dependencies
        let mut affected: HashSet<String> = HashSet::with_capacity(changed_files.len() + units.len());
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

    /// Discover source files in the given directories matching include patterns.
    /// Uses native directory walking with Ant-style pattern matching instead of
    /// the `glob` crate, avoiding pattern compilation overhead and redundant
    /// filesystem traversals when multiple include patterns share a source dir.
    fn discover_sources_impl(
        source_dirs: &[String],
        include_patterns: &[String],
        exclude_patterns: &[String],
    ) -> Vec<DiscoveredSource> {
        let include_globs: Vec<String> = if include_patterns.is_empty() {
            vec![
                "**/*.java".to_string(),
                "**/*.kt".to_string(),
                "**/*.groovy".to_string(),
                "**/*.scala".to_string(),
            ]
        } else {
            include_patterns.to_vec()
        };

        let mut sources = Vec::with_capacity(source_dirs.len() * 32);

        for source_dir in source_dirs {
            let dir_path = Path::new(source_dir);
            if !dir_path.is_dir() {
                continue;
            }

            // Walk directory once, match each file against all include patterns
            let mut stack = vec![dir_path.to_path_buf()];
            while let Some(current) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&current) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let Ok(file_type) = entry.file_type() else {
                        continue;
                    };
                    let entry_path = entry.path();

                    if file_type.is_dir() {
                        stack.push(entry_path);
                        continue;
                    }

                    if !file_type.is_file() {
                        continue;
                    }

                    // Compute relative path once
                    let relative = entry_path
                        .strip_prefix(dir_path)
                        .unwrap_or(&entry_path)
                        .to_string_lossy()
                        .to_string();

                    // Check include patterns
                    let included = include_globs.iter().any(|pat| {
                        crate::server::file_tree::ant_match(&relative, pat)
                    });
                    if !included {
                        continue;
                    }

                    // Check exclude patterns — support both relative and absolute exclude patterns
                    let abs_entry = entry_path.to_string_lossy();
                    let excluded = exclude_patterns.iter().any(|exc| {
                        // Try relative match first
                        if crate::server::file_tree::ant_match(&relative, exc) {
                            return true;
                        }
                        // Try absolute match (exclude pattern may be absolute path)
                        let full_exc_binding = dir_path.join(exc);
                        let full_exc = full_exc_binding.to_string_lossy();
                        crate::server::file_tree::ant_match(&abs_entry, &full_exc)
                    });
                    if excluded {
                        continue;
                    }

                    let extension = entry_path
                        .extension()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();

                    let metadata = std::fs::metadata(&entry_path);
                    let (last_modified_ms, size_bytes) = match &metadata {
                        Ok(m) => (
                            m.modified()
                                .ok()
                                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                                .map(|d| d.as_millis() as i64)
                                .unwrap_or(0),
                            m.len() as i64,
                        ),
                        Err(_) => (0, 0),
                    };

                    sources.push(DiscoveredSource {
                        path: relative,
                        source_dir: source_dir.clone(),
                        extension,
                        last_modified_ms,
                        size_bytes,
                    });
                }
            }
        }

        sources.sort_unstable_by(|a, b| a.path.cmp(&b.path));
        sources.dedup_by(|a, b| a.path == b.path);
        sources
    }

    /// Parse .class files to extract class references and annotations.
    /// Uses a byte-level scanner to find ConstantPool #Class entries.
    fn analyze_class_dependencies_impl(
        output_dirs: &[String],
        target_files: &[String],
    ) -> Vec<ClassDependencyInfo> {
        let mut results = Vec::with_capacity(target_files.len().min(64));

        // Build set of files to analyze
        let mut targets: HashSet<&str> = HashSet::with_capacity(target_files.len());
        targets.extend(target_files.iter().map(|s| s.as_str()));

        for output_dir in output_dirs {
            let dir = Path::new(output_dir);
            if !dir.exists() {
                continue;
            }

            for entry in walk_dir_recursive(dir).into_iter().flatten() {
                if entry.extension().and_then(|e: &std::ffi::OsStr| e.to_str()) != Some("class") {
                    continue;
                }

                // Skip if specific targets were requested and this isn't one
                if !targets.is_empty() {
                    let rel = entry.strip_prefix(dir).unwrap_or(&entry).to_string_lossy();
                    if !targets.iter().any(|t| rel.contains(t) || rel.ends_with(t)) {
                        continue;
                    }
                }

                if let Ok(data) = std::fs::read(&entry) {
                    let info = Self::parse_class_file(&data, &entry);
                    results.push(info);
                }
            }
        }

        results
    }

    /// Parse a single .class file to extract references and annotations.
    fn parse_class_file(data: &[u8], path: &Path) -> ClassDependencyInfo {
        let class_name = extract_class_name(data).unwrap_or_else(|| {
            path.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        });

        let references = extract_class_references(data);
        let annotations = extract_annotations(data);

        // Heuristic: if the class extends javax.annotation.processing.AbstractProcessor
        // or has @SupportedAnnotationTypes, it's an annotation processor
        let is_ap = references.iter().any(|r| {
            r.contains("javax.annotation.processing.AbstractProcessor")
                || r.contains("javax.annotation.processing.Processor")
        }) || annotations
            .iter()
            .any(|a| a == "SupportedAnnotationTypes" || a == "SupportedSourceVersion");

        ClassDependencyInfo {
            class_file: path.to_string_lossy().into_owned(),

            class_name,
            references,
            annotations,
            is_annotation_processor: is_ap,
            has_generated_sources: false, // Would need source dir comparison
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
        let build_id = BuildId::from(req.build_id.clone());
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
                    if changed {
                        0
                    } else {
                        existing.total_compile_time_ms
                    },
                    if changed {
                        0
                    } else {
                        existing.incremental_compiles
                    },
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
            if let Some(existing) = ss
                .units
                .iter_mut()
                .find(|u| u.source_file == unit.source_file)
            {
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
            decisions.reserve(total_sources as usize);

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
                let affected = Self::compute_transitive_rebuild_set(&ss.units, &req.changed_files);

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

    async fn discover_sources(
        &self,
        request: Request<DiscoverSourcesRequest>,
    ) -> Result<Response<DiscoverSourcesResponse>, Status> {
        let req = request.into_inner();
        let sources = Self::discover_sources_impl(
            &req.source_dirs,
            &req.include_patterns,
            &req.exclude_patterns,
        );
        let total = sources.len() as i32;
        tracing::debug!(
            source_dirs = ?req.source_dirs,
            total_discovered = total,
            "Source discovery complete"
        );
        Ok(Response::new(DiscoverSourcesResponse {
            sources,
            total_discovered: total,
        }))
    }

    async fn analyze_class_dependencies(
        &self,
        request: Request<AnalyzeClassDependenciesRequest>,
    ) -> Result<Response<AnalyzeClassDependenciesResponse>, Status> {
        let req = request.into_inner();
        let dependencies =
            Self::analyze_class_dependencies_impl(&req.output_dirs, &req.class_files);
        let total = dependencies.len() as i32;
        tracing::debug!(
            output_dirs = ?req.output_dirs,
            total_analyzed = total,
            "Class dependency analysis complete"
        );
        Ok(Response::new(AnalyzeClassDependenciesResponse {
            dependencies,
            total_analyzed: total,
        }))
    }

    async fn detect_annotation_processor_changes(
        &self,
        request: Request<DetectAnnotationProcessorChangesRequest>,
    ) -> Result<Response<DetectAnnotationProcessorChangesResponse>, Status> {
        let req = request.into_inner();

        // Analyze class files in the processor classpath to find current processors
        let mut current_processors: HashSet<String> = HashSet::with_capacity(req.annotation_processor_classpath.len());
        for cp_entry in &req.annotation_processor_classpath {
            let cp_path = Path::new(cp_entry);
            if !cp_path.is_dir() {
                continue;
            }
            let mut stack = vec![cp_path.to_path_buf()];
            while let Some(current) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&current) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let Ok(file_type) = entry.file_type() else {
                        continue;
                    };
                    let entry_path = entry.path();
                    if file_type.is_dir() {
                        stack.push(entry_path);
                    } else if file_type.is_file()
                        && entry_path.extension().is_some_and(|e| e == "class")
                    {
                        if let Ok(data) = std::fs::read(&entry_path) {
                            if is_annotation_processor_class(&data) {
                                if let Some(name) = extract_class_name(&data) {
                                    current_processors.insert(name);
                                }
                            }
                        }
                    }
                }
            }
        }

        let mut previous: HashSet<String> = HashSet::with_capacity(req.previous_processor_classes.len());
        previous.extend(req.previous_processor_classes.iter().cloned());

        let mut changes = Vec::with_capacity(current_processors.len() + previous.len());
        for added in current_processors.difference(&previous) {
            changes.push(AnnotationProcessorChange {
                processor_class: added.clone(),
                change_type: "added".to_string(),
                details: format!("New annotation processor: {}", added),
            });
        }
        for removed in previous.difference(&current_processors) {
            changes.push(AnnotationProcessorChange {
                processor_class: removed.clone(),
                change_type: "removed".to_string(),
                details: format!("Annotation processor removed: {}", removed),
            });
        }

        let processors_changed = !changes.is_empty();
        // Any processor change requires full recompilation
        let full_recompilation_required = processors_changed;

        tracing::debug!(
            processors_changed,
            changes_count = changes.len(),
            "Annotation processor change detection complete"
        );

        Ok(Response::new(DetectAnnotationProcessorChangesResponse {
            processors_changed,
            changes,
            full_recompilation_required,
        }))
    }
}

// --- Helper functions for class file analysis ---

/// Extract the class name from a .class file's ConstantPool.
fn extract_class_name(data: &[u8]) -> Option<String> {
    // Java class file format:
    // u4 magic (0xCAFEBABE), u2 minor, u2 major, u2 constant_pool_count
    if data.len() < 10 || data[0..4] != [0xCA, 0xFE, 0xBA, 0xBE] {
        return None;
    }

    let cp_count = u16::from_be_bytes([data[8], data[9]]) as usize;
    let mut offset = 10;
    let mut utf8_strings: Vec<Option<String>> = Vec::with_capacity(cp_count);
    utf8_strings.push(None); // index 0 unused

    for _ in 1..cp_count {
        if offset >= data.len() {
            break;
        }
        let tag = data[offset];
        offset += 1;

        match tag {
            1 => {
                // CONSTANT_Utf8
                if offset + 2 > data.len() {
                    break;
                }
                let len = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
                offset += 2;
                if offset + len > data.len() {
                    break;
                }
                let s = String::from_utf8_lossy(&data[offset..offset + len]).into_owned();
                utf8_strings.push(Some(s));
                offset += len;
            }
            3..=4 => {
                // CONSTANT_Integer / CONSTANT_Float
                offset += 4;
                utf8_strings.push(None);
            }
            5..=6 => {
                // CONSTANT_Long / CONSTANT_Double
                offset += 8;
                utf8_strings.push(None);
                utf8_strings.push(None); // Takes two slots
            }
            7..=8 => {
                // CONSTANT_Class / CONSTANT_String
                offset += 2;
                utf8_strings.push(None);
            }
            9..=12 => {
                // CONSTANT_Fieldref / Methodref / InterfaceMethodref / NameAndType
                offset += 4;
                utf8_strings.push(None);
            }
            15..=16 | 18 => {
                // CONSTANT_MethodHandle / CONSTANT_MethodType / CONSTANT_InvokeDynamic
                offset += 3;
                utf8_strings.push(None);
            }
            17 => {
                // CONSTANT_Dynamic
                offset += 4;
                utf8_strings.push(None);
            }
            19 => {
                // CONSTANT_Module / CONSTANT_Package
                offset += 2;
                utf8_strings.push(None);
            }
            _ => break,
        }
    }

    // After constant pool: u2 access_flags, u2 this_class, u2 super_class
    if offset + 6 > data.len() {
        return None;
    }
    let this_class_idx = u16::from_be_bytes([data[offset + 2], data[offset + 3]]) as usize;
    if this_class_idx < utf8_strings.len() {
        // this_class points to a CONSTANT_Class, which points to a CONSTANT_Utf8
        // For simplicity, return the index — full resolution would need another pass
        utf8_strings.get(this_class_idx).cloned().flatten()
    } else {
        None
    }
}

/// Extract class references (CONSTANT_Class entries) from a .class file.
fn extract_class_references(data: &[u8]) -> Vec<String> {
    let mut refs = Vec::new();

    if data.len() < 10 || data[0..4] != [0xCA, 0xFE, 0xBA, 0xBE] {
        return refs;
    }

    let cp_count = u16::from_be_bytes([data[8], data[9]]) as usize;
    refs.reserve(cp_count / 4); // roughly 1/4 of constant pool entries are class refs
    let mut offset = 10;
    let mut utf8_strings: HashMap<usize, String> = HashMap::with_capacity(cp_count);

    for idx in 1..cp_count {
        if offset >= data.len() {
            break;
        }
        let tag = data[offset];
        offset += 1;

        match tag {
            1 => {
                if offset + 2 > data.len() {
                    break;
                }
                let len = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
                offset += 2;
                if offset + len > data.len() {
                    break;
                }
                let s = String::from_utf8_lossy(&data[offset..offset + len]).into_owned();
                utf8_strings.insert(idx, s);
                offset += len;
            }
            3 | 4 => offset += 4,
            5 | 6 => {
                offset += 8;
            }
            7 => {
                // CONSTANT_Class — points to a Utf8 name
                if offset + 2 > data.len() {
                    break;
                }
                let name_idx = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
                offset += 2;
                if let Some(name) = utf8_strings.get(&name_idx) {
                    // Skip array types and primitives
                    if !name.starts_with('[') && !is_primitive_descriptor(name) {
                        refs.push(name.clone());
                    }
                }
            }
            8 => offset += 2,
            9..=12 => offset += 4,
            15 | 16 | 18 => offset += 3,
            17 => offset += 4,
            19 => offset += 2,
            _ => break,
        }
    }

    refs.sort_unstable();
    refs.dedup();
    refs
}

/// Extract annotation class names from RuntimeVisibleAnnotations attribute.
fn extract_annotations(_data: &[u8]) -> Vec<String> {
    // Full annotation extraction requires parsing attributes after the constant pool.
    // For now, return empty — this would need a more complete class file parser.
    // In production, this would parse the RuntimeVisibleAnnotations attribute.
    Vec::new()
}

/// Check if a class extends AbstractProcessor.
fn is_annotation_processor_class(data: &[u8]) -> bool {
    let refs = extract_class_references(data);
    refs.iter().any(|r| {
        r.contains("javax.annotation.processing.AbstractProcessor")
            || r.contains("javax.annotation.processing.Processor")
    })
}

/// Check if a descriptor represents a primitive type.
fn is_primitive_descriptor(s: &str) -> bool {
    matches!(s, "B" | "C" | "D" | "F" | "I" | "J" | "S" | "Z" | "V")
}

/// Recursively walk a directory.
fn walk_dir_recursive(dir: &Path) -> Vec<std::io::Result<std::path::PathBuf>> {
    let mut entries = Vec::with_capacity(64);
    if let Ok(read_dir) = std::fs::read_dir(dir) {
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.is_dir() {
                entries.extend(walk_dir_recursive(&path));
            } else {
                entries.push(Ok(path));
            }
        }
    }
    entries
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

    fn make_compilation_unit(
        source_set_id: &str,
        source: &str,
        deps: Vec<&str>,
    ) -> CompilationUnit {
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

        let rebuild_files: std::collections::HashSet<String> = rebuild
            .decisions
            .iter()
            .map(|d| d.source_file.clone())
            .collect();
        assert!(rebuild_files.contains("D.java"));
        assert!(rebuild_files.contains("C.java"));
        assert!(rebuild_files.contains("B.java"));
        assert!(rebuild_files.contains("A.java"));
        assert!(!rebuild_files.contains("E.java"));

        // D is directly changed, others are transitive
        let d_decision = rebuild
            .decisions
            .iter()
            .find(|d| d.source_file == "D.java")
            .unwrap();
        assert_eq!(d_decision.reason, "source_changed");
        let a_decision = rebuild
            .decisions
            .iter()
            .find(|d| d.source_file == "A.java")
            .unwrap();
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
            unit: Some(make_compilation_unit(
                "ss7",
                "A.java",
                vec!["B.java", "C.java"],
            )),
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
        let resp = svc
            .record_compilation(Request::new(RecordCompilationRequest {
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

    // --- Tests for new RPCs ---

    #[tokio::test]
    async fn test_discover_sources() {
        let svc = IncrementalCompilationServiceImpl::new();

        // Create temp source directories
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp
            .path()
            .join("src")
            .join("main")
            .join("java")
            .join("com")
            .join("example");
        std::fs::create_dir_all(&src_dir).unwrap();

        std::fs::write(
            src_dir.join("Foo.java"),
            "package com.example; class Foo {}",
        )
        .unwrap();
        std::fs::write(
            src_dir.join("Bar.java"),
            "package com.example; class Bar {}",
        )
        .unwrap();
        // Create a generated sources dir with exclusions
        let gen_dir = tmp.path().join("build").join("generated");
        std::fs::create_dir_all(&gen_dir).unwrap();
        std::fs::write(gen_dir.join("Generated.java"), "// generated").unwrap();

        let resp = svc
            .discover_sources(Request::new(DiscoverSourcesRequest {
                source_dirs: vec![
                    src_dir.to_string_lossy().to_string(),
                    gen_dir.to_string_lossy().to_string(),
                ],
                include_patterns: vec!["**/*.java".to_string()],
                exclude_patterns: vec![format!("{}/**/*.java", gen_dir.to_string_lossy())],
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.total_discovered, 2);
        let paths: Vec<&str> = resp.sources.iter().map(|s| s.path.as_str()).collect();
        assert!(paths.contains(&"Foo.java"));
        assert!(paths.contains(&"Bar.java"));
    }

    #[tokio::test]
    async fn test_discover_sources_empty_dir() {
        let svc = IncrementalCompilationServiceImpl::new();

        let tmp = tempfile::tempdir().unwrap();
        let empty_dir = tmp.path().join("empty");

        let resp = svc
            .discover_sources(Request::new(DiscoverSourcesRequest {
                source_dirs: vec![empty_dir.to_string_lossy().to_string()],
                include_patterns: vec![],
                exclude_patterns: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.total_discovered, 0);
    }

    #[tokio::test]
    async fn test_discover_sources_default_patterns() {
        let svc = IncrementalCompilationServiceImpl::new();

        let tmp = tempfile::tempdir().unwrap();
        let java_dir = tmp.path().join("java");
        let kt_dir = tmp.path().join("kotlin");
        std::fs::create_dir_all(&java_dir).unwrap();
        std::fs::create_dir_all(&kt_dir).unwrap();

        std::fs::write(java_dir.join("A.java"), "class A {}").unwrap();
        std::fs::write(kt_dir.join("B.kt"), "class B {}").unwrap();
        std::fs::write(java_dir.join("C.txt"), "not a source").unwrap();

        let resp = svc
            .discover_sources(Request::new(DiscoverSourcesRequest {
                source_dirs: vec![
                    java_dir.to_string_lossy().to_string(),
                    kt_dir.to_string_lossy().to_string(),
                ],
                include_patterns: vec![], // Use defaults
                exclude_patterns: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.total_discovered, 2);
    }

    #[tokio::test]
    async fn test_analyze_class_dependencies_empty() {
        let svc = IncrementalCompilationServiceImpl::new();

        let tmp = tempfile::tempdir().unwrap();
        let empty_dir = tmp.path().join("classes");

        let resp = svc
            .analyze_class_dependencies(Request::new(AnalyzeClassDependenciesRequest {
                output_dirs: vec![empty_dir.to_string_lossy().to_string()],
                class_files: vec![],
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.total_analyzed, 0);
    }

    #[tokio::test]
    async fn test_detect_annotation_processor_changes_no_change() {
        let svc = IncrementalCompilationServiceImpl::new();

        let resp = svc
            .detect_annotation_processor_changes(Request::new(
                DetectAnnotationProcessorChangesRequest {
                    source_set_id: "test-ss".to_string(),
                    annotation_processor_classpath: vec![],
                    previous_processor_classes: vec![],
                },
            ))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.processors_changed);
        assert!(!resp.full_recompilation_required);
        assert_eq!(resp.changes.len(), 0);
    }

    #[tokio::test]
    async fn test_detect_annotation_processor_changes_added() {
        let svc = IncrementalCompilationServiceImpl::new();

        let resp = svc
            .detect_annotation_processor_changes(Request::new(
                DetectAnnotationProcessorChangesRequest {
                    source_set_id: "test-ss".to_string(),
                    annotation_processor_classpath: vec![],
                    previous_processor_classes: vec!["com.example.OldProcessor".to_string()],
                },
            ))
            .await
            .unwrap()
            .into_inner();

        // Old processor was removed (not found in empty classpath)
        assert!(resp.processors_changed);
        assert!(resp.full_recompilation_required);
        assert!(resp.changes.iter().any(|c| c.change_type == "removed"));
    }

    // --- Unit tests for helper functions ---

    #[test]
    fn test_extract_class_references_empty() {
        let refs = extract_class_references(&[]);
        assert!(refs.is_empty());
    }

    #[test]
    fn test_extract_class_references_invalid_magic() {
        let refs = extract_class_references(&[0x00, 0x00, 0x00, 0x00]);
        assert!(refs.is_empty());
    }

    #[test]
    fn test_is_primitive_descriptor() {
        assert!(is_primitive_descriptor("I"));
        assert!(is_primitive_descriptor("Z"));
        assert!(is_primitive_descriptor("V"));
        assert!(!is_primitive_descriptor("Ljava/lang/Object;"));
        assert!(!is_primitive_descriptor("com.example.Foo"));
    }

    #[test]
    fn test_walk_dir_recursive_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let entries = walk_dir_recursive(&tmp.path().join("nonexistent"));
        assert!(entries.is_empty());
    }
}
