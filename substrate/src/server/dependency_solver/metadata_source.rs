use crate::proto::RepositoryDescriptor;

/// Return true when the captured repository metadata-source contract admits
/// Gradle Module Metadata before Maven POM fallback.
pub fn supports_gradle_module_metadata(repo: &RepositoryDescriptor) -> bool {
    matches!(repo.layout.as_str(), "gradle-module-metadata" | "gradle")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(layout: &str) -> RepositoryDescriptor {
        RepositoryDescriptor {
            id: "repo".to_string(),
            url: "https://repo.example.test/maven".to_string(),
            m2compatible: true,
            allow_insecure_protocol: false,
            credentials: Default::default(),
            layout: layout.to_string(),
            ivy_pattern: String::new(),
            include_groups: Vec::new(),
            exclude_groups: Vec::new(),
            include_group_prefixes: Vec::new(),
            exclude_group_prefixes: Vec::new(),
            include_modules: Vec::new(),
            exclude_modules: Vec::new(),
            include_module_versions: Vec::new(),
            exclude_module_versions: Vec::new(),
        }
    }

    #[test]
    fn gradle_metadata_source_layouts_admit_module_metadata() {
        assert!(supports_gradle_module_metadata(&repo("gradle")));
        assert!(supports_gradle_module_metadata(&repo(
            "gradle-module-metadata"
        )));
        assert!(!supports_gradle_module_metadata(&repo("maven-pom")));
        assert!(!supports_gradle_module_metadata(&repo("")));
    }
}
