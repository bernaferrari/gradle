use std::env;
use std::process::Command;

fn main() {
    // Emit version info from Cargo.toml and git
    let version = env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "unknown".into());
    let target = env::var("TARGET").unwrap_or_else(|_| "unknown".into());
    let profile = env::var("PROFILE").unwrap_or_else(|_| "unknown".into());
    
    // Get git commit hash if available
    let git_hash = if let Ok(output) = Command::new("git").args(["rev-parse", "--short", "HEAD"]).output() {
        if output.status.success() {
            String::from_utf8_lossy(&output.stdout).trim().to_string()
        } else {
            "unknown".into()
        }
    } else {
        "unknown".into()
    };
    
    println!("cargo:rustc-env=APP_VERSION={version}");
    println!("cargo:rustc-env=APP_COMMIT={git_hash}");
    println!("cargo:rustc-env=APP_TARGET={target}");
    println!("cargo:rustc-env=APP_PROFILE={profile}");
    println!("cargo:rerun-if-changed=proto/v1/");
    
    let proto_files: Vec<&str> = vec![
        "proto/v1/control.proto",
        "proto/v1/hash.proto",
        "proto/v1/cache.proto",
        "proto/v1/classpath.proto",
        "proto/v1/exec.proto",
        "proto/v1/execution.proto",
        "proto/v1/fingerprint.proto",
        "proto/v1/filetree.proto",
        "proto/v1/taskgraph.proto",
        "proto/v1/configuration.proto",
        "proto/v1/buildops.proto",
        "proto/v1/bootstrap.proto",
        "proto/v1/dependency.proto",
        "proto/v1/filewatch.proto",
        "proto/v1/toolchain.proto",
        "proto/v1/worker.proto",
        "proto/v1/buildlayout.proto",
        "proto/v1/buildplan.proto",
        "proto/v1/reporting.proto",
        "proto/v1/resources.proto",
        "proto/v1/testexec.proto",
        "proto/v1/publishing.proto",
        "proto/v1/incremental.proto",
        "proto/v1/metrics.proto",
        "proto/v1/jvmhost.proto",
        "proto/v1/parser.proto",
        "proto/v1/versioncatalog.proto",
        "proto/v1/ide.proto",
        "proto/v1/native_compile.proto",
    ];

    tonic_build::configure()
        .compile_protos(&proto_files, &["proto"])
        .unwrap();
}
