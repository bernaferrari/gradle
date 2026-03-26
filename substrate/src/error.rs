use std::path::PathBuf;
use thiserror::Error;

/// Comprehensive error type for the Gradle substrate daemon.
/// Covers all error categories that can occur across the 34 gRPC services.
#[derive(Error, Debug)]
pub enum SubstrateError {
    // --- Infrastructure errors ---
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("gRPC transport error: {0}")]
    Grpc(#[from] tonic::transport::Error),

    #[error("Daemon already running at {0}")]
    AlreadyRunning(String),

    // --- Build script parsing errors ---
    #[error("Parse error: {message}")]
    Parse {
        message: String,
        file: Option<PathBuf>,
        line: Option<u32>,
        #[source]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },

    #[error("Build script syntax error at {file}:{line}: {message}")]
    Syntax {
        message: String,
        file: PathBuf,
        line: u32,
    },

    // --- Hashing/fingerprint errors ---
    #[error("Hash error: {0}")]
    Hash(String),

    #[error("Fingerprint error for {path}: {reason}")]
    Fingerprint {
        path: PathBuf,
        reason: String,
    },

    // --- Cache errors ---
    #[error("Cache error: {0}")]
    Cache(String),

    #[error("Cache entry not found: {key}")]
    CacheMiss { key: String },

    #[error("Cache corruption at {path}: {reason}")]
    CacheCorruption {
        path: PathBuf,
        reason: String,
    },

    // --- Process/worker errors ---
    #[error("Process error: {0}")]
    Process(String),

    #[error("Worker {worker_id} crashed: {reason}")]
    WorkerCrash {
        worker_id: String,
        reason: String,
    },

    #[error("Worker {worker_id} timed out after {timeout_ms}ms")]
    WorkerTimeout {
        worker_id: String,
        timeout_ms: i64,
    },

    #[error("Worker pool exhausted: {reason}")]
    PoolExhausted { reason: String },

    #[error("Worker lease expired for {worker_id}")]
    LeaseExpired { worker_id: String },

    // --- Compilation errors ---
    #[error("Java compilation failed: {message}")]
    CompilationFailed { message: String },

    #[error("Compiler not found: {java_home}/bin/java")]
    CompilerNotFound { java_home: String },

    #[error("Compilation output parse error: {reason}")]
    CompilationParse { reason: String },

    // --- Dependency resolution errors ---
    #[error("Dependency resolution failed for {configuration}: {reason}")]
    DependencyResolution {
        configuration: String,
        reason: String,
    },

    #[error("Dependency not found: {notation}")]
    DependencyNotFound { notation: String },

    #[error("Version conflict for {coordinate}: {reason}")]
    VersionConflict {
        coordinate: String,
        reason: String,
    },

    // --- Toolchain errors ---
    #[error("Toolchain not found: {language} {version}")]
    ToolchainNotFound {
        language: String,
        version: String,
    },

    #[error("Invalid toolchain configuration: {reason}")]
    ToolchainConfig { reason: String },

    // --- Execution errors ---
    #[error("Task execution failed for {task_path}: {reason}")]
    TaskExecution {
        task_path: String,
        reason: String,
    },

    #[error("Execution timeout: {operation} exceeded {timeout_ms}ms")]
    ExecutionTimeout {
        operation: String,
        timeout_ms: i64,
    },

    #[error("Out of memory: {reason}")]
    OutOfMemory { reason: String },

    // --- Configuration errors ---
    #[error("Configuration error: {0}")]
    Configuration(String),

    #[error("Config cache invalid: {reason}")]
    ConfigCacheInvalid { reason: String },

    #[error("Invalid setting: {key} = {value}")]
    InvalidSetting {
        key: String,
        value: String,
    },

    // --- Plugin errors ---
    #[error("Plugin error: {plugin_id}: {reason}")]
    Plugin {
        plugin_id: String,
        reason: String,
    },

    #[error("Plugin not found: {plugin_id}")]
    PluginNotFound { plugin_id: String },

    #[error("Plugin conflict: {plugin_id} conflicts with {conflicts_with}")]
    PluginConflict {
        plugin_id: String,
        conflicts_with: String,
    },

    // --- Resource management errors ---
    #[error("Resource exhausted: {resource_type} (requested={requested}, available={available})")]
    ResourceExhausted {
        resource_type: String,
        requested: i64,
        available: i64,
    },

    #[error("Resource reservation failed: {reason}")]
    ResourceReservation { reason: String },

    // --- Serialization/protocol errors ---
    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Protocol error: {reason}")]
    Protocol { reason: String },

    #[error("Invalid request: {reason}")]
    InvalidRequest { reason: String },

    // --- Build lifecycle errors ---
    #[error("Build {build_id} not found")]
    BuildNotFound { build_id: String },

    #[error("Build {build_id} already in state {state}")]
    BuildStateConflict {
        build_id: String,
        state: String,
    },

    // --- Scope errors ---
    #[error("Scope error: {0}")]
    Scope(String),

    // --- Test execution errors ---
    #[error("Test execution failed: {reason}")]
    TestExecution { reason: String },

    #[error("Test framework error: {reason}")]
    TestFramework { reason: String },
}

impl SubstrateError {
    /// Create a parse error with file and line context.
    pub fn parse_with_context(message: impl Into<String>, file: impl Into<PathBuf>, line: u32) -> Self {
        SubstrateError::Parse {
            message: message.into(),
            file: Some(file.into()),
            line: Some(line),
            source: None,
        }
    }

    /// Create a parse error from an underlying error source.
    pub fn parse_from_err(
        message: impl Into<String>,
        file: impl Into<PathBuf>,
        line: u32,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        SubstrateError::Parse {
            message: message.into(),
            file: Some(file.into()),
            line: Some(line),
            source: Some(Box::new(source)),
        }
    }

    /// Get the error category for logging/metrics.
    pub fn category(&self) -> &'static str {
        match self {
            SubstrateError::Io(_) | SubstrateError::Grpc(_) => "infrastructure",
            SubstrateError::AlreadyRunning(_) => "infrastructure",
            SubstrateError::Parse { .. } | SubstrateError::Syntax { .. } => "parsing",
            SubstrateError::Hash(_) | SubstrateError::Fingerprint { .. } => "hashing",
            SubstrateError::Cache(_)
            | SubstrateError::CacheMiss { .. }
            | SubstrateError::CacheCorruption { .. } => "cache",
            SubstrateError::Process(_)
            | SubstrateError::WorkerCrash { .. }
            | SubstrateError::WorkerTimeout { .. }
            | SubstrateError::PoolExhausted { .. }
            | SubstrateError::LeaseExpired { .. } => "worker",
            SubstrateError::CompilationFailed { .. }
            | SubstrateError::CompilerNotFound { .. }
            | SubstrateError::CompilationParse { .. } => "compilation",
            SubstrateError::DependencyResolution { .. }
            | SubstrateError::DependencyNotFound { .. }
            | SubstrateError::VersionConflict { .. } => "dependency",
            SubstrateError::ToolchainNotFound { .. }
            | SubstrateError::ToolchainConfig { .. } => "toolchain",
            SubstrateError::TaskExecution { .. }
            | SubstrateError::ExecutionTimeout { .. }
            | SubstrateError::OutOfMemory { .. } => "execution",
            SubstrateError::Configuration(_)
            | SubstrateError::ConfigCacheInvalid { .. }
            | SubstrateError::InvalidSetting { .. } => "configuration",
            SubstrateError::Plugin { .. }
            | SubstrateError::PluginNotFound { .. }
            | SubstrateError::PluginConflict { .. } => "plugin",
            SubstrateError::ResourceExhausted { .. }
            | SubstrateError::ResourceReservation { .. } => "resource",
            SubstrateError::Serialization(_) | SubstrateError::Protocol { .. } => "protocol",
            SubstrateError::InvalidRequest { .. } => "protocol",
            SubstrateError::BuildNotFound { .. } | SubstrateError::BuildStateConflict { .. } => {
                "lifecycle"
            }
            SubstrateError::Scope(_) => "scope",
            SubstrateError::TestExecution { .. } | SubstrateError::TestFramework { .. } => {
                "testing"
            }
        }
    }
}

impl From<SubstrateError> for tonic::Status {
    fn from(err: SubstrateError) -> Self {
        let code = match &err {
            SubstrateError::InvalidRequest { .. } => tonic::Code::InvalidArgument,
            SubstrateError::BuildNotFound { .. }
            | SubstrateError::CacheMiss { .. }
            | SubstrateError::PluginNotFound { .. }
            | SubstrateError::DependencyNotFound { .. }
            | SubstrateError::ToolchainNotFound { .. } => tonic::Code::NotFound,
            SubstrateError::AlreadyRunning(_)
            | SubstrateError::BuildStateConflict { .. }
            | SubstrateError::PluginConflict { .. }
            | SubstrateError::VersionConflict { .. }
            | SubstrateError::PoolExhausted { .. }
            | SubstrateError::ResourceExhausted { .. } => tonic::Code::AlreadyExists,
            SubstrateError::ExecutionTimeout { .. }
            | SubstrateError::WorkerTimeout { .. }
            | SubstrateError::LeaseExpired { .. } => tonic::Code::DeadlineExceeded,
            SubstrateError::ConfigCacheInvalid { .. }
            | SubstrateError::CacheCorruption { .. } => tonic::Code::DataLoss,
            SubstrateError::OutOfMemory { .. } => tonic::Code::ResourceExhausted,
            _ => tonic::Code::Internal,
        };

        tonic::Status::new(code, err.to_string())
    }
}

impl From<serde_json::Error> for SubstrateError {
    fn from(err: serde_json::Error) -> Self {
        SubstrateError::Serialization(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_categories() {
        assert_eq!(SubstrateError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound, "not found"
        )).category(), "infrastructure");

        assert_eq!(SubstrateError::Hash("bad hash".to_string()).category(), "hashing");
        assert_eq!(SubstrateError::Cache("full".to_string()).category(), "cache");
        assert_eq!(SubstrateError::Process("crash".to_string()).category(), "worker");
        assert_eq!(SubstrateError::CompilationFailed {
            message: "error".to_string()
        }.category(), "compilation");
        assert_eq!(SubstrateError::DependencyResolution {
            configuration: "compileClasspath".to_string(),
            reason: "not found".to_string()
        }.category(), "dependency");
        assert_eq!(SubstrateError::ToolchainNotFound {
            language: "java".to_string(),
            version: "17".to_string()
        }.category(), "toolchain");
        assert_eq!(SubstrateError::TaskExecution {
            task_path: ":compileJava".to_string(),
            reason: "fail".to_string()
        }.category(), "execution");
        assert_eq!(SubstrateError::Plugin {
            plugin_id: "java".to_string(),
            reason: "missing".to_string()
        }.category(), "plugin");
        assert_eq!(SubstrateError::BuildNotFound {
            build_id: "x".to_string()
        }.category(), "lifecycle");
        assert_eq!(SubstrateError::TestExecution {
            reason: "fail".to_string()
        }.category(), "testing");
    }

    #[test]
    fn test_tonic_status_conversion_invalid_request() {
        let err = SubstrateError::InvalidRequest {
            reason: "bad input".to_string(),
        };
        let status: tonic::Status = err.into();
        assert_eq!(status.code(), tonic::Code::InvalidArgument);
        assert!(status.message().contains("bad input"));
    }

    #[test]
    fn test_tonic_status_conversion_not_found() {
        let err = SubstrateError::BuildNotFound {
            build_id: "missing".to_string(),
        };
        let status: tonic::Status = err.into();
        assert_eq!(status.code(), tonic::Code::NotFound);
    }

    #[test]
    fn test_tonic_status_conversion_timeout() {
        let err = SubstrateError::ExecutionTimeout {
            operation: "compile".to_string(),
            timeout_ms: 5000,
        };
        let status: tonic::Status = err.into();
        assert_eq!(status.code(), tonic::Code::DeadlineExceeded);
    }

    #[test]
    fn test_tonic_status_conversion_internal() {
        let err = SubstrateError::Hash("bad".to_string());
        let status: tonic::Status = err.into();
        assert_eq!(status.code(), tonic::Code::Internal);
    }

    #[test]
    fn test_parse_with_context() {
        let err = SubstrateError::parse_with_context("unexpected token", "/build.gradle.kts", 42);
        let msg = err.to_string();
        assert!(msg.contains("unexpected token"));
    }

    #[test]
    fn test_display_formatting() {
        let err = SubstrateError::WorkerCrash {
            worker_id: "worker-1".to_string(),
            reason: "segfault".to_string(),
        };
        let msg = err.to_string();
        assert_eq!(msg, "Worker worker-1 crashed: segfault");

        let err = SubstrateError::ResourceExhausted {
            resource_type: "memory".to_string(),
            requested: 2048,
            available: 1024,
        };
        let msg = err.to_string();
        assert!(msg.contains("2048"));
        assert!(msg.contains("1024"));
    }

    #[test]
    fn test_serde_json_conversion() {
        let result = serde_json::from_str::<()>("invalid json");
        let err = SubstrateError::from(result.unwrap_err());
        assert_eq!(err.category(), "protocol");
    }
}
