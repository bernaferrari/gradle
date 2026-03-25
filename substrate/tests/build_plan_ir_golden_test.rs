use std::fs;
use std::path::PathBuf;

use gradle_substrate_daemon::server::build_plan_ir::{
    canonical_json, fingerprint_sha256_hex, to_envelope, to_proto, validate_schema_version,
    CanonicalBuildPlan,
};

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join("build_plan_ir")
        .join(name)
}

fn load_fixture(name: &str) -> CanonicalBuildPlan {
    let path = fixture_path(name);
    let raw = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed reading fixture {}: {}", path.display(), e));
    serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("failed parsing fixture {}: {}", path.display(), e))
}

#[test]
fn build_plan_ir_golden_fingerprint_is_stable_and_order_insensitive() {
    let fixture_a = load_fixture("simple-plan-v1.json");
    let fixture_b = load_fixture("simple-plan-v1-shuffled.json");

    validate_schema_version(&fixture_a).unwrap();
    validate_schema_version(&fixture_b).unwrap();

    let fp_a = fingerprint_sha256_hex(&fixture_a).unwrap();
    let fp_b = fingerprint_sha256_hex(&fixture_b).unwrap();
    assert_eq!(fp_a, fp_b, "logical plan equivalence must keep fingerprint");

    // Golden lock: this hash should only change when canonicalization policy or
    // fixture semantics intentionally change.
    let expected = "36358745888389d08b45dcddd60a09666592e2589a16f4f7644044ae73132a3d";
    assert_eq!(fp_a, expected, "unexpected canonical fingerprint drift");
}

#[test]
fn build_plan_ir_golden_proto_envelope_is_self_consistent() {
    let fixture = load_fixture("simple-plan-v1.json");
    let envelope = to_envelope(&fixture).unwrap();
    let fp = fingerprint_sha256_hex(&fixture).unwrap();

    assert_eq!(envelope.plan_fingerprint_sha256, fp);
    assert!(envelope.plan.is_some());

    let canonical = canonical_json(&fixture).unwrap();
    let proto = to_proto(&fixture);
    assert_eq!(proto.schema_version, 1);
    assert_eq!(proto.build_id, "sample-build-1");
    assert!(!canonical.is_empty());
}
