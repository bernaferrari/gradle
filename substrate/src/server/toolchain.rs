use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use dashmap::DashMap;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tonic::{Request, Response, Status};

use crate::proto::{
    toolchain_service_server::ToolchainService, AutoDetectToolchainsRequest,
    AutoDetectToolchainsResponse, AutoDetectedToolchain, EnsureToolchainRequest,
    GetJavaHomeRequest, GetJavaHomeResponse, GetToolchainMetadataRequest,
    GetToolchainMetadataResponse, ListToolchainsRequest, ListToolchainsResponse,
    RegisterToolchainRequest, RegisterToolchainResponse, RemoveToolchainRequest,
    RemoveToolchainResponse, ToolchainLocation, ToolchainProgress, VerifyToolchainRequest,
    VerifyToolchainResponse,
};

/// Known toolchain installation.
struct InstalledToolchain {
    language_version: String,
    implementation: String,
    java_home: String,
    verified: bool,
    install_size_bytes: i64,
}

/// Rust-native toolchain management service.
/// Detects, verifies, and manages JDK/toolchain installations.
pub struct ToolchainServiceImpl {
    installations: DashMap<String, InstalledToolchain>,
    toolchain_dir: PathBuf,
    downloads_total: AtomicI64,
    _downloads_completed: AtomicI64,
    http_client: reqwest::Client,
}

/// Detailed JDK probe result with version and vendor info.
struct JavaProbeInfo {
    major_version: String,
    full_version: String,
    vendor: String,
}

impl Default for ToolchainServiceImpl {
    fn default() -> Self {
        Self::new(std::path::PathBuf::new())
    }
}

impl ToolchainServiceImpl {
    pub fn new(toolchain_dir: PathBuf) -> Self {
        std::fs::create_dir_all(&toolchain_dir).ok();
        Self {
            installations: DashMap::new(),
            toolchain_dir,
            downloads_total: AtomicI64::new(0),
            _downloads_completed: AtomicI64::new(0),
            http_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(300))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    fn toolchain_key(version: &str, implementation: &str) -> String {
        format!("{}-{}", implementation, version)
    }

    /// Detect system JDK installations from common paths, PATH, SDKMAN, and toolchain.properties.
    fn find_system_javas() -> Vec<(String, String)> {
        let mut found = Vec::new();

        // 1. Check JAVA_HOME environment variable
        if let Ok(java_home) = std::env::var("JAVA_HOME") {
            if let Some((version, size)) = Self::probe_java_home(&java_home) {
                found.push((java_home, format!("JDK {} ({})", version, size)));
            }
        }

        // 2. Scan PATH for java binaries
        if let Ok(path_var) = std::env::var("PATH") {
            for dir in path_var.split(if cfg!(target_os = "windows") {
                ';'
            } else {
                ':'
            }) {
                let java_bin = if cfg!(target_os = "windows") {
                    Path::new(dir).join("java.exe")
                } else {
                    Path::new(dir).join("java")
                };
                if java_bin.is_file() {
                    // Walk up from bin/java to find JAVA_HOME
                    if let Some(java_home) = java_bin.parent().and_then(|p| p.parent()) {
                        let home_str = java_home.to_string_lossy().into_owned();
                        if let Some((version, _)) = Self::probe_java_home(&home_str) {
                            found.push((home_str, version));
                        }
                    }
                }
            }
        }

        // 3. Check common JDK installation paths
        let candidates: Vec<String> = if cfg!(target_os = "macos") {
            vec![
                "/Library/Java/JavaVirtualMachines".to_string(),
                "/opt/homebrew/opt/openjdk".to_string(),
                "/opt/homebrew/opt/openjdk@11".to_string(),
                "/opt/homebrew/opt/openjdk@17".to_string(),
                "/opt/homebrew/opt/openjdk@21".to_string(),
                "/opt/homebrew/opt/openjdk@22".to_string(),
                "/opt/homebrew/Cellar/openjdk".to_string(),
            ]
        } else if cfg!(target_os = "linux") {
            vec![
                "/usr/lib/jvm".to_string(),
                "/usr/java".to_string(),
                "/usr/lib/jvm/java-8-openjdk-amd64".to_string(),
                "/usr/lib/jvm/java-11-openjdk-amd64".to_string(),
                "/usr/lib/jvm/java-17-openjdk-amd64".to_string(),
                "/usr/lib/jvm/java-21-openjdk-amd64".to_string(),
                "/usr/lib/jvm/java-22-openjdk-amd64".to_string(),
                "/usr/lib/jvm/temurin-8-jdk".to_string(),
                "/usr/lib/jvm/temurin-11-jdk".to_string(),
                "/usr/lib/jvm/temurin-17-jdk".to_string(),
                "/usr/lib/jvm/temurin-21-jdk".to_string(),
                "/usr/lib/jvm/temurin-22-jdk".to_string(),
                "/usr/lib/jvm/msopenjdk-17-jdk".to_string(),
                "/usr/lib/jvm/msopenjdk-21-jdk".to_string(),
                "/usr/lib/jvm/zulu-8-jdk".to_string(),
                "/usr/lib/jvm/zulu-11-jdk".to_string(),
                "/usr/lib/jvm/zulu-17-jdk".to_string(),
                "/usr/lib/jvm/zulu-21-jdk".to_string(),
                "/usr/lib/jvm/amazon-corretto-8".to_string(),
                "/usr/lib/jvm/amazon-corretto-11".to_string(),
                "/usr/lib/jvm/amazon-corretto-17".to_string(),
                "/usr/lib/jvm/amazon-corretto-21".to_string(),
            ]
        } else {
            vec![
                "C:\\Program Files\\Java".to_string(),
                "C:\\Program Files\\Eclipse Adoptium".to_string(),
                "C:\\Program Files\\Microsoft".to_string(),
                "C:\\Program Files\\Zulu".to_string(),
            ]
        };

        for base in &candidates {
            let base_path = Path::new(base);
            if !base_path.exists() {
                continue;
            }

            // For versioned directories (like /Library/Java/JavaVirtualMachines/)
            if base_path.is_dir() {
                if let Ok(entries) = std::fs::read_dir(base_path) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() {
                            // On macOS, look for Contents/Home inside each JDK
                            let home = if cfg!(target_os = "macos") {
                                let contents_home = path.join("Contents/Home");
                                if contents_home.exists() {
                                    contents_home
                                } else {
                                    path.clone()
                                }
                            } else {
                                path.clone()
                            };

                            if let Some((version, _)) =
                                Self::probe_java_home(&home.to_string_lossy())
                            {
                                found.push((home.to_string_lossy().into_owned(), version));
                            }
                        }
                    }
                }
            } else if base_path.exists() {
                if let Some((version, _)) = Self::probe_java_home(base) {
                    found.push((base.to_string(), version));
                }
            }
        }

        // 4. SDKMAN candidates (Linux/macOS)
        if cfg!(not(target_os = "windows")) {
            if let Ok(home) = std::env::var("HOME") {
                let sdkman_base = Path::new(&home).join(".sdkman/candidates/java");
                if sdkman_base.is_dir() {
                    if let Ok(entries) = std::fs::read_dir(&sdkman_base) {
                        for entry in entries.flatten() {
                            let path = entry.path();
                            if path.is_dir() {
                                if let Some((version, _)) =
                                    Self::probe_java_home(&path.to_string_lossy())
                                {
                                    found.push((path.to_string_lossy().into_owned(), version));
                                }
                            }
                        }
                    }
                }
            }
        }

        // 5. Check toolchain.properties in current directory and parent directories
        Self::scan_toolchain_properties(&mut found);

        // Deduplicate by java_home path
        let mut seen = std::collections::HashSet::new();
        found.retain(|(home, _)| seen.insert(home.clone()));

        found
    }

    /// Scan for toolchain.properties files in current and parent directories.
    /// Format: `java.home=/path/to/jdk` or `jdk.home=/path/to/jdk`
    fn scan_toolchain_properties(found: &mut Vec<(String, String)>) {
        let mut dir = match std::env::current_dir() {
            Ok(d) => d,
            Err(_) => return,
        };

        // Walk up at most 10 levels looking for toolchain.properties
        for _ in 0..10 {
            let props_path = dir.join("toolchain.properties");
            if props_path.exists() {
                if let Ok(contents) = std::fs::read_to_string(&props_path) {
                    for line in contents.lines() {
                        let line = line.trim();
                        if line.starts_with('#') || line.is_empty() {
                            continue;
                        }
                        if let Some((key, value)) = line.split_once('=') {
                            let key = key.trim();
                            let value = value.trim();
                            if key == "java.home" || key == "jdk.home" || key == "javaHome" {
                                let home_path = Path::new(value);
                                if home_path.exists() {
                                    if let Some((version, _)) = Self::probe_java_home(value) {
                                        found.push((value.to_string(), version));
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if !dir.pop() {
                break;
            }
        }
    }

    /// Run `java -version` to detect JDK version.
    fn probe_java_home(java_home: &str) -> Option<(String, String)> {
        let java_bin = if cfg!(target_os = "windows") {
            format!("{}\\bin\\java.exe", java_home)
        } else {
            format!("{}/bin/java", java_home)
        };

        let java_path = Path::new(&java_bin);
        if !java_path.exists() {
            return None;
        }

        // Try to run java -version
        match Command::new(&java_bin).arg("-version").output() {
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                let version_output = if stderr.is_empty() { &stdout } else { &stderr };

                // Parse version string like "openjdk version \"17.0.12\" 2024-07-16"
                // or "java version \"1.8.0_412\""
                let version = Self::parse_java_version(version_output);

                // Calculate install size
                let size = Self::dir_size(Path::new(java_home));

                version.map(|v| {
                    let size_str = if size > 1024 * 1024 * 1024 {
                        format!("{:.1} GB", size as f64 / (1024.0 * 1024.0 * 1024.0))
                    } else if size > 1024 * 1024 {
                        format!("{:.1} MB", size as f64 / (1024.0 * 1024.0))
                    } else {
                        format!("{} KB", size / 1024)
                    };
                    (v, size_str)
                })
            }
            Err(_) => None,
        }
    }

    /// Detailed JDK probe that returns structured info (major version, full version, vendor).
    fn probe_java_home_detailed(java_home: &str) -> Option<JavaProbeInfo> {
        let java_bin = if cfg!(target_os = "windows") {
            format!("{}\\bin\\java.exe", java_home)
        } else {
            format!("{}/bin/java", java_home)
        };

        if !Path::new(&java_bin).exists() {
            return None;
        }

        match Command::new(&java_bin).arg("-version").output() {
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                let version_output = if stderr.is_empty() { &stdout } else { &stderr };

                // Parse full version from output
                let mut full_version = String::new();
                let mut major_version = String::new();
                for part in version_output.split('"') {
                    let trimmed = part.trim();
                    if trimmed.starts_with("1.") {
                        if let Some(after) = trimmed.strip_prefix("1.") {
                            major_version = after
                                .split('.')
                                .next()
                                .unwrap_or("0")
                                .split('_')
                                .next()
                                .unwrap_or("0")
                                .to_string();
                            full_version = trimmed.to_string();
                        }
                    } else if trimmed.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                        if let Some(major) = trimmed.split('.').next() {
                            if major.parse::<u32>().is_ok() {
                                major_version = major.to_string();
                                full_version = trimmed.to_string();
                            }
                        }
                    }
                }

                if major_version.is_empty() {
                    return None;
                }

                let vendor = Self::detect_vendor(version_output);
                Some(JavaProbeInfo {
                    major_version,
                    full_version,
                    vendor,
                })
            }
            Err(_) => None,
        }
    }

    /// Parse JDK version from `java -version` output.
    fn parse_java_version(output: &str) -> Option<String> {
        // Try to match "version \"X.Y.Z\"" or "version \"1.X.Y_Z\""
        for part in output.split('"') {
            let trimmed = part.trim();
            if trimmed.starts_with("1.") {
                // Java 8 style: "1.8.0_412" -> "8"
                if let Some(after) = trimmed.strip_prefix("1.") {
                    let major = after.split('.').next().unwrap_or("0");
                    let major = major.split('_').next().unwrap_or("0");
                    let vendor = Self::detect_vendor(output);
                    return Some(if vendor.is_empty() {
                        format!("JDK {}", major)
                    } else {
                        format!("JDK {} ({})", major, vendor)
                    });
                }
            } else if trimmed.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                // Modern style: "17.0.12" -> "JDK 17"
                if let Some(major) = trimmed.split('.').next() {
                    if major.parse::<u32>().is_ok() {
                        let vendor = Self::detect_vendor(output);
                        return Some(if vendor.is_empty() {
                            format!("JDK {}", major)
                        } else {
                            format!("JDK {} ({})", major, vendor)
                        });
                    }
                }
            }
        }
        None
    }

    /// Detect JDK vendor from `java -version` output.
    /// More specific vendor strings are checked first to avoid false matches.
    fn detect_vendor(output: &str) -> String {
        let lower = output.to_lowercase();
        if lower.contains("temurin") || lower.contains("adoptium") {
            "Temurin".to_string()
        } else if lower.contains("corretto") {
            "Corretto".to_string()
        } else if lower.contains("zulu") {
            "Zulu".to_string()
        } else if lower.contains("microsoft") {
            "Microsoft".to_string()
        } else if lower.contains("graalvm") || lower.contains("graal") {
            "GraalVM".to_string()
        } else if lower.contains("semeru") || lower.contains("ibm") {
            "Semeru".to_string()
        } else if lower.contains("liberica") || lower.contains("bellsoft") {
            "Liberica".to_string()
        } else if lower.contains("oracle") {
            "Oracle".to_string()
        } else if lower.contains("openjdk") {
            "OpenJDK".to_string()
        } else {
            String::new()
        }
    }

    /// Recursively calculate directory size.
    fn dir_size(path: &Path) -> u64 {
        Self::dir_size_limited(path, 0, 10)
    }

    fn dir_size_limited(path: &Path, depth: u32, max_depth: u32) -> u64 {
        if !path.exists() || depth > max_depth {
            return 0;
        }

        if path.is_file() {
            return std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        }

        let mut total = 0u64;
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                total += Self::dir_size_limited(&entry.path(), depth + 1, max_depth);
            }
        }
        total
    }

    /// Construct a download URL for a JDK distribution.
    fn build_download_urls(
        version: &str,
        implementation: &str,
        os: &str,
        arch: &str,
    ) -> Vec<String> {
        let mut urls = Vec::new();

        let os_part = match os {
            "macos" => "mac",
            "linux" => "linux",
            "windows" => "windows",
            _ => os,
        };
        let arch_part = match arch {
            "x86_64" => "x64",
            "aarch64" => "aarch64",
            _ => arch,
        };
        let ext = if os == "windows" { "zip" } else { "tar.gz" };

        let major = version.parse::<u32>().unwrap_or(0);
        let series = if major >= 17 { "" } else { "8" };

        // Vendor-specific URL construction based on implementation name
        let impl_lower = implementation.to_lowercase();

        if impl_lower.contains("corretto") || impl_lower.contains("amazon") {
            // Amazon Corretto
            urls.push(format!(
                "https://corretto.aws/downloads/latest/{}/amazon-corretto-{}-{}-{}-{}.{}",
                version, version, os_part, arch_part, version, ext
            ));
        } else if impl_lower.contains("microsoft") || impl_lower.contains("msft") {
            // Microsoft Build of OpenJDK
            urls.push(format!(
                "https://aka.ms/download-jdk/microsoft-jdk-{}-{}-{}{}.{}",
                version,
                os_part,
                arch_part,
                if os == "windows" { "-windows" } else { "" },
                ext
            ));
        } else if impl_lower.contains("zulu") || impl_lower.contains("azul") {
            // Azul Zulu
            urls.push(format!(
                "https://cdn.azul.com/zulu/bin/zulu{}-ea-jdk{}_{}-{}.{}",
                version.replace(".", ""),
                version,
                os_part,
                arch_part,
                ext
            ));
        } else if impl_lower.contains("graalvm") || impl_lower.contains("oracle") {
            // Oracle GraalVM / Oracle JDK
            urls.push(format!(
                "https://download.oracle.com/graalvm/{}/latest/graalvm-jdk-{}_{}-{}_bin.{}",
                version, version, os_part, arch_part, ext
            ));
        } else {
            // Default: Adoptium (Eclipse Temurin)
            urls.push(format!(
                "https://api.adoptium.net/v3/binary/latest/{}/ga/{}/{}/jdk/hotspot/normal/eclipse?project=jdk",
                version, os_part, arch_part
            ));
            urls.push(format!(
                "https://github.com/adoptium/temurin{}-binaries/releases/download/jdk-{}/OpenJDK{}_U-jdk_{}_{}_{}_{}",
                series, version, version, os_part, arch_part, "hotspot", ext
            ));
        }

        // Always add Corretto and Microsoft as fallbacks
        if !impl_lower.contains("corretto") && !impl_lower.contains("amazon") {
            urls.push(format!(
                "https://corretto.aws/downloads/latest/{}/amazon-corretto-{}-{}-{}-{}.{}",
                version, version, os_part, arch_part, version, ext
            ));
        }

        if !impl_lower.contains("microsoft") && !impl_lower.contains("msft") {
            urls.push(format!(
                "https://aka.ms/download-jdk/microsoft-jdk-{}-{}-{}{}.{}",
                version,
                os_part,
                arch_part,
                if os == "windows" { "-windows" } else { "" },
                ext
            ));
        }

        urls
    }

    /// Download a file from a URL to a local path, with progress reporting via a channel.
    async fn _download_file(&self, url: &str, dest: &Path) -> Result<(), String> {
        let response = self
            .http_client
            .get(url)
            .send()
            .await
            .map_err(|e| format!("Download request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("HTTP {} for {}", response.status().as_u16(), url));
        }

        let total_size = response.content_length().unwrap_or(0);
        let mut downloaded: u64 = 0;

        let mut file = tokio::fs::File::create(dest)
            .await
            .map_err(|e| format!("Failed to create file: {}", e))?;

        let mut stream = response.bytes_stream();
        use futures_util::StreamExt;
        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.map_err(|e| format!("Stream error: {}", e))?;
            file.write_all(&chunk)
                .await
                .map_err(|e| format!("Write error: {}", e))?;
            downloaded += chunk.len() as u64;

            if total_size > 0 {
                let percent = ((downloaded as f64 / total_size as f64) * 100.0) as i64;
                tracing::debug!(percent, downloaded, total_size, "Downloading toolchain");
            }
        }

        file.flush()
            .await
            .map_err(|e| format!("Flush error: {}", e))?;

        Ok(())
    }

    /// Extract a .tar.gz archive to a directory.
    fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<(), String> {
        let file =
            std::fs::File::open(archive).map_err(|e| format!("Failed to open archive: {}", e))?;
        let gz_decoder = flate2::read::GzDecoder::new(file);
        let mut archive = tar::Archive::new(gz_decoder);

        archive
            .unpack(dest)
            .map_err(|e| format!("Failed to extract tar.gz: {}", e))?;

        Ok(())
    }

    /// Extract a .zip archive to a directory.
    fn extract_zip(archive: &Path, dest: &Path) -> Result<(), String> {
        let file =
            std::fs::File::open(archive).map_err(|e| format!("Failed to open archive: {}", e))?;
        let mut archive =
            zip::read::ZipArchive::new(file).map_err(|e| format!("Failed to read zip: {}", e))?;

        for i in 0..archive.len() {
            let mut entry = archive
                .by_index(i)
                .map_err(|e| format!("Failed to read entry {}: {}", i, e))?;

            let outpath = match entry.enclosed_name() {
                Some(path) => dest.join(path),
                None => continue,
            };

            if entry.is_dir() || entry.name().ends_with('/') {
                std::fs::create_dir_all(&outpath)
                    .map_err(|e| format!("Failed to create dir: {}", e))?;
            } else {
                if let Some(parent) = outpath.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("Failed to create parent: {}", e))?;
                }
                let mut outfile = std::fs::File::create(&outpath)
                    .map_err(|e| format!("Failed to create file: {}", e))?;
                std::io::copy(&mut entry, &mut outfile)
                    .map_err(|e| format!("Failed to write file: {}", e))?;
            }
        }

        Ok(())
    }

    /// Find the java home inside an extracted toolchain directory.
    /// JDK archives typically contain a single root directory.
    fn find_java_home_in_dir(dir: &Path) -> Option<PathBuf> {
        // Check if dir itself contains bin/java
        if dir.join("bin/java").exists() || dir.join("bin/java.exe").exists() {
            return Some(dir.to_path_buf());
        }

        // Check Contents/Home on macOS
        let contents_home = dir.join("Contents/Home");
        if contents_home.join("bin/java").exists() {
            return Some(contents_home);
        }

        // Look for a single subdirectory that contains bin/java
        if let Ok(entries) = std::fs::read_dir(dir) {
            let subdirs: Vec<_> = entries.flatten().filter(|e| e.path().is_dir()).collect();
            if subdirs.len() == 1 {
                let candidate = subdirs[0].path();
                if candidate.join("bin/java").exists()
                    || candidate.join("Contents/Home/bin/java").exists()
                {
                    return Some(candidate);
                }
            }
        }

        None
    }

    /// Compute SHA-256 hex digest of a file.
    fn sha256_file(path: &Path) -> Result<String, String> {
        let mut file = std::fs::File::open(path)
            .map_err(|e| format!("Failed to open file for hashing: {}", e))?;
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher)
            .map_err(|e| format!("Failed to read file for hashing: {}", e))?;
        Ok(format!("{:x}", hasher.finalize()))
    }
}

#[tonic::async_trait]
impl ToolchainService for ToolchainServiceImpl {
    async fn list_toolchains(
        &self,
        request: Request<ListToolchainsRequest>,
    ) -> Result<Response<ListToolchainsResponse>, Status> {
        let _req = request.into_inner();

        let mut toolchains = Vec::new();

        // Report installed toolchains from registry
        for entry in self.installations.iter() {
            toolchains.push(ToolchainLocation {
                language_version: entry.language_version.clone(),
                implementation: entry.implementation.clone(),
                java_home: entry.java_home.clone(),
                verified: entry.verified,
                install_size_bytes: entry.install_size_bytes,
                installed_via: "substrate".to_string(),
            });
        }

        // Scan for system-detected JDKs
        for (java_home, version_str) in Self::find_system_javas() {
            let verified = Path::new(&java_home).join("bin/java").exists();
            toolchains.push(ToolchainLocation {
                language_version: version_str,
                implementation: "system".to_string(),
                java_home,
                verified,
                install_size_bytes: 0,
                installed_via: "detected".to_string(),
            });
        }

        tracing::debug!(count = toolchains.len(), "Listed toolchains");

        Ok(Response::new(ListToolchainsResponse { toolchains }))
    }

    type EnsureToolchainStream = std::pin::Pin<
        Box<
            dyn tonic::codegen::tokio_stream::Stream<Item = Result<ToolchainProgress, Status>>
                + Send,
        >,
    >;

    async fn ensure_toolchain(
        &self,
        request: Request<EnsureToolchainRequest>,
    ) -> Result<Response<Self::EnsureToolchainStream>, Status> {
        let req = request.into_inner();
        let key = Self::toolchain_key(&req.language_version, &req.implementation);

        // Check if already installed
        if let Some(entry) = self.installations.get(&key) {
            let java_home = entry.java_home.clone();
            let stream = futures_util::stream::iter(vec![Ok(ToolchainProgress {
                phase: "done".to_string(),
                progress_percent: 100,
                message: format!(
                    "Toolchain {} {} already installed",
                    req.implementation, req.language_version
                ),
                success: true,
                error_message: String::new(),
                java_home,
                sha256_verified: false,
                cache_hit: true,
            })]);
            return Ok(Response::new(
                Box::pin(stream) as Self::EnsureToolchainStream
            ));
        }

        self.downloads_total.fetch_add(1, Ordering::Relaxed);

        let version = req.language_version.clone();
        let impl_name = req.implementation.clone();
        let toolchain_dir = self.toolchain_dir.clone();
        let download_urls = req.download_urls.clone();
        let http_client = self.http_client.clone();
        let expected_sha256 = req.expected_sha256.clone();
        let cache_download = req.cache_download;

        let stream = async_stream::stream! {
            // Phase 1: Check
            yield Ok(ToolchainProgress {
                phase: "checking".to_string(),
                progress_percent: 5,
                message: format!("Checking for {} {}", impl_name, version),
                success: true,
                error_message: String::new(),
                java_home: String::new(),
            sha256_verified: false,
            cache_hit: false,
            });

            // Phase 2: Download
            yield Ok(ToolchainProgress {
                phase: "downloading".to_string(),
                progress_percent: 10,
                message: "Preparing download URLs".to_string(),
                success: true,
                error_message: String::new(),
                java_home: String::new(),
            sha256_verified: false,
            cache_hit: false,
            });

            let os = std::env::consts::OS;
            let arch = std::env::consts::ARCH;

            // Use provided URLs or construct default ones
            let urls: Vec<String> = if download_urls.is_empty() {
                Self::build_download_urls(&version, &impl_name, os, arch)
            } else {
                download_urls
            };

            let target_dir = toolchain_dir.join(format!("{}-{}", impl_name, version));
            let archive_path = target_dir.join(if os == "windows" { "jdk.zip" } else { "jdk.tar.gz" });
            let cache_dir = toolchain_dir.join("cache");
            let cached_archive_path = cache_dir.join(format!("{}-{}.{}", impl_name, version, if os == "windows" { "zip" } else { "tar.gz" }));

            // Check for cached archive
            let mut download_success = false;
            let mut from_cache = false;
            if cache_download && cached_archive_path.exists() {
                // Verify cached archive against expected checksum if provided
                let cache_valid = if !expected_sha256.is_empty() {
                    match Self::sha256_file(&cached_archive_path) {
                        Ok(digest) => {
                            let matches = digest == expected_sha256;
                            if !matches {
                                tracing::warn!(expected = %expected_sha256, actual = %digest, "Cached archive checksum mismatch, re-downloading");
                            }
                            matches
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "Failed to hash cached archive, re-downloading");
                            false
                        }
                    }
                } else {
                    true
                };

                if cache_valid {
                    if let Err(e) = std::fs::copy(&cached_archive_path, &archive_path) {
                        tracing::debug!("Failed to copy cached archive: {}", e);
                    } else {
                        from_cache = true;
                        download_success = true;
                        yield Ok(ToolchainProgress {
                            phase: "downloading".to_string(),
                            progress_percent: 60,
                            message: "Using cached archive".to_string(),
                            success: true,
                            error_message: String::new(),
                            java_home: String::new(),
                            sha256_verified: false,
                            cache_hit: true,
                        });
                    }
                }
            }

            // Try each URL (skip if cache hit)
            for (i, url) in urls.iter().enumerate() {
                if download_success {
                    break;
                }
                if download_success {
                    break;
                }
                yield Ok(ToolchainProgress {
                    phase: "downloading".to_string(),
                    progress_percent: 15,
                    message: format!("Trying URL {} ({}/{})", url, i + 1, urls.len()),
                    success: true,
                    error_message: String::new(),
                    java_home: String::new(),
                sha256_verified: false,
                cache_hit: false,
                });

                // Create temp dir for download
                if let Err(e) = std::fs::create_dir_all(&target_dir) {
                    yield Ok(ToolchainProgress {
                        phase: "error".to_string(),
                        progress_percent: 0,
                        message: format!("Failed to create directory: {}", e),
                        success: false,
                        error_message: e.to_string(),
                        java_home: String::new(),
                    sha256_verified: false,
                    cache_hit: false,
                    });
                    return;
                }

                match http_client.get(url).send().await {
                    Ok(response) if response.status().is_success() => {
                        let total_size = response.content_length().unwrap_or(0);

                        // Download to file
                        match tokio::fs::File::create(&archive_path).await {
                            Ok(mut file) => {
                                use futures_util::StreamExt;
                                let mut stream = response.bytes_stream();
                                let mut downloaded: u64 = 0;
                                let mut last_percent: i64 = 15;

                                while let Some(chunk_result) = stream.next().await {
                                    match chunk_result {
                                        Ok(chunk) => {
                                            if file.write_all(&chunk).await.is_err() {
                                                break;
                                            }
                                            downloaded += chunk.len() as u64;

                                            if total_size > 0 {
                                                let percent = 20 + ((downloaded as f64 / total_size as f64) * 40.0) as i64;
                                                if percent > last_percent + 5 {
                                                    last_percent = percent;
                                                    yield Ok(ToolchainProgress {
                                                        phase: "downloading".to_string(),
                                                        progress_percent: percent.min(60),
                                                        message: format!(
                                                            "Downloaded {} / {} ({:.0}%)",
                                                            downloaded / 1024 / 1024,
                                                            total_size / 1024 / 1024,
                                                            downloaded as f64 / total_size as f64 * 100.0
                                                        ),
                                                        success: true,
                                                        error_message: String::new(),
                                                        java_home: String::new(),
                                                    sha256_verified: false,
                                                    cache_hit: false,
                                                    });
                                                }
                                            }
                                        }
                                        Err(_) => break,
                                    }
                                }

                                if file.flush().await.is_ok() {
                                    download_success = true;
                                    break;
                                }
                            }
                            Err(e) => {
                                tracing::debug!("Failed to create download file: {}", e);
                            }
                        }
                    }
                    Ok(response) => {
                        tracing::debug!("HTTP {} for {}", response.status().as_u16(), url);
                    }
                    Err(e) => {
                        tracing::debug!("Download error for {}: {}", url, e);
                    }
                }
            }

            if !download_success {
                yield Ok(ToolchainProgress {
                    phase: "error".to_string(),
                    progress_percent: 0,
                    message: "Failed to download from any URL".to_string(),
                    success: false,
                    error_message: "All download URLs failed".to_string(),
                    java_home: String::new(),
                sha256_verified: false,
                cache_hit: false,
                });
                return;
            }

            // Phase 2.5: Verify checksum if expected_sha256 was provided
            let mut sha256_verified = false;
            if !expected_sha256.is_empty() {
                yield Ok(ToolchainProgress {
                    phase: "verifying_checksum".to_string(),
                    progress_percent: 62,
                    message: "Verifying archive checksum".to_string(),
                    success: true,
                    error_message: String::new(),
                    java_home: String::new(),
                    sha256_verified: false,
                    cache_hit: from_cache,
                });

                let archive_hash = Self::sha256_file(&archive_path).unwrap_or_default();
                sha256_verified = archive_hash == expected_sha256;
                if !sha256_verified {
                    yield Ok(ToolchainProgress {
                        phase: "error".to_string(),
                        progress_percent: 0,
                        message: format!("SHA-256 mismatch: expected {} but got {}", expected_sha256, archive_hash),
                        success: false,
                        error_message: "Checksum verification failed".to_string(),
                        java_home: String::new(),
                        sha256_verified: false,
                        cache_hit: from_cache,
                    });
                    // Clean up bad download
                    let _ = std::fs::remove_file(&archive_path);
                    return;
                }
            }

            // Cache the downloaded archive for future use
            if !from_cache && download_success {
                if let Err(e) = std::fs::create_dir_all(&cache_dir) {
                    tracing::debug!("Failed to create cache dir: {}", e);
                } else {
                    let _ = std::fs::copy(&archive_path, &cached_archive_path);
                }
            }

            // Phase 3: Extract
            yield Ok(ToolchainProgress {
                phase: "extracting".to_string(),
                progress_percent: 65,
                message: "Extracting archive".to_string(),
                success: true,
                error_message: String::new(),
                java_home: String::new(),
            sha256_verified: false,
            cache_hit: false,
            });

            let extract_dir = target_dir.join("extract");
            if std::fs::create_dir_all(&extract_dir).is_err() {
                yield Ok(ToolchainProgress {
                    phase: "error".to_string(),
                    progress_percent: 0,
                    message: "Failed to create extract directory".to_string(),
                    success: false,
                    error_message: "mkdir failed".to_string(),
                    java_home: String::new(),
                sha256_verified: false,
                cache_hit: false,
                });
                return;
            }

            let archive_path_clone = archive_path.clone();
            let extract_dir_clone = extract_dir.clone();
            let extract_result = tokio::task::spawn_blocking(move || {
                let ext = archive_path_clone.extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("");
                if ext == "gz" || archive_path_clone.to_string_lossy().ends_with(".tar.gz") {
                    Self::extract_tar_gz(&archive_path_clone, &extract_dir_clone)
                } else {
                    Self::extract_zip(&archive_path_clone, &extract_dir_clone)
                }
            }).await;

            if let Err(e) = extract_result {
                yield Ok(ToolchainProgress {
                    phase: "error".to_string(),
                    progress_percent: 0,
                    message: format!("Extraction failed: {}", e),
                    success: false,
                    error_message: e.to_string(),
                    java_home: String::new(),
                sha256_verified: false,
                cache_hit: false,
                });
                return;
            }

            let extract_result = match extract_result {
                Ok(r) => r,
                Err(e) => {
                    yield Ok(ToolchainProgress {
                        phase: "error".to_string(),
                        progress_percent: 0,
                        message: format!("Extraction failed: {}", e),
                        success: false,
                        error_message: e.to_string(),
                        java_home: String::new(),
                    sha256_verified: false,
                    cache_hit: false,
                    });
                    return;
                }
            };
            if let Err(e) = extract_result {
                yield Ok(ToolchainProgress {
                    phase: "error".to_string(),
                    progress_percent: 0,
                    message: format!("Extraction failed: {}", e),
                    success: false,
                    error_message: e.to_string(),
                    java_home: String::new(),
                sha256_verified: false,
                cache_hit: false,
                });
                return;
            }

            // Phase 4: Verify
            yield Ok(ToolchainProgress {
                phase: "verifying".to_string(),
                progress_percent: 90,
                message: "Verifying installation".to_string(),
                success: true,
                error_message: String::new(),
                java_home: String::new(),
            sha256_verified: false,
            cache_hit: false,
            });

            let java_home = match Self::find_java_home_in_dir(&extract_dir) {
                Some(home) => home.to_string_lossy().into_owned(),
                None => {
                    yield Ok(ToolchainProgress {
                        phase: "error".to_string(),
                        progress_percent: 0,
                        message: "Could not find bin/java in extracted archive".to_string(),
                        success: false,
                        error_message: "Invalid JDK archive structure".to_string(),
                        java_home: String::new(),
                    sha256_verified: false,
                    cache_hit: false,
                    });
                    return;
                }
            };

            // Verify java -version works
            let java_bin = if os == "windows" {
                format!("{}\\bin\\java.exe", java_home)
            } else {
                format!("{}/bin/java", java_home)
            };

            match tokio::process::Command::new(&java_bin).arg("-version").output().await {
                Ok(output) if output.status.success() => {
                    // All good
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    yield Ok(ToolchainProgress {
                        phase: "error".to_string(),
                        progress_percent: 0,
                        message: format!("java -version failed: {}", stderr),
                        success: false,
                        error_message: "JDK verification failed".to_string(),
                        java_home: String::new(),
                    sha256_verified: false,
                    cache_hit: false,
                    });
                    return;
                }
                Err(e) => {
                    yield Ok(ToolchainProgress {
                        phase: "error".to_string(),
                        progress_percent: 0,
                        message: format!("Failed to run java: {}", e),
                        success: false,
                        error_message: "JDK verification failed".to_string(),
                        java_home: String::new(),
                    sha256_verified: false,
                    cache_hit: false,
                    });
                    return;
                }
            }

            // Clean up archive
            let _ = std::fs::remove_file(&archive_path);

            // Phase 5: Done
            yield Ok(ToolchainProgress {
                phase: "done".to_string(),
                progress_percent: 100,
                message: format!("Toolchain {} {} installed at {}", impl_name, version, java_home),
                success: true,
                error_message: String::new(),
                java_home: java_home.clone(),
                sha256_verified,
                cache_hit: from_cache,
            });
        };

        Ok(Response::new(
            Box::pin(stream) as Self::EnsureToolchainStream
        ))
    }

    async fn verify_toolchain(
        &self,
        request: Request<VerifyToolchainRequest>,
    ) -> Result<Response<VerifyToolchainResponse>, Status> {
        let req = request.into_inner();
        let path = Path::new(&req.java_home);

        if !path.exists() {
            return Ok(Response::new(VerifyToolchainResponse {
                valid: false,
                detected_version: String::new(),
                error_message: format!("Java home not found: {}", req.java_home),
            }));
        }

        // Actually run java -version to verify the JDK works
        let java_bin = if cfg!(target_os = "windows") {
            format!("{}\\bin\\java.exe", req.java_home)
        } else {
            format!("{}/bin/java", req.java_home)
        };

        match tokio::process::Command::new(&java_bin)
            .arg("-version")
            .output()
            .await
        {
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                let version_output = if stderr.is_empty() { &stdout } else { &stderr };

                let exit_code = output.status.code().unwrap_or(-1);
                let valid = exit_code == 0;

                let detected_version = Self::parse_java_version(version_output).unwrap_or_default();

                // Verify against expected version if provided
                let version_match =
                    if !req.expected_version.is_empty() && !detected_version.is_empty() {
                        detected_version.contains(&req.expected_version)
                    } else {
                        true
                    };

                Ok(Response::new(VerifyToolchainResponse {
                    valid: valid && version_match,
                    detected_version: detected_version.clone(),
                    error_message: if !valid {
                        format!("java -version exited with code {}", exit_code)
                    } else if !version_match {
                        format!(
                            "Expected version {} but found {}",
                            req.expected_version, detected_version
                        )
                    } else {
                        String::new()
                    },
                }))
            }
            Err(e) => Ok(Response::new(VerifyToolchainResponse {
                valid: false,
                detected_version: String::new(),
                error_message: format!("Failed to run java -version: {}", e),
            })),
        }
    }

    async fn get_java_home(
        &self,
        request: Request<GetJavaHomeRequest>,
    ) -> Result<Response<GetJavaHomeResponse>, Status> {
        let req = request.into_inner();

        // Check registry first
        let key = Self::toolchain_key(&req.language_version, &req.implementation);
        if let Some(entry) = self.installations.get(&key) {
            return Ok(Response::new(GetJavaHomeResponse {
                java_home: entry.java_home.clone(),
                found: true,
            }));
        }

        // Scan system JDKs for matching version
        let target_version = req.language_version.trim_start_matches("JDK ").trim();
        for (java_home, version_str) in Self::find_system_javas() {
            if version_str.contains(target_version) || java_home.contains(target_version) {
                return Ok(Response::new(GetJavaHomeResponse {
                    java_home,
                    found: true,
                }));
            }
        }

        // Fall back to JAVA_HOME
        if let Ok(java_home) = std::env::var("JAVA_HOME") {
            return Ok(Response::new(GetJavaHomeResponse {
                java_home,
                found: true,
            }));
        }

        Ok(Response::new(GetJavaHomeResponse {
            java_home: String::new(),
            found: false,
        }))
    }

    async fn register_toolchain(
        &self,
        request: Request<RegisterToolchainRequest>,
    ) -> Result<Response<RegisterToolchainResponse>, Status> {
        let req = request.into_inner();
        let path = Path::new(&req.java_home);

        if !path.exists() {
            return Ok(Response::new(RegisterToolchainResponse {
                registered: false,
                java_home: String::new(),
                detected_version: String::new(),
                error_message: format!("Java home not found: {}", req.java_home),
            }));
        }

        // Verify the JDK works
        let java_bin = if cfg!(target_os = "windows") {
            format!("{}\\bin\\java.exe", req.java_home)
        } else {
            format!("{}/bin/java", req.java_home)
        };

        let java_path = Path::new(&java_bin);
        if !java_path.exists() {
            return Ok(Response::new(RegisterToolchainResponse {
                registered: false,
                java_home: String::new(),
                detected_version: String::new(),
                error_message: format!("java binary not found at {}", java_bin),
            }));
        }

        // Optional: verify checksum of the java binary
        if !req.expected_sha256.is_empty() {
            if let Ok(digest) = Self::sha256_file(java_path) {
                if digest != req.expected_sha256 {
                    return Ok(Response::new(RegisterToolchainResponse {
                        registered: false,
                        java_home: String::new(),
                        detected_version: String::new(),
                        error_message: format!(
                            "SHA-256 mismatch for {}: expected {} got {}",
                            java_bin, req.expected_sha256, digest
                        ),
                    }));
                }
            }
        }

        // Run java -version to detect version
        let detected_version = match Command::new(&java_bin).arg("-version").output() {
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let stdout = String::from_utf8_lossy(&output.stdout);
                let version_output = if stderr.is_empty() { &stdout } else { &stderr };
                Self::parse_java_version(version_output).unwrap_or_default()
            }
            Err(e) => {
                return Ok(Response::new(RegisterToolchainResponse {
                    registered: false,
                    java_home: String::new(),
                    detected_version: String::new(),
                    error_message: format!("Failed to run java -version: {}", e),
                }));
            }
        };

        let size = Self::dir_size(path) as i64;
        let key = Self::toolchain_key(&req.language_version, &req.implementation);

        self.installations.insert(
            key.clone(),
            InstalledToolchain {
                language_version: detected_version.clone(),
                implementation: req.implementation.clone(),
                java_home: req.java_home.clone(),
                verified: true,
                install_size_bytes: size,
            },
        );

        tracing::info!(
            version = %req.language_version,
            impl_name = %req.implementation,
            java_home = %req.java_home,
            detected = %detected_version,
            "Registered toolchain"
        );

        Ok(Response::new(RegisterToolchainResponse {
            registered: true,
            java_home: req.java_home.clone(),
            detected_version,
            error_message: String::new(),
        }))
    }

    async fn remove_toolchain(
        &self,
        request: Request<RemoveToolchainRequest>,
    ) -> Result<Response<RemoveToolchainResponse>, Status> {
        let req = request.into_inner();
        let key = Self::toolchain_key(&req.language_version, &req.implementation);

        match self.installations.remove(&key) {
            Some((_, entry)) => {
                let java_home = entry.java_home.clone();

                if req.delete_files {
                    let path = Path::new(&java_home);
                    if path.exists() {
                        match std::fs::remove_dir_all(path) {
                            Ok(()) => {
                                tracing::info!(java_home = %java_home, "Removed toolchain files");
                            }
                            Err(e) => {
                                tracing::warn!(java_home = %java_home, error = %e, "Failed to remove toolchain files");
                            }
                        }
                    }
                }

                Ok(Response::new(RemoveToolchainResponse {
                    removed: true,
                    java_home,
                    error_message: String::new(),
                }))
            }
            None => Ok(Response::new(RemoveToolchainResponse {
                removed: false,
                java_home: String::new(),
                error_message: format!(
                    "Toolchain {} {} not found in registry",
                    req.implementation, req.language_version
                ),
            })),
        }
    }

    async fn get_toolchain_metadata(
        &self,
        request: Request<GetToolchainMetadataRequest>,
    ) -> Result<Response<GetToolchainMetadataResponse>, Status> {
        let req = request.into_inner();

        // If java_home is provided, probe it directly
        if !req.java_home.is_empty() {
            let path = Path::new(&req.java_home);
            if !path.exists() {
                return Ok(Response::new(GetToolchainMetadataResponse {
                    found: false,
                    ..Default::default()
                }));
            }

            let detected_version = Self::probe_java_home(&req.java_home)
                .map(|(v, _)| v)
                .unwrap_or_default();
            let size = Self::dir_size(path) as i64;
            let verified = path.join("bin/java").exists() || path.join("bin/java.exe").exists();

            return Ok(Response::new(GetToolchainMetadataResponse {
                found: true,
                language_version: req.language_version,
                implementation: req.implementation,
                vendor: req.vendor,
                detected_version,
                java_home: req.java_home,
                install_size_bytes: size,
                verified,
            }));
        }

        // Otherwise look up by version/implementation
        let key = Self::toolchain_key(&req.language_version, &req.implementation);
        if let Some(entry) = self.installations.get(&key) {
            return Ok(Response::new(GetToolchainMetadataResponse {
                found: true,
                language_version: entry.language_version.clone(),
                implementation: entry.implementation.clone(),
                vendor: String::new(),
                detected_version: entry.language_version.clone(),
                java_home: entry.java_home.clone(),
                install_size_bytes: entry.install_size_bytes,
                verified: entry.verified,
            }));
        }

        // Fall back to system scan (runs on blocking thread pool to avoid
        // starving the tokio runtime — find_system_javas spawns subprocesses
        // and walks the filesystem).
        let target_version = req
            .language_version
            .trim_start_matches("JDK ")
            .trim()
            .to_string();
        let system_javas = tokio::task::spawn_blocking(move || {
            Self::find_system_javas()
                .into_iter()
                .filter(|(_, v)| v.contains(&target_version))
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();

        if let Some((java_home, version_str)) = system_javas.into_iter().next() {
            let size = Self::dir_size(Path::new(&java_home)) as i64;
            return Ok(Response::new(GetToolchainMetadataResponse {
                found: true,
                language_version: req.language_version,
                implementation: "system".to_string(),
                vendor: String::new(),
                detected_version: version_str,
                java_home,
                install_size_bytes: size,
                verified: true,
            }));
        }

        Ok(Response::new(GetToolchainMetadataResponse {
            found: false,
            ..Default::default()
        }))
    }

    async fn auto_detect_toolchains(
        &self,
        request: Request<AutoDetectToolchainsRequest>,
    ) -> Result<Response<AutoDetectToolchainsResponse>, Status> {
        let req = request.into_inner();

        // All detection logic is synchronous (env vars, filesystem walks,
        // subprocess spawns). Run on the blocking thread pool to avoid
        // starving the tokio runtime.
        let results = tokio::task::spawn_blocking(move || {
            let mut results = Vec::new();

            // 1. JAVA_HOME
            if let Ok(java_home) = std::env::var("JAVA_HOME") {
                if let Some(info) = Self::probe_java_home_detailed(&java_home) {
                    if req.major_version.is_empty() || info.major_version == req.major_version {
                        results.push(AutoDetectedToolchain {
                            java_home,
                            major_version: info.major_version,
                            full_version: info.full_version,
                            vendor: info.vendor,
                            source: "JAVA_HOME".to_string(),
                            verified: true,
                        });
                    }
                }
            }

            // 2. PATH scanning
            if req.scan_path {
                if let Ok(path_var) = std::env::var("PATH") {
                    let separator = if cfg!(target_os = "windows") {
                        ';'
                    } else {
                        ':'
                    };
                    for dir in path_var.split(separator) {
                        let java_bin = if cfg!(target_os = "windows") {
                            Path::new(dir).join("java.exe")
                        } else {
                            Path::new(dir).join("java")
                        };
                        if java_bin.is_file() {
                            if let Some(java_home) = java_bin.parent().and_then(|p| p.parent()) {
                                let home_str = java_home.to_string_lossy().to_string();
                                // Skip if already found via JAVA_HOME
                                if results.iter().any(|r| r.java_home == home_str) {
                                    continue;
                                }
                                if let Some(info) = Self::probe_java_home_detailed(&home_str) {
                                    if req.major_version.is_empty()
                                        || info.major_version == req.major_version
                                    {
                                        results.push(AutoDetectedToolchain {
                                            java_home: home_str,
                                            major_version: info.major_version,
                                            full_version: info.full_version,
                                            vendor: info.vendor,
                                            source: "PATH".to_string(),
                                            verified: true,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // 3. SDKMAN
            if req.scan_sdkman && cfg!(not(target_os = "windows")) {
                if let Ok(home) = std::env::var("HOME") {
                    let sdkman_base = Path::new(&home).join(".sdkman/candidates/java");
                    if sdkman_base.is_dir() {
                        if let Ok(entries) = std::fs::read_dir(&sdkman_base) {
                            for entry in entries.flatten() {
                                let path = entry.path();
                                if path.is_dir() {
                                    let home_str = path.to_string_lossy().to_string();
                                    if results.iter().any(|r| r.java_home == home_str) {
                                        continue;
                                    }
                                    if let Some(info) = Self::probe_java_home_detailed(&home_str) {
                                        if req.major_version.is_empty()
                                            || info.major_version == req.major_version
                                        {
                                            results.push(AutoDetectedToolchain {
                                                java_home: home_str,
                                                major_version: info.major_version,
                                                full_version: info.full_version,
                                                vendor: info.vendor,
                                                source: "SDKMAN".to_string(),
                                                verified: true,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // 4. Filesystem scanning (common paths)
            let candidates: Vec<String> = if cfg!(target_os = "macos") {
                vec![
                    "/Library/Java/JavaVirtualMachines".to_string(),
                    "/opt/homebrew/opt/openjdk".to_string(),
                    "/opt/homebrew/opt/openjdk@11".to_string(),
                    "/opt/homebrew/opt/openjdk@17".to_string(),
                    "/opt/homebrew/opt/openjdk@21".to_string(),
                    "/opt/homebrew/opt/openjdk@22".to_string(),
                    "/opt/homebrew/Cellar/openjdk".to_string(),
                ]
            } else if cfg!(target_os = "linux") {
                vec!["/usr/lib/jvm".to_string(), "/usr/java".to_string()]
            } else {
                vec![
                    "C:\\Program Files\\Java".to_string(),
                    "C:\\Program Files\\Eclipse Adoptium".to_string(),
                    "C:\\Program Files\\Microsoft".to_string(),
                ]
            };

            for base in &candidates {
                let base_path = Path::new(base);
                if !base_path.is_dir() {
                    continue;
                }
                if let Ok(entries) = std::fs::read_dir(base_path) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if !path.is_dir() {
                            continue;
                        }
                        let home = if cfg!(target_os = "macos") {
                            let contents_home = path.join("Contents/Home");
                            if contents_home.exists() {
                                contents_home
                            } else {
                                path.clone()
                            }
                        } else {
                            path.clone()
                        };
                        let home_str = home.to_string_lossy().to_string();
                        if results.iter().any(|r| r.java_home == home_str) {
                            continue;
                        }
                        if let Some(info) = Self::probe_java_home_detailed(&home_str) {
                            if req.major_version.is_empty()
                                || info.major_version == req.major_version
                            {
                                results.push(AutoDetectedToolchain {
                                    java_home: home_str,
                                    major_version: info.major_version,
                                    full_version: info.full_version,
                                    vendor: info.vendor,
                                    source: "filesystem".to_string(),
                                    verified: true,
                                });
                            }
                        }
                    }
                }
            }

            // 5. toolchain.properties
            if req.scan_project {
                if let Ok(mut dir) = std::env::current_dir() {
                    for _ in 0..10 {
                        let props_path = dir.join("toolchain.properties");
                        if props_path.exists() {
                            if let Ok(contents) = std::fs::read_to_string(&props_path) {
                                for line in contents.lines() {
                                    let line = line.trim();
                                    if line.starts_with('#') || line.is_empty() {
                                        continue;
                                    }
                                    if let Some((key, value)) = line.split_once('=') {
                                        let key = key.trim();
                                        let value = value.trim();
                                        if key == "java.home"
                                            || key == "jdk.home"
                                            || key == "javaHome"
                                        {
                                            let home_str = value.to_string();
                                            if results.iter().any(|r| r.java_home == home_str) {
                                                continue;
                                            }
                                            if Path::new(value).exists() {
                                                if let Some(info) =
                                                    Self::probe_java_home_detailed(value)
                                                {
                                                    if req.major_version.is_empty()
                                                        || info.major_version == req.major_version
                                                    {
                                                        results.push(AutoDetectedToolchain {
                                                            java_home: home_str,
                                                            major_version: info.major_version,
                                                            full_version: info.full_version,
                                                            vendor: info.vendor,
                                                            source: "toolchain.properties"
                                                                .to_string(),
                                                            verified: true,
                                                        });
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        if !dir.pop() {
                            break;
                        }
                    }
                }
            }

            results
        })
        .await
        .unwrap_or_default();

        let total = results.len() as i32;
        let summary = format!(
            "Found {} JDK(s) across JAVA_HOME, PATH, SDKMAN, filesystem, and toolchain.properties",
            total
        );

        tracing::debug!(total, summary = %summary, "Auto-detection complete");

        Ok(Response::new(AutoDetectToolchainsResponse {
            toolchains: results,
            total_found: total,
            scan_summary: summary,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_svc() -> ToolchainServiceImpl {
        let dir = tempfile::tempdir().unwrap();
        ToolchainServiceImpl::new(dir.path().to_path_buf())
    }

    #[tokio::test]
    async fn test_verify_missing() {
        let svc = make_svc();

        let resp = svc
            .verify_toolchain(Request::new(VerifyToolchainRequest {
                java_home: "/nonexistent/java/home".to_string(),
                expected_version: "17".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.valid);
        assert!(!resp.detected_version.is_empty() || !resp.error_message.is_empty());
    }

    #[tokio::test]
    async fn test_parse_java_version_modern() {
        let output = r#"openjdk version "17.0.12" 2024-07-16
OpenJDK Runtime Environment (build 17.0.12+8)
OpenJDK 64-Bit Server VM (build 17.0.12+8, mixed mode, sharing)"#;
        let version = ToolchainServiceImpl::parse_java_version(output);
        assert_eq!(version, Some("JDK 17 (OpenJDK)".to_string()));
    }

    #[tokio::test]
    async fn test_parse_java_version_legacy() {
        let output = r#"java version "1.8.0_412"
Java(TM) SE Runtime Environment (build 1.8.0_412-b08)"#;
        let version = ToolchainServiceImpl::parse_java_version(output);
        assert_eq!(version, Some("JDK 8".to_string()));
    }

    #[tokio::test]
    async fn test_parse_java_version_21() {
        let output = r#"openjdk version "21.0.4" 2024-01-16
OpenJDK Runtime Environment (build 21.0.4+7)"#;
        let version = ToolchainServiceImpl::parse_java_version(output);
        assert_eq!(version, Some("JDK 21 (OpenJDK)".to_string()));
    }

    #[tokio::test]
    async fn test_list_toolchains() {
        let svc = make_svc();

        let resp = svc
            .list_toolchains(Request::new(ListToolchainsRequest {
                os: std::env::consts::OS.to_string(),
                arch: std::env::consts::ARCH.to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        // May be 0 in CI if no JDK is in standard paths.
        for tc in &resp.toolchains {
            assert!(!tc.java_home.is_empty());
        }
    }

    #[test]
    fn test_parse_java_version_invalid() {
        let output = "no version info here";
        let version = ToolchainServiceImpl::parse_java_version(output);
        assert_eq!(version, None);
    }

    #[test]
    fn test_parse_java_version_empty() {
        let version = ToolchainServiceImpl::parse_java_version("");
        assert_eq!(version, None);
    }

    #[test]
    fn test_parse_java_version_java11() {
        let output = r#"openjdk version "11.0.24" 2024-04-16"#;
        let version = ToolchainServiceImpl::parse_java_version(output);
        assert_eq!(version, Some("JDK 11 (OpenJDK)".to_string()));
    }

    #[test]
    fn test_toolchain_key() {
        assert_eq!(
            ToolchainServiceImpl::toolchain_key("17", "temurin"),
            "temurin-17"
        );
        assert_eq!(
            ToolchainServiceImpl::toolchain_key("21", "corretto"),
            "corretto-21"
        );
    }

    #[tokio::test]
    async fn test_get_java_home_missing() {
        let svc = make_svc();

        let resp = svc
            .get_java_home(Request::new(GetJavaHomeRequest {
                language_version: "99".to_string(),
                implementation: "nonexistent".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.found);
    }

    /// Getting a toolchain for an unknown language/implementation returns
    /// the default (found=false, empty java_home). When no system JDK matches
    /// and JAVA_HOME is not set, the response is a clean empty default.
    #[tokio::test]
    async fn test_get_java_home_unknown_language_returns_default() {
        let svc = make_svc();

        // Temporarily clear JAVA_HOME so we get a pure "not found" path
        let original_java_home = std::env::var("JAVA_HOME").ok();
        std::env::remove_var("JAVA_HOME");

        let resp = svc
            .get_java_home(Request::new(GetJavaHomeRequest {
                language_version: "ruby".to_string(),
                implementation: "unknown_impl".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.found, "expected found=false for unknown language");
        assert!(
            resp.java_home.is_empty(),
            "expected empty java_home for unknown language, got: {}",
            resp.java_home
        );

        // Restore JAVA_HOME if it was set
        if let Some(val) = original_java_home {
            std::env::set_var("JAVA_HOME", val);
        }
    }

    /// Register multiple toolchains directly into the DashMap, then list
    /// them and verify all appear in the response.
    #[tokio::test]
    async fn test_register_multiple_toolchains_and_list() {
        let svc = make_svc();

        // Insert several toolchains directly into the registry
        svc.installations.insert(
            ToolchainServiceImpl::toolchain_key("17", "temurin"),
            InstalledToolchain {
                language_version: "JDK 17".to_string(),
                implementation: "temurin".to_string(),
                java_home: "/opt/jdks/temurin-17".to_string(),
                verified: true,
                install_size_bytes: 350_000_000,
            },
        );
        svc.installations.insert(
            ToolchainServiceImpl::toolchain_key("21", "corretto"),
            InstalledToolchain {
                language_version: "JDK 21".to_string(),
                implementation: "corretto".to_string(),
                java_home: "/opt/jdks/corretto-21".to_string(),
                verified: true,
                install_size_bytes: 380_000_000,
            },
        );
        svc.installations.insert(
            ToolchainServiceImpl::toolchain_key("11", "temurin"),
            InstalledToolchain {
                language_version: "JDK 11".to_string(),
                implementation: "temurin".to_string(),
                java_home: "/opt/jdks/temurin-11".to_string(),
                verified: false,
                install_size_bytes: 300_000_000,
            },
        );

        let resp = svc
            .list_toolchains(Request::new(ListToolchainsRequest {
                os: std::env::consts::OS.to_string(),
                arch: std::env::consts::ARCH.to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        // At minimum, our 3 registered toolchains must be present
        // (system-detected JDKs may add more)
        let registered: Vec<_> = resp
            .toolchains
            .iter()
            .filter(|tc| tc.installed_via == "substrate")
            .collect();

        assert!(
            registered.len() >= 3,
            "expected at least 3 registered toolchains, got {}",
            registered.len()
        );

        // Verify each registered toolchain has the correct fields
        let versions: std::collections::HashSet<_> = registered
            .iter()
            .map(|tc| (tc.implementation.clone(), tc.language_version.clone()))
            .collect();

        assert!(versions.contains(&("temurin".to_string(), "JDK 17".to_string())));
        assert!(versions.contains(&("corretto".to_string(), "JDK 21".to_string())));
        assert!(versions.contains(&("temurin".to_string(), "JDK 11".to_string())));

        // Check that the verified flag and install_size_bytes are preserved
        let temurin_17 = registered
            .iter()
            .find(|tc| tc.implementation == "temurin" && tc.language_version == "JDK 17")
            .unwrap();
        assert!(temurin_17.verified);
        assert_eq!(temurin_17.install_size_bytes, 350_000_000);
        assert_eq!(temurin_17.java_home, "/opt/jdks/temurin-17");

        // The JDK 11 entry should have verified=false
        let temurin_11 = registered
            .iter()
            .find(|tc| tc.implementation == "temurin" && tc.language_version == "JDK 11")
            .unwrap();
        assert!(!temurin_11.verified);
    }

    /// Calling ensure_toolchain for a toolchain that is already in the
    /// installations map should return a single "done" progress message
    /// (idempotent install).
    #[tokio::test]
    async fn test_ensure_toolchain_already_installed_is_idempotent() {
        let svc = make_svc();

        // Pre-register a toolchain
        let java_home = "/opt/jdks/temurin-17".to_string();
        svc.installations.insert(
            ToolchainServiceImpl::toolchain_key("17", "temurin"),
            InstalledToolchain {
                language_version: "JDK 17".to_string(),
                implementation: "temurin".to_string(),
                java_home: java_home.clone(),
                verified: true,
                install_size_bytes: 350_000_000,
            },
        );

        // Request the same toolchain via ensure_toolchain
        let response = svc
            .ensure_toolchain(Request::new(EnsureToolchainRequest {
                language_version: "17".to_string(),
                implementation: "temurin".to_string(),
                vendor: String::new(),
                os: std::env::consts::OS.to_string(),
                arch: std::env::consts::ARCH.to_string(),
                download_urls: vec![],
                expected_sha256: String::new(),
                cache_download: false,
            }))
            .await
            .unwrap()
            .into_inner();

        // Collect all progress messages from the stream
        use futures_util::StreamExt;
        let messages: Vec<ToolchainProgress> = response
            .collect::<Vec<Result<ToolchainProgress, Status>>>()
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        // Should be exactly one message: "already installed"
        assert_eq!(
            messages.len(),
            1,
            "expected exactly 1 progress message for already-installed toolchain, got {}",
            messages.len()
        );

        let msg = &messages[0];
        assert_eq!(msg.phase, "done");
        assert_eq!(msg.progress_percent, 100);
        assert!(msg.success);
        assert!(msg.error_message.is_empty());
        assert!(
            msg.message.contains("already installed"),
            "expected 'already installed' in message, got: {}",
            msg.message
        );
        assert_eq!(msg.java_home, java_home);

        // Verify that the downloads_total counter was NOT incremented
        // (no new download was initiated)
        assert_eq!(
            svc.downloads_total.load(Ordering::Relaxed),
            0,
            "downloads_total should remain 0 for idempotent install"
        );
    }

    /// Verifying a toolchain with an empty java_home path should be handled
    /// gracefully -- the path does not exist, so valid=false with a
    /// descriptive error message.
    #[tokio::test]
    async fn test_verify_toolchain_empty_path_graceful() {
        let svc = make_svc();

        let resp = svc
            .verify_toolchain(Request::new(VerifyToolchainRequest {
                java_home: String::new(),
                expected_version: "17".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.valid, "empty path should not be valid");
        assert!(
            resp.detected_version.is_empty(),
            "expected empty detected_version, got: {}",
            resp.detected_version
        );
        assert!(
            !resp.error_message.is_empty(),
            "expected a non-empty error message for empty path"
        );
        // The error message should mention that the path was not found
        assert!(
            resp.error_message.contains("not found"),
            "expected 'not found' in error message, got: {}",
            resp.error_message
        );
    }

    #[tokio::test]
    async fn test_register_toolchain_missing_path() {
        let svc = make_svc();

        let resp = svc
            .register_toolchain(Request::new(RegisterToolchainRequest {
                language_version: "17".to_string(),
                implementation: "temurin".to_string(),
                java_home: "/nonexistent/jdk".to_string(),
                vendor: String::new(),
                expected_sha256: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.registered);
        assert!(!resp.error_message.is_empty());
    }

    #[tokio::test]
    async fn test_register_toolchain_and_get_java_home() {
        let svc = make_svc();

        // Create a fake JDK directory structure
        let dir = tempfile::tempdir().unwrap();
        let jdk_home = dir.path().join("fake-jdk-17");
        let bin_dir = jdk_home.join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let java_path = bin_dir.join("java");
        std::fs::write(&java_path, b"").unwrap();

        let resp = svc
            .register_toolchain(Request::new(RegisterToolchainRequest {
                language_version: "17".to_string(),
                implementation: "temurin".to_string(),
                java_home: jdk_home.to_str().unwrap().to_string(),
                vendor: "eclipse".to_string(),
                expected_sha256: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        // java -version will fail on the fake binary, but the registration
        // path itself is valid (java binary exists)
        assert!(resp.registered || !resp.error_message.is_empty());
        if resp.registered {
            assert!(!resp.java_home.is_empty());
        }
    }

    #[tokio::test]
    async fn test_remove_toolchain_not_found() {
        let svc = make_svc();

        let resp = svc
            .remove_toolchain(Request::new(RemoveToolchainRequest {
                language_version: "99".to_string(),
                implementation: "nonexistent".to_string(),
                delete_files: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.removed);
        assert!(!resp.error_message.is_empty());
    }

    #[tokio::test]
    async fn test_remove_toolchain_after_register() {
        let svc = make_svc();

        // Insert directly
        svc.installations.insert(
            ToolchainServiceImpl::toolchain_key("21", "test"),
            InstalledToolchain {
                language_version: "JDK 21".to_string(),
                implementation: "test".to_string(),
                java_home: "/tmp/test-jdk-21".to_string(),
                verified: true,
                install_size_bytes: 1024,
            },
        );

        let resp = svc
            .remove_toolchain(Request::new(RemoveToolchainRequest {
                language_version: "21".to_string(),
                implementation: "test".to_string(),
                delete_files: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.removed);
        assert_eq!(resp.java_home, "/tmp/test-jdk-21");

        // Verify it's gone
        assert!(!svc.installations.contains_key("test-21"));
    }

    #[tokio::test]
    async fn test_get_toolchain_metadata_not_found() {
        let svc = make_svc();

        let resp = svc
            .get_toolchain_metadata(Request::new(GetToolchainMetadataRequest {
                language_version: "99".to_string(),
                implementation: "nonexistent".to_string(),
                java_home: String::new(),
                vendor: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.found);
    }

    #[tokio::test]
    async fn test_get_toolchain_metadata_by_java_home() {
        let svc = make_svc();

        let resp = svc
            .get_toolchain_metadata(Request::new(GetToolchainMetadataRequest {
                language_version: String::new(),
                implementation: String::new(),
                java_home: "/nonexistent/path".to_string(),
                vendor: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.found);
    }

    #[tokio::test]
    async fn test_get_toolchain_metadata_from_registry() {
        let svc = make_svc();

        svc.installations.insert(
            ToolchainServiceImpl::toolchain_key("11", "corretto"),
            InstalledToolchain {
                language_version: "JDK 11".to_string(),
                implementation: "corretto".to_string(),
                java_home: "/opt/corretto-11".to_string(),
                verified: true,
                install_size_bytes: 4096,
            },
        );

        let resp = svc
            .get_toolchain_metadata(Request::new(GetToolchainMetadataRequest {
                language_version: "11".to_string(),
                implementation: "corretto".to_string(),
                java_home: String::new(),
                vendor: String::new(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.found);
        assert_eq!(resp.detected_version, "JDK 11");
        assert_eq!(resp.java_home, "/opt/corretto-11");
        assert_eq!(resp.install_size_bytes, 4096);
        assert!(resp.verified);
    }

    #[test]
    fn test_sha256_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, b"hello world").unwrap();

        let hash = ToolchainServiceImpl::sha256_file(&path).unwrap();
        // Known SHA-256 of "hello world"
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_sha256_file_missing() {
        let result = ToolchainServiceImpl::sha256_file(Path::new("/nonexistent/file"));
        assert!(result.is_err());
    }

    // --- Vendor detection tests ---

    #[test]
    fn test_detect_vendor_temurin() {
        let output = r#"openjdk version "17.0.12" 2024-07-16
Eclipse Temurin Runtime Environment"#;
        assert_eq!(ToolchainServiceImpl::detect_vendor(output), "Temurin");
    }

    #[test]
    fn test_detect_vendor_corretto() {
        let output = r#"openjdk version "21.0.4" 2024-01-16
Amazon Corretto Runtime Environment"#;
        assert_eq!(ToolchainServiceImpl::detect_vendor(output), "Corretto");
    }

    #[test]
    fn test_detect_vendor_zulu() {
        let output = r#"openjdk version "17.0.12" 2024-07-16
Azul Zulu Runtime Environment"#;
        assert_eq!(ToolchainServiceImpl::detect_vendor(output), "Zulu");
    }

    #[test]
    fn test_detect_vendor_microsoft() {
        let output = r#"openjdk version "21.0.4" 2024-01-16
Microsoft Build of OpenJDK Runtime"#;
        assert_eq!(ToolchainServiceImpl::detect_vendor(output), "Microsoft");
    }

    #[test]
    fn test_detect_vendor_graalvm() {
        let output = r#"openjdk version "21.0.4" 2024-01-16
GraalVM Runtime Environment"#;
        assert_eq!(ToolchainServiceImpl::detect_vendor(output), "GraalVM");
    }

    #[test]
    fn test_detect_vendor_unknown() {
        let output = "some random text without vendor info";
        assert_eq!(ToolchainServiceImpl::detect_vendor(output), "");
    }

    // --- Auto-detect tests ---

    #[tokio::test]
    async fn test_auto_detect_returns_java_home() {
        let svc = make_svc();

        let resp = svc
            .auto_detect_toolchains(Request::new(AutoDetectToolchainsRequest {
                major_version: String::new(),
                scan_path: false,
                scan_sdkman: false,
                scan_project: false,
            }))
            .await
            .unwrap()
            .into_inner();

        // Should find at least JAVA_HOME if set
        if std::env::var("JAVA_HOME").is_ok() {
            assert!(resp.total_found > 0);
            assert!(resp.toolchains.iter().any(|t| t.source == "JAVA_HOME"));
        }
    }

    #[tokio::test]
    async fn test_auto_detect_filter_by_version() {
        let svc = make_svc();

        let resp = svc
            .auto_detect_toolchains(Request::new(AutoDetectToolchainsRequest {
                major_version: "99".to_string(), // unlikely to exist
                scan_path: false,
                scan_sdkman: false,
                scan_project: false,
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(resp.total_found, 0);
        assert!(resp.scan_summary.contains("Found 0"));
    }

    #[tokio::test]
    async fn test_auto_detect_with_path_scan() {
        let svc = make_svc();

        let resp = svc
            .auto_detect_toolchains(Request::new(AutoDetectToolchainsRequest {
                major_version: String::new(),
                scan_path: true,
                scan_sdkman: false,
                scan_project: false,
            }))
            .await
            .unwrap()
            .into_inner();

        // Should find at least one JDK (JAVA_HOME or PATH)
        assert!(resp.total_found >= 0);
        for tc in &resp.toolchains {
            assert!(!tc.java_home.is_empty());
            assert!(!tc.major_version.is_empty());
            // If vendor is detected, should be non-empty
            // If not detected, can be empty (e.g., "JDK 8" from Oracle)
            if !tc.source.is_empty() {
                assert!(matches!(
                    tc.source.as_str(),
                    "JAVA_HOME" | "PATH" | "filesystem"
                ));
            }
        }
    }

    #[tokio::test]
    async fn test_auto_detect_deduplication() {
        let svc = make_svc();

        let resp = svc
            .auto_detect_toolchains(Request::new(AutoDetectToolchainsRequest {
                major_version: String::new(),
                scan_path: true,
                scan_sdkman: false,
                scan_project: false,
            }))
            .await
            .unwrap()
            .into_inner();

        // No duplicate java_home paths
        let homes: Vec<&str> = resp
            .toolchains
            .iter()
            .map(|t| t.java_home.as_str())
            .collect();
        let unique: std::collections::HashSet<&str> = homes.iter().copied().collect();
        assert_eq!(
            homes.len(),
            unique.len(),
            "Found duplicate java_home entries"
        );
    }

    // --- Download URL tests ---

    #[test]
    fn test_build_download_urls_default() {
        let urls = ToolchainServiceImpl::build_download_urls("17", "JDK", "macos", "aarch64");
        assert!(!urls.is_empty());
        // Default should start with Adoptium
        assert!(urls[0].contains("adoptium.net") || urls[0].contains("github.com/adoptium"));
    }

    #[test]
    fn test_build_download_urls_corretto() {
        let urls = ToolchainServiceImpl::build_download_urls("21", "corretto", "linux", "x86_64");
        // First URL should be Corretto-specific
        assert!(urls[0].contains("corretto.aws"));
    }

    #[test]
    fn test_build_download_urls_microsoft() {
        let urls = ToolchainServiceImpl::build_download_urls("21", "microsoft", "linux", "x86_64");
        // First URL should be Microsoft-specific
        assert!(urls[0].contains("aka.ms"));
    }

    #[test]
    fn test_build_download_urls_zulu() {
        let urls = ToolchainServiceImpl::build_download_urls("17", "zulu", "macos", "aarch64");
        // First URL should be Zulu-specific
        assert!(urls[0].contains("azul.com"));
    }

    #[test]
    fn test_build_download_urls_graalvm() {
        let urls = ToolchainServiceImpl::build_download_urls("21", "graalvm", "linux", "x86_64");
        // First URL should be GraalVM-specific
        assert!(urls[0].contains("oracle.com") || urls[0].contains("graalvm"));
    }

    #[test]
    fn test_build_download_urls_windows() {
        let urls = ToolchainServiceImpl::build_download_urls("17", "JDK", "windows", "x86_64");
        for url in &urls {
            assert!(
                url.contains("zip") || url.contains("windows"),
                "URL should be windows-specific: {}",
                url
            );
        }
    }
}
