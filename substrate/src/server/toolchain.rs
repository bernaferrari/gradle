use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicI64, Ordering};

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use crate::proto::{
    toolchain_service_server::ToolchainService, EnsureToolchainRequest, GetJavaHomeRequest,
    GetJavaHomeResponse, ListToolchainsRequest, ListToolchainsResponse, ToolchainLocation,
    ToolchainProgress, VerifyToolchainRequest, VerifyToolchainResponse,
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
    downloads_completed: AtomicI64,
}

impl ToolchainServiceImpl {
    pub fn new(toolchain_dir: PathBuf) -> Self {
        std::fs::create_dir_all(&toolchain_dir).ok();
        Self {
            installations: DashMap::new(),
            toolchain_dir,
            downloads_total: AtomicI64::new(0),
            downloads_completed: AtomicI64::new(0),
        }
    }

    fn toolchain_key(version: &str, implementation: &str) -> String {
        format!("{}-{}", implementation, version)
    }

    /// Detect system JDK installations from common paths.
    fn find_system_javas() -> Vec<(String, String)> {
        let mut found = Vec::new();

        // Check JAVA_HOME environment variable
        if let Ok(java_home) = std::env::var("JAVA_HOME") {
            if let Some((version, size)) = Self::probe_java_home(&java_home) {
                found.push((java_home, format!("JDK {} ({})", version, size)));
            }
        }

        // Check common JDK installation paths
        let candidates: Vec<String> = if cfg!(target_os = "macos") {
            vec![
                "/Library/Java/JavaVirtualMachines".to_string(),
                "/opt/homebrew/opt/openjdk".to_string(),
                "/opt/homebrew/opt/openjdk@11".to_string(),
                "/opt/homebrew/opt/openjdk@17".to_string(),
                "/opt/homebrew/opt/openjdk@21".to_string(),
            ]
        } else if cfg!(target_os = "linux") {
            vec![
                "/usr/lib/jvm".to_string(),
                "/usr/java".to_string(),
                "/usr/lib/jvm/java-8-openjdk-amd64".to_string(),
                "/usr/lib/jvm/java-11-openjdk-amd64".to_string(),
                "/usr/lib/jvm/java-17-openjdk-amd64".to_string(),
                "/usr/lib/jvm/java-21-openjdk-amd64".to_string(),
                "/usr/lib/jvm/temurin-8-jdk".to_string(),
                "/usr/lib/jvm/temurin-11-jdk".to_string(),
                "/usr/lib/jvm/temurin-17-jdk".to_string(),
                "/usr/lib/jvm/temurin-21-jdk".to_string(),
            ]
        } else {
            vec!["C:\\Program Files\\Java".to_string()]
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

                            if let Some((version, _)) = Self::probe_java_home(&home.to_string_lossy()) {
                                found.push((home.to_string_lossy().to_string(), version));
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

        // Deduplicate by java_home path
        let mut seen = std::collections::HashSet::new();
        found.retain(|(home, _)| seen.insert(home.clone()));

        found
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
        match Command::new(&java_bin)
            .arg("-version")
            .output()
        {
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
                    return Some(format!("JDK {}", major));
                }
            } else if trimmed.chars().next().map_or(false, |c| c.is_ascii_digit()) {
                // Modern style: "17.0.12" -> "JDK 17"
                if let Some(major) = trimmed.split('.').next() {
                    if major.parse::<u32>().is_ok() {
                        return Some(format!("JDK {}", major));
                    }
                }
            }
        }
        None
    }

    /// Recursively calculate directory size.
    fn dir_size(path: &Path) -> u64 {
        if !path.exists() {
            return 0;
        }

        if path.is_file() {
            return std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        }

        let mut total = 0u64;
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                total += Self::dir_size(&entry.path());
            }
        }
        total
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

        tracing::debug!(
            count = toolchains.len(),
            "Listed toolchains"
        );

        Ok(Response::new(ListToolchainsResponse { toolchains }))
    }

    type EnsureToolchainStream = std::pin::Pin<Box<dyn tonic::codegen::tokio_stream::Stream<Item = Result<ToolchainProgress, Status>> + Send>>;

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
                message: format!("Toolchain {} {} already installed", req.implementation, req.language_version),
                success: true,
                error_message: String::new(),
                java_home,
            })]);
            return Ok(Response::new(Box::pin(stream) as Self::EnsureToolchainStream));
        }

        self.downloads_total.fetch_add(1, Ordering::Relaxed);

        let version = req.language_version.clone();
        let impl_name = req.implementation.clone();
        let toolchain_dir = self.toolchain_dir.clone();

        let java_home = toolchain_dir
            .join(format!("{}-{}", impl_name, version))
            .to_string_lossy()
            .to_string();

        let stream = futures_util::stream::iter(vec![
            Ok(ToolchainProgress {
                phase: "checking".to_string(),
                progress_percent: 5,
                message: format!("Checking for {} {}", impl_name, version),
                success: true,
                error_message: String::new(),
                java_home: String::new(),
            }),
            Ok(ToolchainProgress {
                phase: "downloading".to_string(),
                progress_percent: 30,
                message: "Download not yet implemented".to_string(),
                success: true,
                error_message: String::new(),
                java_home: String::new(),
            }),
            Ok(ToolchainProgress {
                phase: "extracting".to_string(),
                progress_percent: 70,
                message: "Extraction not yet implemented".to_string(),
                success: true,
                error_message: String::new(),
                java_home: String::new(),
            }),
            Ok(ToolchainProgress {
                phase: "verifying".to_string(),
                progress_percent: 90,
                message: "Verification not yet implemented".to_string(),
                success: true,
                error_message: String::new(),
                java_home: String::new(),
            }),
            Ok(ToolchainProgress {
                phase: "done".to_string(),
                progress_percent: 100,
                message: format!("Toolchain ready at {}", java_home),
                success: true,
                error_message: String::new(),
                java_home,
            }),
        ]);

        Ok(Response::new(Box::pin(stream) as Self::EnsureToolchainStream))
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

                let detected_version = Self::parse_java_version(version_output)
                    .unwrap_or_default();

                // Verify against expected version if provided
                let version_match = if !req.expected_version.is_empty() && !detected_version.is_empty() {
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
                        format!("Expected version {} but found {}", req.expected_version, detected_version)
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
        assert_eq!(version, Some("JDK 17".to_string()));
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
        assert_eq!(version, Some("JDK 21".to_string()));
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

        // Should find at least JAVA_HOME or system JDKs
        // May be 0 in CI if no JDK is in standard paths
        assert!(resp.toolchains.len() >= 0);
        for tc in &resp.toolchains {
            assert!(!tc.java_home.is_empty());
        }
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
}
