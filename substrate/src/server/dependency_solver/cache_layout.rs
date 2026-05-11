//! Cache-key and store-path layout for native dependency resolution.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::proto::RepositoryDescriptor;

use super::artifact_selection::{artifact_cache_key, group_to_path, normalize_extension};

pub(crate) fn artifact_path(
    store_dir: &Path,
    group: &str,
    name: &str,
    version: &str,
    classifier: &str,
    extension: &str,
) -> PathBuf {
    let group_path = group_to_path(group);
    let filename = if classifier.is_empty() {
        format!("{name}-{version}.{extension}")
    } else {
        format!("{name}-{version}-{classifier}.{extension}")
    };
    store_dir
        .join(group_path)
        .join(name)
        .join(version)
        .join(filename)
}

pub(crate) fn repository_cache_id(repo: &RepositoryDescriptor) -> String {
    let digest = sha256_hex(repo.url.as_bytes());
    digest[..16].to_string()
}

pub(crate) fn metadata_cache_key(
    repo: &RepositoryDescriptor,
    group: &str,
    name: &str,
    version: &str,
    extension: &str,
) -> String {
    let extension = normalize_extension(extension);
    let repo_id = repository_cache_id(repo);
    let mut key = String::with_capacity(
        "metadata:".len()
            + repo_id.len()
            + group.len()
            + name.len()
            + version.len()
            + extension.len()
            + 4,
    );
    key.push_str("metadata:");
    key.push_str(&repo_id);
    key.push(':');
    key.push_str(group);
    key.push(':');
    key.push_str(name);
    key.push(':');
    key.push_str(version);
    key.push(':');
    key.push_str(&extension);
    key
}

pub(crate) fn module_metadata_cache_key(
    repo: &RepositoryDescriptor,
    group: &str,
    name: &str,
    extension: &str,
) -> String {
    let extension = normalize_extension(extension);
    let repo_id = repository_cache_id(repo);
    let mut key = String::with_capacity(
        "module-metadata:".len() + repo_id.len() + group.len() + name.len() + extension.len() + 3,
    );
    key.push_str("module-metadata:");
    key.push_str(&repo_id);
    key.push(':');
    key.push_str(group);
    key.push(':');
    key.push_str(name);
    key.push(':');
    key.push_str(&extension);
    key
}

pub(crate) fn metadata_url_cache_key(url: &str, extension: &str) -> String {
    let extension = normalize_extension(extension);
    let digest = sha256_hex(url.as_bytes());
    let mut key = String::with_capacity("metadata-url:".len() + digest.len() + extension.len() + 1);
    key.push_str("metadata-url:");
    key.push_str(&digest);
    key.push(':');
    key.push_str(&extension);
    key
}

pub(crate) fn metadata_path(
    store_dir: &Path,
    repo: &RepositoryDescriptor,
    group: &str,
    name: &str,
    version: &str,
    extension: &str,
) -> PathBuf {
    let filename = format!("{name}-{version}.{}", normalize_extension(extension));
    store_dir
        .join("_metadata")
        .join(repository_cache_id(repo))
        .join(group_to_path(group))
        .join(name)
        .join(version)
        .join(filename)
}

pub(crate) fn module_metadata_path(
    store_dir: &Path,
    repo: &RepositoryDescriptor,
    group: &str,
    name: &str,
    extension: &str,
) -> PathBuf {
    store_dir
        .join("_metadata")
        .join(repository_cache_id(repo))
        .join(group_to_path(group))
        .join(name)
        .join(normalize_extension(extension))
}

pub(crate) fn metadata_url_path(store_dir: &Path, url: &str, extension: &str) -> PathBuf {
    let digest = sha256_hex(url.as_bytes());
    store_dir
        .join("_metadata")
        .join("by-url")
        .join(format!("{digest}.{}", normalize_extension(extension)))
}

pub(crate) fn coordinate_artifact_cache_key(
    group: &str,
    name: &str,
    version: &str,
    classifier: &str,
    extension: &str,
) -> String {
    artifact_cache_key(group, name, version, classifier, extension)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(url: &str) -> RepositoryDescriptor {
        RepositoryDescriptor {
            id: "repo".to_string(),
            url: url.to_string(),
            m2compatible: true,
            allow_insecure_protocol: false,
            credentials: Default::default(),
            layout: String::new(),
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
    fn artifact_layout_uses_maven_coordinates() {
        assert_eq!(
            artifact_path(
                Path::new("/store"),
                "com.example",
                "demo",
                "1.0",
                "sources",
                "jar"
            ),
            PathBuf::from("/store/com/example/demo/1.0/demo-1.0-sources.jar")
        );
    }

    #[test]
    fn metadata_keys_are_repository_scoped() {
        let primary_repo = repo("https://repo.example.test/maven");
        let key = metadata_cache_key(&primary_repo, "com.example", "demo", "1.0", ".pom");
        assert!(key.starts_with("metadata:"));
        assert!(key.ends_with(":com.example:demo:1.0:pom"));
        assert_ne!(
            key,
            metadata_cache_key(
                &repo("https://repo2.example.test/maven"),
                "com.example",
                "demo",
                "1.0",
                ".pom"
            )
        );
    }
}
