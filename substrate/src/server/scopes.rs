//! Typed scope identifiers for Gradle's service scope hierarchy.
//!
//! Gradle scopes: Global → UserHome → BuildSession → BuildTree → Build → Project
//!
//! These newtype wrappers prevent accidentally passing a build-scoped ID where
//! a session-scoped ID is expected, catching cross-scope data leaks at compile time.

use std::collections::HashSet;
use std::fmt;
use std::hash::Hash;

use dashmap::DashMap;

// ---------------------------------------------------------------------------
// Scope identifier newtypes
// ---------------------------------------------------------------------------

/// Identifies a single build execution within a build tree.
///
/// Build-scoped data is created when a build starts and discarded when it completes.
/// Multiple builds can exist within a session (e.g., composite builds).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BuildId(pub String);

/// Identifies a Gradle build session (one `gradle` invocation).
///
/// Session-scoped data persists across multiple builds within a continuous build
/// (e.g., `--continuous` mode).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);

/// Identifies a build tree (used for composite builds).
///
/// A build tree contains one or more builds that execute together.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TreeId(pub String);

/// Identifies a project within a build.
///
/// Project-scoped data is specific to a single subproject.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProjectPath(pub String);

// ---------------------------------------------------------------------------
// Trait implementations for interop with proto (String) layer
// ---------------------------------------------------------------------------

impl AsRef<str> for BuildId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for SessionId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for TreeId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for ProjectPath {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BuildId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for TreeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Display for ProjectPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for BuildId {
    fn from(s: String) -> Self {
        BuildId(s)
    }
}

impl From<String> for SessionId {
    fn from(s: String) -> Self {
        SessionId(s)
    }
}

impl From<String> for TreeId {
    fn from(s: String) -> Self {
        TreeId(s)
    }
}

impl From<String> for ProjectPath {
    fn from(s: String) -> Self {
        ProjectPath(s)
    }
}

impl From<BuildId> for String {
    fn from(id: BuildId) -> Self {
        id.0
    }
}

impl From<SessionId> for String {
    fn from(id: SessionId) -> Self {
        id.0
    }
}

impl From<TreeId> for String {
    fn from(id: TreeId) -> Self {
        id.0
    }
}

impl From<ProjectPath> for String {
    fn from(id: ProjectPath) -> Self {
        id.0
    }
}

// ---------------------------------------------------------------------------
// Scope Registry — tracks session → build membership
// ---------------------------------------------------------------------------

/// Tracks which builds belong to which sessions and trees.
/// Enables runtime validation of scope membership.
#[derive(Default)]
pub struct ScopeRegistry {
    /// session_id → set of build_ids belonging to that session
    sessions: DashMap<SessionId, HashSet<BuildId>>,
    /// build_id → session_id (reverse lookup)
    build_to_session: DashMap<BuildId, SessionId>,
    /// tree_id → set of build_ids in that tree
    trees: DashMap<TreeId, HashSet<BuildId>>,
}

impl ScopeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a build as belonging to a session.
    pub fn register_build(&self, session_id: SessionId, build_id: BuildId) {
        self.build_to_session
            .insert(build_id.clone(), session_id.clone());
        self.sessions
            .entry(session_id)
            .or_default()
            .insert(build_id);
    }

    /// Associate a build with a tree.
    pub fn register_tree(&self, tree_id: TreeId, build_id: BuildId) {
        self.trees.entry(tree_id).or_default().insert(build_id);
    }

    /// Look up the session that owns a build.
    pub fn session_for_build(&self, build_id: &BuildId) -> Option<SessionId> {
        self.build_to_session
            .get(build_id)
            .map(|r| r.value().clone())
    }

    /// Validate that a build belongs to a session.
    pub fn validate_build_in_session(&self, build_id: &BuildId, session_id: &SessionId) -> bool {
        self.build_to_session
            .get(build_id)
            .map(|r| r.value() == session_id)
            .unwrap_or(false)
    }

    /// Remove a build from all tracking (called when build completes).
    pub fn cleanup_build(&self, build_id: &BuildId) {
        if let Some((_, session_id)) = self.build_to_session.remove(build_id) {
            if let Some(mut session_builds) = self.sessions.get_mut(&session_id) {
                session_builds.remove(build_id);
            }
        }
        // Also remove from all trees
        for mut tree_builds in self.trees.iter_mut() {
            tree_builds.remove(build_id);
        }
    }

    /// Get all build IDs for a session.
    pub fn builds_in_session(&self, session_id: &SessionId) -> Vec<BuildId> {
        self.sessions
            .get(session_id)
            .map(|r| r.value().iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Get all build IDs for a tree.
    pub fn builds_in_tree(&self, tree_id: &TreeId) -> Vec<BuildId> {
        self.trees
            .get(tree_id)
            .map(|r| r.value().iter().cloned().collect())
            .unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_id_from_string() {
        let id = BuildId::from("test-build".to_string());
        assert_eq!(id.as_ref(), "test-build");
        assert_eq!(format!("{}", id), "test-build");
    }

    #[test]
    fn test_build_id_into_string() {
        let id = BuildId("my-build".to_string());
        let s: String = id.into();
        assert_eq!(s, "my-build");
    }

    #[test]
    fn test_scope_types_are_distinct() {
        // These should NOT be interchangeable at the type level.
        // This test verifies the types exist and compile separately.
        let _build: BuildId = BuildId::from("b1".to_string());
        let _session: SessionId = SessionId::from("s1".to_string());
        let _tree: TreeId = TreeId::from("t1".to_string());
        let _project: ProjectPath = ProjectPath::from(":app".to_string());
    }

    #[test]
    fn test_scope_registry_register_and_lookup() {
        let registry = ScopeRegistry::new();
        let session = SessionId::from("session-1".to_string());
        let build = BuildId::from("build-1".to_string());

        registry.register_build(session.clone(), build.clone());

        assert_eq!(registry.session_for_build(&build), Some(session.clone()));
        assert!(registry.validate_build_in_session(&build, &session));
        assert!(!registry
            .validate_build_in_session(&build, &SessionId::from("other-session".to_string())));
    }

    #[test]
    fn test_scope_registry_multiple_builds_per_session() {
        let registry = ScopeRegistry::new();
        let session = SessionId::from("session-1".to_string());
        let b1 = BuildId::from("build-1".to_string());
        let b2 = BuildId::from("build-2".to_string());

        registry.register_build(session.clone(), b1.clone());
        registry.register_build(session.clone(), b2.clone());

        let builds = registry.builds_in_session(&session);
        assert_eq!(builds.len(), 2);
    }

    #[test]
    fn test_scope_registry_cleanup() {
        let registry = ScopeRegistry::new();
        let session = SessionId::from("session-1".to_string());
        let build = BuildId::from("build-1".to_string());

        registry.register_build(session.clone(), build.clone());
        registry.cleanup_build(&build);

        assert_eq!(registry.session_for_build(&build), None);
        assert_eq!(registry.builds_in_session(&session).len(), 0);
    }

    #[test]
    fn test_scope_registry_tree() {
        let registry = ScopeRegistry::new();
        let tree = TreeId::from("tree-1".to_string());
        let b1 = BuildId::from("build-1".to_string());
        let b2 = BuildId::from("build-2".to_string());

        registry.register_tree(tree.clone(), b1.clone());
        registry.register_tree(tree.clone(), b2.clone());

        let builds = registry.builds_in_tree(&tree);
        assert_eq!(builds.len(), 2);
    }
}
