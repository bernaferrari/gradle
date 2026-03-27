fn main() {
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
    ];

    tonic_build::configure()
        .compile_protos(&proto_files, &["proto"])
        .unwrap();
}
