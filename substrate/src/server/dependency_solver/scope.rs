//! Dependency scope policy for resolved graphs.

use crate::proto::ResolvedDependency;

/// Dependency scope classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyScope {
    Compile,
    Runtime,
    Test,
    Provided,
    System,
}

impl DependencyScope {
    /// Parse a scope string (case-insensitive).
    pub fn from_str_loose(s: &str) -> Self {
        match s.as_bytes() {
            b"compile" | b"compileonly" | b"api" | b"Compile" | b"CompileOnly" | b"Api"
            | b"COMPILE" | b"COMPILEONLY" | b"API" => DependencyScope::Compile,
            b"runtime" | b"implementation" | b"runtimeonly" | b"Runtime" | b"Implementation"
            | b"RuntimeOnly" | b"RUNTIME" | b"IMPLEMENTATION" | b"RUNTIMEONLY" => {
                DependencyScope::Runtime
            }
            b"test"
            | b"testimplementation"
            | b"testruntimeonly"
            | b"Test"
            | b"TestImplementation"
            | b"TestRuntimeOnly"
            | b"TEST"
            | b"TESTIMPLEMENTATION"
            | b"TESTRUNTIMEONLY" => DependencyScope::Test,
            b"provided" | b"Provided" | b"PROVIDED" => DependencyScope::Provided,
            b"system" | b"System" | b"SYSTEM" => DependencyScope::System,
            _ => DependencyScope::Compile,
        }
    }

    /// Returns true if this scope includes the given dependency scope.
    pub fn includes(&self, other: &DependencyScope) -> bool {
        match self {
            DependencyScope::Compile => matches!(
                other,
                DependencyScope::Compile | DependencyScope::Provided | DependencyScope::System
            ),
            DependencyScope::Runtime => {
                matches!(other, DependencyScope::Compile | DependencyScope::Runtime)
            }
            DependencyScope::Test => true,
            DependencyScope::Provided => {
                matches!(other, DependencyScope::Compile | DependencyScope::Provided)
            }
            DependencyScope::System => matches!(other, DependencyScope::System),
        }
    }

    /// Scopes that are transitively inherited.
    pub fn transitive_scopes(&self) -> Vec<DependencyScope> {
        match self {
            DependencyScope::Compile => vec![DependencyScope::Compile, DependencyScope::Runtime],
            DependencyScope::Runtime => vec![DependencyScope::Runtime],
            DependencyScope::Test => vec![DependencyScope::Compile, DependencyScope::Runtime],
            DependencyScope::Provided => vec![],
            DependencyScope::System => vec![],
        }
    }
}

/// Filter resolved dependencies by target scope recursively.
pub fn filter_by_scope(
    deps: Vec<ResolvedDependency>,
    target: &DependencyScope,
) -> Vec<ResolvedDependency> {
    deps.into_iter()
        .filter_map(|mut dep| {
            let dep_scope = DependencyScope::from_str_loose(&dep.scope);
            if target.includes(&dep_scope) {
                dep.dependencies = filter_by_scope(dep.dependencies, target);
                Some(dep)
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dep(name: &str, scope: &str) -> ResolvedDependency {
        ResolvedDependency {
            group: "org.example".to_string(),
            name: name.to_string(),
            version: "1.0".to_string(),
            selected_version: "1.0".to_string(),
            dependencies: Vec::new(),
            resolved: true,
            failure_reason: String::new(),
            artifact_url: String::new(),
            artifact_size: 0,
            artifact_sha256: String::new(),
            scope: scope.to_string(),
        }
    }

    #[test]
    fn compile_scope_excludes_runtime_and_test_recursively() {
        let mut compile = dep("compile-lib", "compile");
        compile.dependencies = vec![
            dep("runtime-child", "runtime"),
            dep("provided-child", "provided"),
        ];
        let filtered = filter_by_scope(
            vec![
                compile,
                dep("runtime-lib", "runtime"),
                dep("test-lib", "test"),
            ],
            &DependencyScope::Compile,
        );

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "compile-lib");
        assert_eq!(filtered[0].dependencies.len(), 1);
        assert_eq!(filtered[0].dependencies[0].name, "provided-child");
    }

    #[test]
    fn runtime_scope_keeps_compile_and_runtime_only() {
        let filtered = filter_by_scope(
            vec![
                dep("compile-lib", "compile"),
                dep("runtime-lib", "runtime"),
                dep("provided-lib", "provided"),
                dep("test-lib", "test"),
            ],
            &DependencyScope::Runtime,
        );

        assert_eq!(
            filtered
                .iter()
                .map(|dependency| dependency.name.as_str())
                .collect::<Vec<_>>(),
            vec!["compile-lib", "runtime-lib"]
        );
    }
}
