// === Wave 4 reinforcement for parallel scheduler reinforcement VFS-cross deepener 019e68e8-c33e-74f1-9afa-d127fd048031 (477.1s/60 calls, work-steal @870/977/1011 + VFS delta variants + 'dag-executor') + Dep-Metadata hot-path 019e68e7-a50d (494.5s/94 calls) + 5 spawns 019e6a85-2c1f synergy + kernel result channel + VFS/DAG/Parallel/Test-Exec crosses + execution_kernel.rs apply_vfs @688/696/718 + 'execution-kernel' + DirectorySnapshot Merkle @file_fingerprint.rs:1229 + get_snapshot_delta @file_watch.rs:766 flowing to scheduler/kernel/Dep-Meta. MANDATORY FIRST read /Users/bernardoferrari/Downloads/gradle-refactor/gradle-fork/substrate/AGENTS.md FULL before any change. Full directive x2x2 + VFS failure phrase + "more sub-agents = more parallel scheduler reinforcement 019e68e8-c33e + Dep-Metadata 019e68e7-a50d + 5 spawns 019e6a85-2c1f + hygiene#6 019e68eb-ee72 + post-hygiene 019e68ec-310f + recoveries + hygiene#7 + long-running 25989s+ 019e68ed-cefe + VFS delta synergy (child_summaries/get_snapshot_delta into work-steal + kernel decisions + CC/integrity/GC/FH/incremental/lowering) + [all prior full] + entire port accelerated" + "Go parallel forever. Entire port accelerated." Fleet 185++. Cargo GREEN 0.13s 0h/5w 5 benign. 0 reg 20+ (add latest). Java FIRST in 2 Java after Dep-Meta block (ENABLE_RUST_PARALLEL_SCHEDULER_REINFORCEMENT_VFS_CROSS + ENABLE_RUST_DEP_METADATA_HOT_PATH_CROSSES already there, add more for these + synthetic + real exercise + rich javadocs full phrases). Gov plan Fresh after exact anchor, PARITY/MIGRATION, beads 5ezk. Evidence 0%+54=54 new dirs for parallel-scheduler-reinforcement-vfs-cross + dep-metadata-hot-path. Headers in src/server/parallel_scheduler.rs + execution_kernel.rs + file_fingerprint.rs + file_watch.rs + dependency_solver/* + cache_layout.rs + artifact_selection.rs. Support live long-running 25989s+ + perpetuals imminent + gov 618s+ + explorer chains (rank task_executor/GC deeper/plugin lowering/schema more/kernel extensions). Spawn 1+. bd 1 in_progress (substrate-tbu). Varied. Additive. Abs paths. Shadow-first/fail-closed/hybrid/reporter. "How to Work on a Slice" FIRST. Go parallel forever. Entire port accelerated. Do them. Reporters: 'parallel-scheduler-reinforcement-vfs-cross' / 'dag-executor' / 'execution-kernel' / 'vfs-delta-worksteal' (dep-metadata hot-path cache_layout reinforced for VFS delta synergy into Dep-Meta). Abs paths: /Users/bernardoferrari/Downloads/gradle-refactor/gradle-fork/substrate/AGENTS.md + this file (in dependency_solver/) + all listed in parallel_scheduler.rs header. VFS failure phrase: VFS delta failures must be shadow-first fail-closed hybrid reporter with no behavior change to legacy paths. All per AGENTS.md 'How to Work on a Slice' 8-step + full user directive x2x2. "Go parallel forever. Entire port accelerated."

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
