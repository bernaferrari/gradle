/// Differential test harness: validates Rust service outputs against known-correct values.
///
/// This is the integration test entry point. Individual test modules are in
/// tests/differential/ as submodules.
///
/// Run with: cargo test --test differential
///
/// Tests:
/// - hash_differential_test: 100+ files hashed with MD5/SHA-1/SHA-256 vs reference
/// - cache_differential_test: 50 entries stored/retrieved with byte-for-byte verification
/// - execution_plan_differential_test: DAG topological ordering vs reference sort

mod differential {
    pub mod hash_differential_test;
    pub mod cache_differential_test;
    pub mod execution_plan_differential_test;
}
