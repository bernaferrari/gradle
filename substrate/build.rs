fn main() {
    tonic_build::configure()
        .compile_protos(&["proto/v1/substrate.proto"], &["proto"])
        .unwrap();
}
