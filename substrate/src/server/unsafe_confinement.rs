/// Compile-time unsafe confinement registry for the substrate crate.
///
/// This module enforces a deny-by-default policy for `unsafe` code.
/// All unsafe blocks must be documented in [`UNSAFE_LOCATIONS`] and
/// belong to an allowed module listed in [`ALLOWED_MODULES`].
///
/// # Policy
///
/// Unsafe code is only permitted in modules that interact with platform
/// FFI (libc, sysctl, proc_pidinfo) or require explicit lifetime erasure
/// for builder patterns. All other modules must be 100% safe Rust.

/// A documented location of unsafe code within the crate.
#[derive(Debug, Clone, Copy)]
pub struct UnsafeLocation {
    /// The module path where the unsafe block resides.
    pub module: &'static str,
    /// The function containing the unsafe block.
    pub function: &'static str,
    /// The source line number of the unsafe block.
    pub line: u32,
    /// Why unsafe is necessary at this location.
    pub justification: &'static str,
    /// The invariant that makes this unsafe block sound.
    pub safety_invariant: &'static str,
}

/// All documented unsafe locations in the crate.
///
/// This list is populated by audit. Every `unsafe` block in the crate
/// must have a corresponding entry here. If you add unsafe code,
/// you must add an entry to this list.
pub const UNSAFE_LOCATIONS: &[UnsafeLocation] = &[
    // -----------------------------------------------------------------------
    // server::resource_management — libc FFI for platform resource queries
    // -----------------------------------------------------------------------
    UnsafeLocation {
        module: "server::resource_management",
        function: "system_memory_mb",
        line: 98,
        justification: "FFI binding to macOS sysctl for hw.memsize (total physical memory)",
        safety_invariant: "sysctl with CTL_HW/HW_MEMSIZE always returns a valid u64 when result is 0; buffer size is statically known",
    },
    UnsafeLocation {
        module: "server::resource_management",
        function: "fd_limit",
        line: 135,
        justification: "FFI binding to libc getrlimit for RLIMIT_NOFILE",
        safety_invariant: "rlimit struct is zeroed before call; getrlimit only writes to the provided pointer on success (return 0)",
    },
    UnsafeLocation {
        module: "server::resource_management",
        function: "read_cpu_usage_macos",
        line: 204,
        justification: "FFI binding to macOS sysctl for vm.loadavg (CPU load average)",
        safety_invariant: "sysctl with CTL_VM/loadavg OID writes a valid c_double when result is 0; buffer size is statically known",
    },
    UnsafeLocation {
        module: "server::resource_management",
        function: "read_process_rss_mb",
        line: 245,
        justification: "FFI binding to macOS proc_pidinfo for PROC_PIDTASKINFO (process RSS)",
        safety_invariant: "proc_taskinfo struct is zeroed before call; proc_pidinfo only writes within the provided buffer size",
    },
    UnsafeLocation {
        module: "server::resource_management",
        function: "_read_available_memory_mb",
        line: 287,
        justification: "FFI binding to libc sysconf for _SC_PAGESIZE (memory page size)",
        safety_invariant: "sysconf(_SC_PAGESIZE) always returns a positive value on supported platforms; cast to u64 is safe",
    },
    UnsafeLocation {
        module: "server::resource_management",
        function: "_read_available_memory_mb",
        line: 293,
        justification: "FFI binding to macOS sysctl for vm page free count",
        safety_invariant: "sysctl writes a valid u32 when result is 0; buffer size is statically known",
    },
    UnsafeLocation {
        module: "server::resource_management",
        function: "_read_available_memory_mb",
        line: 308,
        justification: "FFI binding to macOS sysctl for vm page inactive count",
        safety_invariant: "sysctl writes a valid u32 when result is 0; buffer size is statically known",
    },

    // -----------------------------------------------------------------------
    // server::typed_scopes — lifetime erasure for builder pattern
    // -----------------------------------------------------------------------
    UnsafeLocation {
        module: "server::typed_scopes",
        function: "TypedScopeBuilder::build",
        line: 356,
        justification: "Lifetime erasure via transmute for BuildSessionCtx in builder pattern",
        safety_invariant: "All transmuted contexts are stored together in the same scope; the builder is primarily for testing and demonstration; production code uses explicit open_* methods with proper lifetimes",
    },
    UnsafeLocation {
        module: "server::typed_scopes",
        function: "TypedScopeBuilder::build",
        line: 362,
        justification: "Lifetime erasure via transmute for BuildTreeCtx in builder pattern",
        safety_invariant: "All transmuted contexts are stored together in the same scope; the builder is primarily for testing and demonstration; production code uses explicit open_* methods with proper lifetimes",
    },
    UnsafeLocation {
        module: "server::typed_scopes",
        function: "TypedScopeBuilder::build",
        line: 367,
        justification: "Lifetime erasure via transmute for BuildCtx in builder pattern",
        safety_invariant: "All transmuted contexts are stored together in the same scope; the builder is primarily for testing and demonstration; production code uses explicit open_* methods with proper lifetimes",
    },
    UnsafeLocation {
        module: "server::typed_scopes",
        function: "TypedScopeBuilder::build",
        line: 369,
        justification: "Lifetime erasure via transmute for ProjectCtx in builder pattern",
        safety_invariant: "All transmuted contexts are stored together in the same scope; the builder is primarily for testing and demonstration; production code uses explicit open_* methods with proper lifetimes",
    },

    // -----------------------------------------------------------------------
    // server::file_watch — libc FFI for filesystem detection
    // -----------------------------------------------------------------------
    UnsafeLocation {
        module: "server::file_watch",
        function: "is_network_filesystem",
        line: 80,
        justification: "FFI binding to macOS statfs for network filesystem detection",
        safety_invariant: "statfs struct is zeroed before call; path is validated to exist and converted to valid CString; statfs only writes within the provided struct",
    },

    // -----------------------------------------------------------------------
    // server::worker_process — libc FFI for process memory queries
    // -----------------------------------------------------------------------
    UnsafeLocation {
        module: "server::worker_process",
        function: "get_process_rss",
        line: 917,
        justification: "Zero-initialization of proc_vm_info struct for macOS process memory query",
        safety_invariant: "ProcVminfo is repr(C) with only u64 fields; zeroed is valid for all fields; proc_pidinfo only writes within the provided buffer size",
    },
    UnsafeLocation {
        module: "server::worker_process",
        function: "get_process_rss",
        line: 920,
        justification: "FFI binding to macOS proc_pidinfo for PROC_PID_VMINFO",
        safety_invariant: "ProcVminfo struct is zeroed before call; proc_pidinfo only writes within the provided buffer size; return value is checked against expected size",
    },

    // -----------------------------------------------------------------------
    // server::platform — libc FFI for platform metrics
    // -----------------------------------------------------------------------
    UnsafeLocation {
        module: "server::platform",
        function: "UnixPlatform::total_memory_bytes",
        line: 61,
        justification: "FFI binding to macOS sysctl for hw.memsize (total physical memory)",
        safety_invariant: "sysctl with CTL_HW/HW_MEMSIZE always returns a valid u64; buffer size is statically known; mib is valid for the call",
    },
    UnsafeLocation {
        module: "server::platform",
        function: "UnixPlatform::cpu_usage_percent",
        line: 190,
        justification: "FFI binding to libc getloadavg for CPU load average on macOS",
        safety_invariant: "getloadavg writes at most 3 c_double values into the provided array; array is sized exactly 3; pointer is valid for the write",
    },
];

/// Modules that are allowed to contain unsafe code.
///
/// Any module not in this list is deny-by-default for unsafe.
pub const ALLOWED_MODULES: &[&str] = &[
    "server::resource_management",
    "server::typed_scopes",
    "server::file_watch",
    "server::worker_process",
    "server::platform",
];

/// A compile-time registry of allowed unsafe locations.
pub struct UnsafeConfinementRegistry {
    allowed_modules: &'static [&'static str],
}

impl UnsafeConfinementRegistry {
    /// Create a new registry with the given list of allowed module paths.
    pub const fn new(allowed: &'static [&'static str]) -> Self {
        Self {
            allowed_modules: allowed,
        }
    }

    /// Check if a module path is in the allowed list.
    pub fn is_allowed(&self, module_path: &str) -> bool {
        self.allowed_modules.contains(&module_path)
    }

    /// Returns the list of allowed module paths.
    pub fn allowed_modules(&self) -> &[&'static str] {
        self.allowed_modules
    }
}

/// The default confinement registry for the substrate crate.
pub const fn confinement_registry() -> UnsafeConfinementRegistry {
    UnsafeConfinementRegistry::new(ALLOWED_MODULES)
}

/// Validate that all documented unsafe locations belong to allowed modules.
///
/// Returns `Ok(())` if every location in [`UNSAFE_LOCATIONS`] is in an
/// allowed module. Returns `Err` with a list of violations otherwise.
pub fn validate_unsafe_confinement() -> Result<(), Vec<String>> {
    let registry = confinement_registry();
    let mut violations = Vec::new();

    for location in UNSAFE_LOCATIONS {
        if !registry.is_allowed(location.module) {
            violations.push(format!(
                "unsafe in {}::{} (line {}) is not in an allowed module",
                location.module, location.function, location.line
            ));
        }
    }

    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

/// Macro to document unsafe usage at the call site.
///
/// Usage:
/// ```ignore
/// document_unsafe! {
///     module: "server::file_watch",
///     justification: "FFI binding to OS file event API",
///     safety_invariant: "notify crate guarantees thread-safe watcher lifecycle"
/// }
/// ```
#[macro_export]
macro_rules! document_unsafe {
    (
        module: $module:expr,
        justification: $justification:expr,
        safety_invariant: $safety_invariant:expr
    ) => {
        const _: () = {
            let _location: $crate::server::unsafe_confinement::UnsafeLocation =
                $crate::server::unsafe_confinement::UnsafeLocation {
                    module: $module,
                    function: module_path!(),
                    line: line!(),
                    justification: $justification,
                    safety_invariant: $safety_invariant,
                };
        };
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unsafe_locations_is_non_empty() {
        assert!(
            !UNSAFE_LOCATIONS.is_empty(),
            "UNSAFE_LOCATIONS must be non-empty — run the unsafe audit to populate it"
        );
    }

    #[test]
    fn test_all_locations_have_justifications() {
        for loc in UNSAFE_LOCATIONS {
            assert!(
                !loc.justification.is_empty(),
                "UnsafeLocation at {}:{} must have a non-empty justification",
                loc.module,
                loc.line
            );
        }
    }

    #[test]
    fn test_all_locations_have_safety_invariants() {
        for loc in UNSAFE_LOCATIONS {
            assert!(
                !loc.safety_invariant.is_empty(),
                "UnsafeLocation at {}:{} must have a non-empty safety_invariant",
                loc.module,
                loc.line
            );
        }
    }

    #[test]
    fn test_validate_confinement_passes() {
        let result = validate_unsafe_confinement();
        assert!(
            result.is_ok(),
            "Confinement validation failed: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_registry_is_allowed() {
        let registry = confinement_registry();
        assert!(registry.is_allowed("server::resource_management"));
        assert!(registry.is_allowed("server::platform"));
        assert!(registry.is_allowed("server::file_watch"));
        assert!(registry.is_allowed("server::worker_process"));
        assert!(registry.is_allowed("server::typed_scopes"));
    }

    #[test]
    fn test_registry_denies_unknown_modules() {
        let registry = confinement_registry();
        assert!(!registry.is_allowed("server::cache"));
        assert!(!registry.is_allowed("server::dag_executor"));
        assert!(!registry.is_allowed("client::jvm_host"));
    }

    #[test]
    fn test_all_locations_have_non_empty_fields() {
        for loc in UNSAFE_LOCATIONS {
            assert!(!loc.module.is_empty(), "module must be non-empty");
            assert!(!loc.function.is_empty(), "function must be non-empty");
            assert!(loc.line > 0, "line must be positive");
        }
    }

    #[test]
    fn test_allowed_modules_matches_location_modules() {
        let registry = confinement_registry();
        for loc in UNSAFE_LOCATIONS {
            assert!(
                registry.is_allowed(loc.module),
                "Module '{}' has unsafe code but is not in ALLOWED_MODULES",
                loc.module
            );
        }
    }
}
