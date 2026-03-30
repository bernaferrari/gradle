//! Native compilation support service.
//!
//! Parses compile_commands.json (the standard C++ compilation database format)
//! and provides compiler information via gRPC.

use std::path::Path;

use tonic::{Request, Response, Status};

use crate::proto::{
    native_compile_service_server::NativeCompileService, CompileCommand, CompilerInfo,
    GetCompilerInfoRequest, GetCompilerInfoResponse, ParseCompileCommandsRequest,
    ParseCompileCommandsResponse,
};

#[derive(Debug, Default)]
pub struct NativeCompileServiceImpl;

#[tonic::async_trait]
impl NativeCompileService for NativeCompileServiceImpl {
    async fn get_compiler_info(
        &self,
        _request: Request<GetCompilerInfoRequest>,
    ) -> Result<Response<GetCompilerInfoResponse>, Status> {
        // Try to detect compilers on the system
        let compilers = detect_compilers();
        Ok(Response::new(GetCompilerInfoResponse {
            compilers,
            error_message: String::new(),
        }))
    }

    async fn parse_compile_commands(
        &self,
        request: Request<ParseCompileCommandsRequest>,
    ) -> Result<Response<ParseCompileCommandsResponse>, Status> {
        let path = &request.get_ref().path;

        // If path is a directory, look for compile_commands.json inside it
        let json_path = Path::new(path);
        let json_path = if json_path.is_dir() {
            json_path.join("compile_commands.json")
        } else {
            json_path.to_path_buf()
        };

        if !json_path.exists() {
            return Ok(Response::new(ParseCompileCommandsResponse {
                entries: vec![],
                total_entries: 0,
                error_message: format!("File not found: {}", json_path.display()),
            }));
        }

        let content = std::fs::read_to_string(&json_path)
            .map_err(|e| Status::internal(format!("Failed to read {}: {}", json_path.display(), e)))?;

        let entries: Vec<serde_json::Value> = serde_json::from_str(&content)
            .map_err(|e| Status::internal(format!("Failed to parse JSON: {}", e)))?;

        let commands: Vec<CompileCommand> = entries
            .into_iter()
            .filter_map(|entry| {
                let directory = entry.get("directory")?.as_str()?.to_string();
                let file = entry.get("file")?.as_str()?.to_string();
                let command = entry
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let arguments: Vec<String> = entry
                    .get("arguments")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let output = entry
                    .get("output")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                Some(CompileCommand {
                    directory,
                    file,
                    command,
                    arguments,
                    output,
                })
            })
            .collect();

        let total = commands.len() as i32;

        Ok(Response::new(ParseCompileCommandsResponse {
            entries: commands,
            total_entries: total,
            error_message: String::new(),
        }))
    }
}

/// Detect compilers available on the system.
fn detect_compilers() -> Vec<CompilerInfo> {
    let mut compilers = vec![];

    // Try clang
    if let Some(info) = get_compiler_version("clang", &["--version"]) {
        compilers.push(CompilerInfo {
            name: "clang".to_string(),
            path: info.path,
            version: info.version,
            target_triple: info.target,
            supported_languages: vec!["c".to_string(), "c++".to_string(), "objc".to_string()],
        });
    }

    // Try gcc
    if let Some(info) = get_compiler_version("gcc", &["--version"]) {
        compilers.push(CompilerInfo {
            name: "gcc".to_string(),
            path: info.path,
            version: info.version,
            target_triple: info.target,
            supported_languages: vec!["c".to_string(), "c++".to_string()],
        });
    }

    compilers
}

struct CompilerVersionInfo {
    path: String,
    version: String,
    target: String,
}

/// Get compiler version by running it with the given args.
fn get_compiler_version(name: &str, args: &[&str]) -> Option<CompilerVersionInfo> {
    let output = std::process::Command::new(name)
        .args(args)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = stdout.lines().next().unwrap_or("").to_string();

    let path = which_compiler(name)?;

    Some(CompilerVersionInfo {
        path,
        version,
        target: String::new(),
    })
}

/// Find the path to a compiler binary.
fn which_compiler(name: &str) -> Option<String> {
    let output = std::process::Command::new("which")
        .arg(name)
        .output()
        .ok()?;

    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path.is_empty() {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_nonexistent_file() {
        let service = NativeCompileServiceImpl;
        let req = Request::new(ParseCompileCommandsRequest {
            path: "/nonexistent/compile_commands.json".to_string(),
        });
        let rt = tokio::runtime::Runtime::new().unwrap();
        let resp = rt.block_on(service.parse_compile_commands(req)).unwrap();
        assert!(resp.get_ref().error_message.contains("File not found"));
        assert_eq!(resp.get_ref().total_entries, 0);
    }

    #[test]
    fn test_parse_valid_compile_commands() {
        let dir = tempfile::tempdir().unwrap();
        let json_path = dir.path().join("compile_commands.json");
        std::fs::write(
            &json_path,
            r#"[
                {"directory": "/src", "file": "main.c", "command": "clang -c main.c -o main.o", "output": "main.o"},
                {"directory": "/src", "file": "lib.c", "command": "clang -c lib.c -o lib.o", "output": "lib.o"}
            ]"#,
        )
        .unwrap();

        let service = NativeCompileServiceImpl;
        let req = Request::new(ParseCompileCommandsRequest {
            path: json_path.to_string_lossy().to_string(),
        });
        let rt = tokio::runtime::Runtime::new().unwrap();
        let resp = rt.block_on(service.parse_compile_commands(req)).unwrap();
        assert_eq!(resp.get_ref().total_entries, 2);
        assert_eq!(resp.get_ref().entries[0].file, "main.c");
        assert_eq!(resp.get_ref().entries[1].file, "lib.c");
    }

    #[test]
    fn test_parse_directory_with_compile_commands() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("compile_commands.json"),
            r#"[{"directory": "/src", "file": "test.c", "command": "gcc -c test.c"}]"#,
        )
        .unwrap();

        let service = NativeCompileServiceImpl;
        let req = Request::new(ParseCompileCommandsRequest {
            path: dir.path().to_string_lossy().to_string(),
        });
        let rt = tokio::runtime::Runtime::new().unwrap();
        let resp = rt.block_on(service.parse_compile_commands(req)).unwrap();
        assert_eq!(resp.get_ref().total_entries, 1);
    }

    #[test]
    fn test_parse_with_arguments_array() {
        let dir = tempfile::tempdir().unwrap();
        let json_path = dir.path().join("compile_commands.json");
        std::fs::write(
            &json_path,
            r#"[{"directory": "/src", "file": "main.cpp", "arguments": ["clang++", "-std=c++17", "-c", "main.cpp", "-o", "main.o"]}]"#,
        )
        .unwrap();

        let service = NativeCompileServiceImpl;
        let req = Request::new(ParseCompileCommandsRequest {
            path: json_path.to_string_lossy().to_string(),
        });
        let rt = tokio::runtime::Runtime::new().unwrap();
        let resp = rt.block_on(service.parse_compile_commands(req)).unwrap();
        assert_eq!(resp.get_ref().total_entries, 1);
        assert_eq!(resp.get_ref().entries[0].arguments.len(), 6);
    }

    #[test]
    fn test_detect_compilers_returns_something() {
        let compilers = detect_compilers();
        // At least one compiler should be found on a development machine
        // (but don't fail if none found, as CI may not have them)
        // Just verify the function runs without panic
        let _ = compilers;
    }
}
