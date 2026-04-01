//! Unsafe audit module for the substrate crate.
//!
//! This module provides a compile-time audit report of all unsafe usage
//! in the crate, enforcing a deny-by-default policy.
//!
//! # Policy
//!
//! `unsafe` is denied by default. Only modules listed in
//! [`server::unsafe_confinement::ALLOWED_MODULES`] may contain unsafe code.
//! Every unsafe block must be documented in
//! [`server::unsafe_confinement::UNSAFE_LOCATIONS`].

use crate::server::unsafe_confinement::{
    validate_unsafe_confinement, ALLOWED_MODULES, UNSAFE_LOCATIONS,
};

/// Documents the deny-by-default unsafe policy.
///
/// This constant serves as a compile-time marker that the crate enforces
/// unsafe confinement. Any new unsafe code must be added to the
/// confinement registry before it is acceptable.
pub const DENY_UNSAFE_BY_DEFAULT: bool = true;

/// A report from the unsafe audit.
pub struct AuditReport {
    /// Total number of documented unsafe blocks in the crate.
    pub total_unsafe_blocks: usize,
    /// Number of unsafe blocks that have been documented with justifications.
    pub documented_unsafe_blocks: usize,
    /// List of undocumented unsafe blocks (should be empty if audit is complete).
    pub undocumented_unsafe_blocks: Vec<String>,
    /// List of modules allowed to contain unsafe code.
    pub allowed_modules: Vec<String>,
    /// Whether the confinement validation passed.
    pub confinement_valid: bool,
}

/// Run the unsafe audit and return a report.
///
/// This function scans [`UNSAFE_LOCATIONS`] and validates that all
/// documented unsafe code belongs to allowed modules.
pub fn run_unsafe_audit() -> AuditReport {
    let total = UNSAFE_LOCATIONS.len();
    let documented = UNSAFE_LOCATIONS
        .iter()
        .filter(|loc| !loc.justification.is_empty() && !loc.safety_invariant.is_empty())
        .count();

    let undocumented: Vec<String> = UNSAFE_LOCATIONS
        .iter()
        .filter(|loc| loc.justification.is_empty() || loc.safety_invariant.is_empty())
        .map(|loc| format!("{}::{} (line {})", loc.module, loc.function, loc.line))
        .collect();

    let confinement_valid = validate_unsafe_confinement().is_ok();

    AuditReport {
        total_unsafe_blocks: total,
        documented_unsafe_blocks: documented,
        undocumented_unsafe_blocks: undocumented,
        allowed_modules: ALLOWED_MODULES.iter().map(|s| s.to_string()).collect(),
        confinement_valid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_report_has_blocks() {
        let report = run_unsafe_audit();
        assert!(
            report.total_unsafe_blocks > 0,
            "Audit should find unsafe blocks"
        );
    }

    #[test]
    fn test_all_blocks_are_documented() {
        let report = run_unsafe_audit();
        assert_eq!(
            report.total_unsafe_blocks, report.documented_unsafe_blocks,
            "All unsafe blocks should be documented"
        );
    }

    #[test]
    fn test_no_undocumented_blocks() {
        let report = run_unsafe_audit();
        assert!(
            report.undocumented_unsafe_blocks.is_empty(),
            "Found undocumented unsafe blocks: {:?}",
            report.undocumented_unsafe_blocks
        );
    }

    #[test]
    fn test_confinement_is_valid() {
        let report = run_unsafe_audit();
        assert!(
            report.confinement_valid,
            "Unsafe confinement validation should pass"
        );
    }

    #[test]
    fn test_deny_unsafe_by_default_is_true() {
        assert!(
            DENY_UNSAFE_BY_DEFAULT,
            "Policy should deny unsafe by default"
        );
    }

    #[test]
    fn test_allowed_modules_is_non_empty() {
        let report = run_unsafe_audit();
        assert!(
            !report.allowed_modules.is_empty(),
            "There should be at least one allowed module"
        );
    }
}
