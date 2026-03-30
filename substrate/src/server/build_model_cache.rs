//! In-memory cache of Gradle build model data for IDE queries.
//!
//! Populated from JVM via JvmHostService during bootstrap.
//! Invalidated on file watch events (build script changes).

use std::collections::HashMap;
use std::sync::Arc;
use dashmap::DashMap;

/// A cached project model entry.
#[derive(Debug, Clone)]
pub struct CachedProject {
    pub name: String,
    pub path: String,
    pub description: String,
    pub tasks: Vec<String>,
    pub source_set_names: Vec<String>,
}

/// A cached dependency entry.
#[derive(Debug, Clone)]
pub struct CachedDependency {
    pub group: String,
    pub artifact: String,
    pub version: String,
    pub configuration: String,
}

/// In-memory cache of build model data keyed by project path.
#[derive(Debug, Clone, Default)]
pub struct BuildModelCache {
    projects: Arc<DashMap<String, CachedProject>>,
    dependencies: Arc<DashMap<String, Vec<CachedDependency>>>,
}

impl BuildModelCache {
    /// Create a new empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or update a project in the cache.
    pub fn put_project(&self, project: CachedProject) {
        self.projects.insert(project.path.clone(), project);
    }

    /// Get a project by path.
    pub fn get_project(&self, path: &str) -> Option<CachedProject> {
        self.projects.get(path).map(|r| r.value().clone())
    }

    /// List all cached project paths.
    pub fn project_paths(&self) -> Vec<String> {
        self.projects.iter().map(|r| r.key().clone()).collect()
    }

    /// Insert dependencies for a project.
    pub fn put_dependencies(&self, project_path: String, deps: Vec<CachedDependency>) {
        self.dependencies.insert(project_path, deps);
    }

    /// Get dependencies for a project.
    pub fn get_dependencies(&self, project_path: &str) -> Option<Vec<CachedDependency>> {
        self.dependencies.get(project_path).map(|r| r.value().clone())
    }

    /// Invalidate cache entries for a project (called on build script changes).
    pub fn invalidate_project(&self, path: &str) {
        self.projects.remove(path);
        self.dependencies.remove(path);
    }

    /// Invalidate all cached data.
    pub fn invalidate_all(&self) {
        self.projects.clear();
        self.dependencies.clear();
    }

    /// Returns the number of cached projects.
    pub fn project_count(&self) -> usize {
        self.projects.len()
    }

    /// Returns the number of cached dependency entries.
    pub fn dependency_count(&self) -> usize {
        self.dependencies.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_put_and_get_project() {
        let cache = BuildModelCache::new();
        let project = CachedProject {
            name: "app".to_string(),
            path: ":app".to_string(),
            description: "Main app".to_string(),
            tasks: vec!["build".to_string(), "test".to_string()],
            source_set_names: vec!["main".to_string(), "test".to_string()],
        };
        cache.put_project(project.clone());
        let retrieved = cache.get_project(":app").unwrap();
        assert_eq!(retrieved.name, "app");
        assert_eq!(retrieved.tasks.len(), 2);
    }

    #[test]
    fn test_project_paths() {
        let cache = BuildModelCache::new();
        cache.put_project(CachedProject {
            name: "a".to_string(),
            path: ":a".to_string(),
            description: String::new(),
            tasks: vec![],
            source_set_names: vec![],
        });
        cache.put_project(CachedProject {
            name: "b".to_string(),
            path: ":b".to_string(),
            description: String::new(),
            tasks: vec![],
            source_set_names: vec![],
        });
        let mut paths = cache.project_paths();
        paths.sort();
        assert_eq!(paths, vec![":a", ":b"]);
    }

    #[test]
    fn test_invalidate_project() {
        let cache = BuildModelCache::new();
        cache.put_project(CachedProject {
            name: "app".to_string(),
            path: ":app".to_string(),
            description: String::new(),
            tasks: vec![],
            source_set_names: vec![],
        });
        cache.invalidate_project(":app");
        assert!(cache.get_project(":app").is_none());
        assert_eq!(cache.project_count(), 0);
    }

    #[test]
    fn test_invalidate_all() {
        let cache = BuildModelCache::new();
        cache.put_project(CachedProject {
            name: "a".to_string(),
            path: ":a".to_string(),
            description: String::new(),
            tasks: vec![],
            source_set_names: vec![],
        });
        cache.put_project(CachedProject {
            name: "b".to_string(),
            path: ":b".to_string(),
            description: String::new(),
            tasks: vec![],
            source_set_names: vec![],
        });
        cache.invalidate_all();
        assert_eq!(cache.project_count(), 0);
    }

    #[test]
    fn test_dependencies() {
        let cache = BuildModelCache::new();
        let deps = vec![CachedDependency {
            group: "org.example".to_string(),
            artifact: "lib".to_string(),
            version: "1.0".to_string(),
            configuration: "implementation".to_string(),
        }];
        cache.put_dependencies(":app".to_string(), deps.clone());
        let retrieved = cache.get_dependencies(":app").unwrap();
        assert_eq!(retrieved.len(), 1);
        assert_eq!(retrieved[0].artifact, "lib");
    }

    #[test]
    fn test_missing_project_returns_none() {
        let cache = BuildModelCache::new();
        assert!(cache.get_project(":nonexistent").is_none());
    }
}
