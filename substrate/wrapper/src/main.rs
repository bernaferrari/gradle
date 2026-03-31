//! Rust-native Gradle wrapper binary.
//!
//! Reads `gradle-wrapper.properties`, downloads/verifies the distribution ZIP,
//! and launches Gradle. Optionally launches the substrate daemon if present.

use std::fmt::Write as FmtWrite;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};

// ---------------------------------------------------------------------------
// Properties
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct WrapperProperties {
    distribution_url: String,
    distribution_base: String,
    distribution_path: String,
    zip_store_base: String,
    zip_store_path: String,
    distribution_sha256_sum: Option<String>,
    network_timeout: u64,
    validate_distribution_url: bool,
}

impl Default for WrapperProperties {
    fn default() -> Self {
        Self {
            distribution_url: String::new(),
            distribution_base: "GRADLE_USER_HOME".to_string(),
            distribution_path: "wrapper/dists".to_string(),
            zip_store_base: "GRADLE_USER_HOME".to_string(),
            zip_store_path: "wrapper/dists".to_string(),
            distribution_sha256_sum: None,
            network_timeout: 10000,
            validate_distribution_url: true,
        }
    }
}

/// Unescape Java properties escapes (e.g. `\:` → `:`).
fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn parse_properties(content: &str) -> Result<WrapperProperties, String> {
    let mut props = WrapperProperties::default();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = unescape(value.trim());
            match key {
                "distributionUrl" => props.distribution_url = value,
                "distributionBase" => props.distribution_base = value,
                "distributionPath" => props.distribution_path = value,
                "zipStoreBase" => props.zip_store_base = value,
                "zipStorePath" => props.zip_store_path = value,
                "distributionSha256Sum" => {
                    if !value.is_empty() {
                        props.distribution_sha256_sum = Some(value);
                    }
                }
                "networkTimeout" => {
                    if let Ok(t) = value.parse() {
                        props.network_timeout = t;
                    }
                }
                "validateDistributionUrl" => {
                    props.validate_distribution_url =
                        value == "true" || value == "TRUE" || value == "1";
                }
                _ => {} // ignore unknown keys
            }
        }
    }
    if props.distribution_url.is_empty() {
        return Err("distributionUrl is required".to_string());
    }
    Ok(props)
}

fn load_properties(project_dir: &Path) -> Result<WrapperProperties, String> {
    let props_path = project_dir.join("gradle").join("wrapper").join("gradle-wrapper.properties");
    let content = std::fs::read_to_string(&props_path)
        .map_err(|e| format!("Failed to read {}: {}", props_path.display(), e))?;
    parse_properties(&content)
}

// ---------------------------------------------------------------------------
// Path assembler
// ---------------------------------------------------------------------------

fn resolve_base(key: &str) -> PathBuf {
    match key {
        "GRADLE_USER_HOME" => {
            if let Ok(home) = std::env::var("GRADLE_USER_HOME") {
                PathBuf::from(home)
            } else if let Ok(home) = std::env::var("HOME") {
                PathBuf::from(home).join(".gradle")
            } else {
                PathBuf::from(".gradle")
            }
        }
        "PROJECT" => PathBuf::from("."),
        other => PathBuf::from(other),
    }
}

fn url_hash(url: &str) -> String {
    let digest = Sha256::digest(url.as_bytes());
    digest[..8].iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{:02x}", b);
        s
    })
}

fn extract_version_from_url(url: &str) -> String {
    // URL format: .../gradle-{version}-bin.zip or .../gradle-{version}-all.zip
    let filename = url.rsplit('/').next().unwrap_or(url);
    let name = filename.strip_suffix("-bin.zip")
        .or_else(|| filename.strip_suffix("-all.zip"))
        .unwrap_or(filename);
    name.strip_prefix("gradle-").unwrap_or(name).to_string()
}

struct InstallationPaths {
    dist_dir: PathBuf,
    zip_path: PathBuf,
    marker_path: PathBuf,
}

fn build_paths(props: &WrapperProperties) -> InstallationPaths {
    let version = extract_version_from_url(&props.distribution_url);
    let hash = url_hash(&props.distribution_url);
    let dir_name = format!("{}-{}", version, hash);

    let dist_base = resolve_base(&props.distribution_base);
    let dist_dir = dist_base.join(&props.distribution_path).join(&dir_name);

    let zip_base = resolve_base(&props.zip_store_base);
    let zip_path = zip_base
        .join(&props.zip_store_path)
        .join(&dir_name)
        .join(format!("{}-bin.zip", version));

    let marker_path = dist_dir.join(".ok");

    InstallationPaths {
        dist_dir,
        zip_path,
        marker_path,
    }
}

// ---------------------------------------------------------------------------
// Download + verify + install
// ---------------------------------------------------------------------------

fn download_with_progress(url: &str, dest: &Path, timeout_ms: u64) -> Result<(), String> {
    eprintln!("Downloading {}...", url);

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_millis(timeout_ms))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let mut response = client
        .get(url)
        .send()
        .map_err(|e| format!("Download failed: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }

    let total = response.content_length().unwrap_or(0);
    let mut downloaded: u64 = 0;

    // Create parent directory
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create directory: {}", e))?;
    }

    let mut file = std::fs::File::create(dest)
        .map_err(|e| format!("Failed to create file: {}", e))?;

    let mut buf = [0u8; 8192];
    loop {
        let n = response
            .read(&mut buf)
            .map_err(|e| format!("Read error: {}", e))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n])
            .map_err(|e| format!("Write error: {}", e))?;
        downloaded += n as u64;
        if total > 0 {
            let percent = (downloaded as f64 / total as f64) * 100.0;
            eprint!("\r  {}%", percent as u32);
        } else {
            eprint!("\r  {} bytes", downloaded);
        }
    }
    eprintln!();
    Ok(())
}

fn verify_sha256(path: &Path, expected: &str) -> Result<(), String> {
    let data = std::fs::read(path).map_err(|e| format!("Failed to read ZIP: {}", e))?;
    let digest = Sha256::digest(&data);
    let hex: String = digest.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{:02x}", b);
        s
    });
    if hex == expected {
        Ok(())
    } else {
        Err(format!(
            "SHA-256 mismatch: expected {} got {}",
            expected, hex
        ))
    }
}

fn install_distribution(props: &WrapperProperties, paths: &InstallationPaths) -> Result<(), String> {
    // Already installed?
    if paths.marker_path.exists() {
        return Ok(());
    }

    // Download
    download_with_progress(&props.distribution_url, &paths.zip_path, props.network_timeout)?;

    // Verify SHA-256
    if let Some(ref expected) = props.distribution_sha256_sum {
        verify_sha256(&paths.zip_path, expected)?;
        eprintln!("SHA-256 verified.");
    }

    // Extract ZIP
    eprintln!("Extracting distribution...");
    let file = std::fs::File::open(&paths.zip_path)
        .map_err(|e| format!("Failed to open ZIP: {}", e))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("Failed to read ZIP: {}", e))?;

    std::fs::create_dir_all(&paths.dist_dir)
        .map_err(|e| format!("Failed to create dist dir: {}", e))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read entry: {}", e))?;
        let outpath = match entry.enclosed_name() {
            Some(p) => paths.dist_dir.join(p),
            None => continue,
        };

        if entry.is_dir() {
            std::fs::create_dir_all(&outpath)
                .map_err(|e| format!("Failed to create dir: {}", e))?;
            continue;
        }

        if let Some(parent) = outpath.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create parent: {}", e))?;
        }

        let mut outfile = std::fs::File::create(&outpath)
            .map_err(|e| format!("Failed to create file: {}", e))?;
        io::copy(&mut entry, &mut outfile)
            .map_err(|e| format!("Failed to write file: {}", e))?;
    }

    // Verify launcher JAR exists
    let lib_dir = find_lib_dir(&paths.dist_dir)?;
    let launcher = find_file_with_prefix(&lib_dir, "gradle-launcher-", ".jar")
        .ok_or_else(|| "gradle-launcher-*.jar not found in distribution lib/".to_string())?;

    // Write marker
    std::fs::write(&paths.marker_path, "")
        .map_err(|e| format!("Failed to write marker: {}", e))?;

    eprintln!(
        "Installed to {}",
        paths.dist_dir.display()
    );
    drop(launcher);
    Ok(())
}

fn find_lib_dir(dist_dir: &Path) -> Result<PathBuf, String> {
    // The ZIP contains a single top-level directory
    for entry in std::fs::read_dir(dist_dir)
        .map_err(|e| format!("Failed to read dist dir: {}", e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read entry: {}", e))?;
        if entry.path().is_dir() {
            let lib = entry.path().join("lib");
            if lib.exists() {
                return Ok(lib);
            }
        }
    }
    Err("Could not find lib/ directory in distribution".to_string())
}

fn find_file_with_prefix(dir: &Path, prefix: &str, suffix: &str) -> Option<PathBuf> {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(prefix) && name.ends_with(suffix) {
                return Some(entry.path());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Gradle launcher
// ---------------------------------------------------------------------------

fn find_java() -> Result<PathBuf, String> {
    // 1. JAVA_HOME
    if let Ok(java_home) = std::env::var("JAVA_HOME") {
        let java = PathBuf::from(&java_home).join("bin").join("java");
        if java.exists() {
            return Ok(java);
        }
        // Windows
        let java = PathBuf::from(&java_home).join("bin").join("java.exe");
        if java.exists() {
            return Ok(java);
        }
    }

    // 2. PATH
    let java = which("java").ok_or_else(|| "java not found. Set JAVA_HOME or add java to PATH.".to_string())?;
    Ok(java)
}

fn which(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn launch_gradle(dist_dir: &Path, args: &[String]) -> Result<i32, String> {
    let java = find_java()?;

    let lib_dir = find_lib_dir(dist_dir)?;
    let launcher = find_file_with_prefix(&lib_dir, "gradle-launcher-", ".jar")
        .ok_or_else(|| "gradle-launcher-*.jar not found".to_string())?;

    let mut cmd = Command::new(&java);
    cmd.arg("-cp")
        .arg(&launcher)
        .arg("org.gradle.launcher.GradleMain");
    cmd.args(args);
    cmd.stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    let status = cmd.status().map_err(|e| format!("Failed to launch Gradle: {}", e))?;
    Ok(status.code().unwrap_or(1))
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn print_usage() {
    eprintln!("Usage: gradlew [gradle-args...]");
    eprintln!();
    eprintln!("Rust-native Gradle wrapper. Reads gradle-wrapper.properties,");
    eprintln!("downloads the distribution if needed, and launches Gradle.");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && (args[1] == "--help" || args[1] == "-h") {
        print_usage();
        std::process::exit(0);
    }

    // Find project root (directory containing gradle/wrapper/gradle-wrapper.properties)
    let project_dir = find_project_root().unwrap_or_else(|| PathBuf::from("."));

    let props = match load_properties(&project_dir) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    };

    let paths = build_paths(&props);

    if let Err(e) = install_distribution(&props, &paths) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }

    // Pass through args (skip our own binary name)
    let gradle_args: Vec<String> = args.into_iter().skip(1).collect();

    match launch_gradle(&paths.dist_dir, &gradle_args) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }
}

fn find_project_root() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join("gradle").join("wrapper").join("gradle-wrapper.properties").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_properties_full() {
        let content = "\
distributionBase=GRADLE_USER_HOME
distributionPath=wrapper/dists
distributionUrl=https\\://services.gradle.org/distributions/gradle-9.5-bin.zip
networkTimeout=10000
validateDistributionUrl=true
zipStoreBase=GRADLE_USER_HOME
zipStorePath=wrapper/dists
distributionSha256Sum=abc123
";
        let props = parse_properties(content).unwrap();
        assert_eq!(props.distribution_url, "https://services.gradle.org/distributions/gradle-9.5-bin.zip");
        assert_eq!(props.distribution_sha256_sum.as_deref(), Some("abc123"));
        assert_eq!(props.network_timeout, 10000);
        assert!(props.validate_distribution_url);
    }

    #[test]
    fn test_parse_properties_defaults() {
        let content = "distributionUrl=https://example.com/gradle-8.0-bin.zip\n";
        let props = parse_properties(content).unwrap();
        assert_eq!(props.distribution_base, "GRADLE_USER_HOME");
        assert_eq!(props.distribution_path, "wrapper/dists");
        assert_eq!(props.zip_store_base, "GRADLE_USER_HOME");
        assert_eq!(props.zip_store_path, "wrapper/dists");
        assert_eq!(props.distribution_sha256_sum, None);
        assert_eq!(props.network_timeout, 10000);
    }

    #[test]
    fn test_parse_properties_missing_distribution_url() {
        let content = "networkTimeout=5000\n";
        let result = parse_properties(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_unescape_colon() {
        assert_eq!(unescape("https\\://example.com"), "https://example.com");
    }

    #[test]
    fn test_extract_version() {
        assert_eq!(
            extract_version_from_url("https://services.gradle.org/distributions/gradle-9.5-bin.zip"),
            "9.5"
        );
        assert_eq!(
            extract_version_from_url("https://services.gradle.org/distributions/gradle-8.0-milestone-1-all.zip"),
            "8.0-milestone-1"
        );
    }

    #[test]
    fn test_url_hash_deterministic() {
        let url = "https://services.gradle.org/distributions/gradle-9.5-bin.zip";
        let h1 = url_hash(url);
        let h2 = url_hash(url);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 16); // 8 bytes = 16 hex chars
    }

    #[test]
    fn test_url_hash_different_urls() {
        let h1 = url_hash("https://example.com/gradle-8.0-bin.zip");
        let h2 = url_hash("https://example.com/gradle-9.0-bin.zip");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_build_paths() {
        let props = WrapperProperties {
            distribution_url: "https://services.gradle.org/distributions/gradle-9.5-bin.zip".to_string(),
            ..Default::default()
        };
        let paths = build_paths(&props);
        assert!(paths.dist_dir.to_string_lossy().contains("9.5"));
        assert!(paths.dist_dir.to_string_lossy().contains(".gradle"));
        assert!(paths.zip_path.to_string_lossy().ends_with("9.5-bin.zip"));
        assert!(paths.marker_path.to_string_lossy().ends_with(".ok"));
    }

    #[test]
    fn test_verify_sha256() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.bin");
        std::fs::write(&file_path, b"hello").unwrap();
        // SHA-256 of "hello"
        let expected = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
        assert!(verify_sha256(&file_path, expected).is_ok());
    }

    #[test]
    fn test_verify_sha256_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.bin");
        std::fs::write(&file_path, b"hello").unwrap();
        assert!(verify_sha256(&file_path, "0000000000000000000000000000000000000000000000000000000000000000").is_err());
    }

    #[test]
    fn test_resolve_base_gradle_user_home() {
        let path = resolve_base("GRADLE_USER_HOME");
        assert!(path.to_string_lossy().contains(".gradle"));
    }

    #[test]
    fn test_resolve_base_project() {
        let path = resolve_base("PROJECT");
        assert_eq!(path, PathBuf::from("."));
    }
}
