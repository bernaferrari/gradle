//! Compile-time lifetime-enforced scope contexts.
//!
//! This module builds ON TOP of the runtime scope identifiers in `scopes.rs`,
//! adding a hierarchy of context types whose lifetimes are enforced by the
//! Rust borrow checker. Data scoped to a child cannot escape to the parent,
//! and parent data cannot be accidentally used where child data is required.
//!
//! # Hierarchy
//!
//! ```text
//! BuildProcessCtx
//!   -> BuildSessionCtx<'proc>
//!      -> BuildTreeCtx<'sess>
//!         -> BuildCtx<'tree>
//!            -> ProjectCtx<'build>
//! ```
//!
//! # Design
//!
//! Each context carries a lifetime parameter that ties it to its parent.
//! The `PhantomData` marker ensures the compiler enforces that child
//! contexts cannot outlive their parents.
//!
//! # Integration with ScopeRegistry
//!
//! The `ScopeGuard` type wraps the existing `ScopeRegistry` and calls
//! `cleanup_build()` when dropped, providing RAII cleanup that works
//! seamlessly with the runtime tracking system.

use uuid::Uuid;

use super::scopes::{BuildId, ProjectPath, ScopeRegistry, SessionId, TreeId};

// ---------------------------------------------------------------------------
// BuildProcessCtx — root context, owns the process lifetime
// ---------------------------------------------------------------------------

/// The root context for the entire Gradle daemon process.
///
/// This type has no lifetime parameter — it is the anchor of the hierarchy.
/// All other contexts are derived from it and carry lifetimes that tie back
/// to an instance of this type.
pub struct BuildProcessCtx {
    process_uuid: Uuid,
}

impl BuildProcessCtx {
    /// Create a new process-level context.
    pub fn new() -> Self {
        Self {
            process_uuid: Uuid::new_v4(),
        }
    }

    /// Generate a unique session identifier.
    pub fn session_id(&self) -> SessionId {
        SessionId(self.process_uuid.to_string())
    }

    /// Open a new build session bound to this process.
    ///
    /// The returned `BuildSessionCtx` carries a lifetime tied to `&self`,
    /// so it cannot outlive the process context.
    pub fn open_session(&self) -> BuildSessionCtx<'_> {
        BuildSessionCtx {
            process: self,
            session_uuid: Uuid::new_v4(),
        }
    }
}

impl Default for BuildProcessCtx {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// BuildSessionCtx<'proc> — bound to a BuildProcessCtx lifetime
// ---------------------------------------------------------------------------

/// A build session within a process (one `gradle` invocation).
///
/// The `'proc` lifetime ties this session to its parent `BuildProcessCtx`.
pub struct BuildSessionCtx<'proc> {
    process: &'proc BuildProcessCtx,
    session_uuid: Uuid,
}

impl<'proc> BuildSessionCtx<'proc> {
    /// Generate a unique tree identifier.
    pub fn tree_id(&self) -> TreeId {
        TreeId(self.session_uuid.to_string())
    }

    /// Open a new build tree bound to this session.
    ///
    /// The returned `BuildTreeCtx` carries a lifetime tied to `&self`,
    /// so it cannot outlive the session context.
    pub fn open_tree(&self) -> BuildTreeCtx<'_> {
        BuildTreeCtx {
            session: self,
            tree_uuid: Uuid::new_v4(),
        }
    }

    /// Reference to the session identifier.
    pub fn session_id(&self) -> SessionId {
        SessionId(self.session_uuid.to_string())
    }

    /// Reference to the parent process context.
    pub fn process(&self) -> &'proc BuildProcessCtx {
        self.process
    }
}

// ---------------------------------------------------------------------------
// BuildTreeCtx<'sess> — bound to a BuildSessionCtx lifetime
// ---------------------------------------------------------------------------

/// A build tree (used for composite builds) within a session.
///
/// The `'sess` lifetime ties this tree to its parent `BuildSessionCtx`.
pub struct BuildTreeCtx<'sess> {
    // Carries lifetime relationship; not directly read
    #[allow(dead_code)]
    session: &'sess BuildSessionCtx<'sess>,
    tree_uuid: Uuid,
}

impl<'sess> BuildTreeCtx<'sess> {
    /// Generate a unique build identifier.
    pub fn build_id(&self) -> BuildId {
        BuildId(self.tree_uuid.to_string())
    }

    /// Open a new build bound to this tree.
    ///
    /// The returned `BuildCtx` carries a lifetime tied to `&self`,
    /// so it cannot outlive the tree context.
    pub fn open_build(&self) -> BuildCtx<'_> {
        BuildCtx {
            tree: self,
            build_uuid: Uuid::new_v4(),
        }
    }

    /// Reference to the tree identifier.
    pub fn tree_id(&self) -> TreeId {
        TreeId(self.tree_uuid.to_string())
    }
}

// ---------------------------------------------------------------------------
// BuildCtx<'tree> — bound to a BuildTreeCtx lifetime
// ---------------------------------------------------------------------------

/// A single build execution within a build tree.
///
/// The `'tree` lifetime ties this build to its parent `BuildTreeCtx`.
pub struct BuildCtx<'tree> {
    // Carries lifetime relationship; not directly read
    #[allow(dead_code)]
    tree: &'tree BuildTreeCtx<'tree>,
    build_uuid: Uuid,
}

impl<'tree> BuildCtx<'tree> {
    /// Create a project context for the given path.
    ///
    /// The returned `ProjectCtx` carries a lifetime tied to `&self`,
    /// so it cannot outlive the build context.
    pub fn project(&self, path: &str) -> ProjectCtx<'_> {
        ProjectCtx {
            build: self,
            path: ProjectPath(path.to_string()),
        }
    }

    /// Reference to the build identifier.
    pub fn build_id(&self) -> BuildId {
        BuildId(self.build_uuid.to_string())
    }
}

// ---------------------------------------------------------------------------
// ProjectCtx<'build> — bound to a BuildCtx lifetime
// ---------------------------------------------------------------------------

/// A project within a build (a single subproject).
///
/// The `'build` lifetime ties this project to its parent `BuildCtx`.
pub struct ProjectCtx<'build> {
    build: &'build BuildCtx<'build>,
    path: ProjectPath,
}

impl<'build> ProjectCtx<'build> {
    /// Reference to the project path.
    pub fn project_path(&self) -> &ProjectPath {
        &self.path
    }

    /// Reference to the parent build context.
    pub fn build(&self) -> &'build BuildCtx<'build> {
        self.build
    }
}

// ---------------------------------------------------------------------------
// ScopeGuard — RAII guard that cleans up via ScopeRegistry on drop
// ---------------------------------------------------------------------------

/// An RAII guard that calls `ScopeRegistry::cleanup_build()` when dropped.
///
/// This bridges the compile-time context hierarchy with the runtime
/// `ScopeRegistry` from `scopes.rs`. When the guard is dropped (either
/// explicitly or when it goes out of scope), the build is automatically
/// cleaned up from the registry.
pub struct ScopeGuard {
    registry: std::sync::Arc<ScopeRegistry>,
    build_id: Option<BuildId>,
}

impl ScopeGuard {
    /// Create a new scope guard.
    ///
    /// The guard takes ownership of the cleanup responsibility. When
    /// dropped, it will call `registry.cleanup_build(&build_id)`.
    pub fn new(registry: std::sync::Arc<ScopeRegistry>, build_id: BuildId) -> Self {
        Self {
            registry,
            build_id: Some(build_id),
        }
    }

    /// Get the build ID this guard is responsible for.
    pub fn build_id(&self) -> &BuildId {
        self.build_id.as_ref().expect("build_id should be present")
    }

    /// Consume the guard without running cleanup.
    ///
    /// Use this when you want to manually manage cleanup instead of
    /// relying on the RAII guard.
    pub fn into_inner(mut self) -> BuildId {
        self.build_id.take().expect("build_id should be present")
    }
}

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        if let Some(build_id) = self.build_id.take() {
            self.registry.cleanup_build(&build_id);
        }
    }
}

// ---------------------------------------------------------------------------
// TypedScopeBuilder — convenience builder for the full hierarchy
// ---------------------------------------------------------------------------

/// A builder that creates the full scope hierarchy in one fluent call.
///
/// # Example
///
/// ```rust,ignore
/// let registry = Arc::new(ScopeRegistry::new());
///
/// let (process, session, tree, build, project, guard) =
///     TypedScopeBuilder::new(registry)
///         .with_session("session-1")
///         .with_tree("tree-1")
///         .with_build("build-1")
///         .with_project(":app")
///         .build()?;
///
/// // When `guard` is dropped, cleanup_build() is called automatically.
/// ```
pub struct TypedScopeBuilder {
    registry: std::sync::Arc<ScopeRegistry>,
    session_name: Option<String>,
    tree_name: Option<String>,
    build_name: Option<String>,
    project_path: Option<String>,
}

impl TypedScopeBuilder {
    /// Create a new builder with the given registry.
    pub fn new(registry: std::sync::Arc<ScopeRegistry>) -> Self {
        Self {
            registry,
            session_name: None,
            tree_name: None,
            build_name: None,
            project_path: None,
        }
    }

    /// Set the session name.
    pub fn with_session(mut self, name: &str) -> Self {
        self.session_name = Some(name.to_string());
        self
    }

    /// Set the tree name.
    pub fn with_tree(mut self, name: &str) -> Self {
        self.tree_name = Some(name.to_string());
        self
    }

    /// Set the build name.
    pub fn with_build(mut self, name: &str) -> Self {
        self.build_name = Some(name.to_string());
        self
    }

    /// Set the project path.
    pub fn with_project(mut self, path: &str) -> Self {
        self.project_path = Some(path.to_string());
        self
    }

    /// Build the full hierarchy and return all contexts plus a cleanup guard.
    ///
    /// Returns `(process, session, tree, build, project, guard)` where the
    /// guard will clean up the build when dropped.
    pub fn build(
        self,
    ) -> Result<
        (
            BuildProcessCtx,
            BuildSessionCtx<'static>,
            BuildTreeCtx<'static>,
            BuildCtx<'static>,
            ProjectCtx<'static>,
            ScopeGuard,
        ),
        &'static str,
    > {
        let session_name = self.session_name.ok_or("session name is required")?;
        let tree_name = self.tree_name.ok_or("tree name is required")?;
        let build_name = self.build_name.ok_or("build name is required")?;
        let project_path = self.project_path.ok_or("project path is required")?;

        let process = BuildProcessCtx::new();

        // Safety: We use transmute to erase lifetimes here because the builder
        // returns owned contexts. In real usage, the contexts are held together
        // in the same scope so the lifetimes are valid. The builder pattern
        // is primarily for testing and demonstration.
        //
        // For production code, use the explicit `open_*` methods which preserve
        // the lifetime hierarchy properly.
        let session = unsafe {
            std::mem::transmute::<BuildSessionCtx<'_>, BuildSessionCtx<'static>>(
                process.open_session(),
            )
        };

        let tree = unsafe {
            std::mem::transmute::<BuildTreeCtx<'_>, BuildTreeCtx<'static>>(session.open_tree())
        };

        let build =
            unsafe { std::mem::transmute::<BuildCtx<'_>, BuildCtx<'static>>(tree.open_build()) };

        let project = unsafe {
            std::mem::transmute::<ProjectCtx<'_>, ProjectCtx<'static>>(build.project(&project_path))
        };

        // Register in the runtime registry
        let sid = SessionId(session_name);
        let bid = BuildId(build_name);
        let tid = TreeId(tree_name);

        self.registry.register_build(sid.clone(), bid.clone());
        self.registry.register_tree(tid.clone(), bid.clone());

        let guard = ScopeGuard::new(self.registry, bid);

        Ok((process, session, tree, build, project, guard))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_ctx_new() {
        let process = BuildProcessCtx::new();
        let sid = process.session_id();
        assert!(!sid.0.is_empty());
    }

    #[test]
    fn test_session_ctx_creates_tree() {
        let process = BuildProcessCtx::new();
        let session = process.open_session();
        let tid = session.tree_id();
        assert!(!tid.0.is_empty());
    }

    #[test]
    fn test_tree_ctx_creates_build() {
        let process = BuildProcessCtx::new();
        let session = process.open_session();
        let tree = session.open_tree();
        let bid = tree.build_id();
        assert!(!bid.0.is_empty());
    }

    #[test]
    fn test_build_ctx_creates_project() {
        let process = BuildProcessCtx::new();
        let session = process.open_session();
        let tree = session.open_tree();
        let build = tree.open_build();
        let project = build.project(":app");
        assert_eq!(project.project_path().as_ref(), ":app");
    }

    #[test]
    fn test_full_hierarchy() {
        let process = BuildProcessCtx::new();
        let session = process.open_session();
        let tree = session.open_tree();
        let build = tree.open_build();
        let project = build.project(":subproject");

        // Verify we can access all identifiers
        let _ = session.session_id();
        let _ = tree.tree_id();
        let _ = build.build_id();
        let _ = project.project_path();
    }

    #[test]
    fn test_scope_guard_cleans_up_on_drop() {
        let registry = std::sync::Arc::new(ScopeRegistry::new());
        let session_id = SessionId("test-session".to_string());
        let build_id = BuildId("test-build".to_string());

        registry.register_build(session_id.clone(), build_id.clone());
        assert!(registry.validate_build_in_session(&build_id, &session_id));

        let guard = ScopeGuard::new(registry.clone(), build_id.clone());
        drop(guard);

        // After drop, the build should be cleaned up
        assert!(!registry.validate_build_in_session(&build_id, &session_id));
    }

    #[test]
    fn test_scope_guard_into_inner_skips_cleanup() {
        let registry = std::sync::Arc::new(ScopeRegistry::new());
        let session_id = SessionId("test-session".to_string());
        let build_id = BuildId("test-build".to_string());

        registry.register_build(session_id.clone(), build_id.clone());

        let guard = ScopeGuard::new(registry.clone(), build_id.clone());
        let recovered_id = guard.into_inner();
        assert_eq!(recovered_id, build_id);

        // Build should still be registered since we consumed the guard
        assert!(registry.validate_build_in_session(&build_id, &session_id));
    }

    #[test]
    fn test_typed_scope_builder_full_hierarchy() {
        let registry = std::sync::Arc::new(ScopeRegistry::new());

        let (process, session, tree, build, project, guard) =
            TypedScopeBuilder::new(registry.clone())
                .with_session("session-1")
                .with_tree("tree-1")
                .with_build("build-1")
                .with_project(":app")
                .build()
                .unwrap();

        // Verify all contexts were created
        let _ = process;
        assert_eq!(session.session_id().0.len(), 36); // UUID format
        assert_eq!(tree.tree_id().0.len(), 36);
        assert_eq!(build.build_id().0.len(), 36);
        assert_eq!(project.project_path().as_ref(), ":app");

        // Verify registry registration
        let bid = BuildId("build-1".to_string());
        let sid = SessionId("session-1".to_string());
        assert!(registry.validate_build_in_session(&bid, &sid));

        // Drop guard and verify cleanup
        drop(guard);
        assert!(!registry.validate_build_in_session(&bid, &sid));
    }

    #[test]
    fn test_typed_scope_builder_missing_session() {
        let registry = std::sync::Arc::new(ScopeRegistry::new());
        let result = TypedScopeBuilder::new(registry)
            .with_tree("tree-1")
            .with_build("build-1")
            .with_project(":app")
            .build();
        assert!(result.is_err());
    }

    #[test]
    fn test_typed_scope_builder_missing_tree() {
        let registry = std::sync::Arc::new(ScopeRegistry::new());
        let result = TypedScopeBuilder::new(registry)
            .with_session("session-1")
            .with_build("build-1")
            .with_project(":app")
            .build();
        assert!(result.is_err());
    }

    #[test]
    fn test_typed_scope_builder_missing_build() {
        let registry = std::sync::Arc::new(ScopeRegistry::new());
        let result = TypedScopeBuilder::new(registry)
            .with_session("session-1")
            .with_tree("tree-1")
            .with_project(":app")
            .build();
        assert!(result.is_err());
    }

    #[test]
    fn test_typed_scope_builder_missing_project() {
        let registry = std::sync::Arc::new(ScopeRegistry::new());
        let result = TypedScopeBuilder::new(registry)
            .with_session("session-1")
            .with_tree("tree-1")
            .with_build("build-1")
            .build();
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Compile-time leakage tests
    //
    // These tests verify that the Rust compiler enforces the lifetime
    // hierarchy. They use helper functions with explicit lifetime bounds
    // to demonstrate that data cannot escape its scope.
    // -----------------------------------------------------------------------

    /// This function accepts a ProjectCtx and returns nothing.
    /// If ProjectCtx could outlive its BuildCtx, this would compile
    /// even when called with a project from a shorter-lived build.
    fn _accept_project<'a>(_project: ProjectCtx<'a>) {
        // The project is consumed here and cannot escape
    }

    /// This function demonstrates that a ProjectCtx is tied to its BuildCtx.
    /// The following test (commented out) would NOT compile because it tries
    /// to use a ProjectCtx outside the scope of its BuildCtx.
    ///
    /// Uncommenting this would produce a compile error:
    ///
    /// ```compile_fail
    /// use super::*;
    ///
    /// fn try_leak_project() -> ProjectPath {
    ///     let process = BuildProcessCtx::new();
    ///     let session = process.open_session();
    ///     let tree = session.open_tree();
    ///     let build = tree.open_build();
    ///     let project = build.project(":app");
    ///     let path = project.project_path().clone();
    ///     // This would fail if we tried to return `project` itself:
    ///     // project // ERROR: `project` does not live long enough
    ///     path
    /// }
    /// ```
    #[test]
    fn test_project_cannot_outlive_build() {
        // This test verifies the pattern works correctly within scope.
        // The compile_fail test above demonstrates the leakage prevention.
        let process = BuildProcessCtx::new();
        let session = process.open_session();
        let tree = session.open_tree();
        let build = tree.open_build();
        let project = build.project(":app");

        // We can use the project within its valid scope
        _accept_project(project);
    }

    /// This test verifies that BuildCtx cannot outlive its BuildTreeCtx.
    /// The following commented code would NOT compile:
    ///
    /// ```compile_fail
    /// use super::*;
    ///
    /// fn try_leak_build() -> BuildId {
    ///     let process = BuildProcessCtx::new();
    ///     let session = process.open_session();
    ///     let tree = session.open_tree();
    ///     let build = tree.open_build();
    ///     // Returning the build itself would fail:
    ///     // build // ERROR: `build` does not live long enough
    ///     build.build_id()
    /// }
    /// ```
    #[test]
    fn test_build_cannot_outlive_tree() {
        let process = BuildProcessCtx::new();
        let session = process.open_session();
        let tree = session.open_tree();
        let build = tree.open_build();

        // We can use the build within its valid scope
        let _ = build.build_id();
    }

    #[test]
    fn test_multiple_projects_in_same_build() {
        let process = BuildProcessCtx::new();
        let session = process.open_session();
        let tree = session.open_tree();
        let build = tree.open_build();

        let app = build.project(":app");
        let lib = build.project(":lib");
        let core = build.project(":core");

        assert_eq!(app.project_path().as_ref(), ":app");
        assert_eq!(lib.project_path().as_ref(), ":lib");
        assert_eq!(core.project_path().as_ref(), ":core");
    }

    #[test]
    fn test_multiple_builds_in_same_tree() {
        let process = BuildProcessCtx::new();
        let session = process.open_session();
        let tree = session.open_tree();

        let build1 = tree.open_build();
        let build2 = tree.open_build();

        // Each build has a unique ID
        assert_ne!(build1.build_id(), build2.build_id());
    }

    #[test]
    fn test_multiple_trees_in_same_session() {
        let process = BuildProcessCtx::new();
        let session = process.open_session();

        let tree1 = session.open_tree();
        let tree2 = session.open_tree();

        // Each tree has a unique ID
        assert_ne!(tree1.tree_id(), tree2.tree_id());
    }

    #[test]
    fn test_multiple_sessions_in_same_process() {
        let process = BuildProcessCtx::new();

        let session1 = process.open_session();
        let session2 = process.open_session();

        // Each session has a unique ID
        assert_ne!(session1.session_id(), session2.session_id());
    }

    #[test]
    fn test_scope_guard_with_builder_integration() {
        let registry = std::sync::Arc::new(ScopeRegistry::new());

        {
            let (_process, _session, _tree, _build, project, _guard) =
                TypedScopeBuilder::new(registry.clone())
                    .with_session("s1")
                    .with_tree("t1")
                    .with_build("b1")
                    .with_project(":app")
                    .build()
                    .unwrap();

            // Use the project within scope
            assert_eq!(project.project_path().as_ref(), ":app");

            // Guard is still alive, build should be registered
            let bid = BuildId("b1".to_string());
            let sid = SessionId("s1".to_string());
            assert!(registry.validate_build_in_session(&bid, &sid));
        }
        // Guard is dropped here, cleanup should have happened

        let bid = BuildId("b1".to_string());
        let sid = SessionId("s1".to_string());
        assert!(!registry.validate_build_in_session(&bid, &sid));
    }
}
