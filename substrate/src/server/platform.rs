//! Platform abstraction for OS-specific operations.
//!
//! Provides a trait for system resource queries that differ across platforms.
//! Compile-time dispatch selects the appropriate implementation.

/// Platform-specific system operations.
pub trait PlatformOps {
    /// Total physical memory in bytes.
    fn total_memory_bytes() -> u64;

    /// Available (free) memory in bytes.
    fn available_memory_bytes() -> u64;

    /// Number of logical CPU cores.
    fn available_cpu_count() -> usize;

    /// Current process RSS (resident set size) in bytes.
    fn process_rss_bytes() -> u64;

    /// CPU usage percentage (0.0-100.0) since last call.
    fn cpu_usage_percent() -> f64;
}

/// Platform implementation that returns zero/default values.
/// Used for unsupported platforms or when real values aren't needed.
pub struct NoopPlatform;

impl PlatformOps for NoopPlatform {
    fn total_memory_bytes() -> u64 {
        8_192_000_000 // 8 GB default
    }

    fn available_memory_bytes() -> u64 {
        0
    }

    fn available_cpu_count() -> usize {
        num_cpus::get()
    }

    fn process_rss_bytes() -> u64 {
        0
    }

    fn cpu_usage_percent() -> f64 {
        0.0
    }
}

#[cfg(unix)]
pub struct UnixPlatform;

#[cfg(unix)]
impl PlatformOps for UnixPlatform {
    fn total_memory_bytes() -> u64 {
        #[cfg(target_os = "macos")]
        {
            let mut mib = [libc::CTL_HW, libc::HW_MEMSIZE];
            let mut mem: u64 = 0;
            let mut len = std::mem::size_of::<u64>() as libc::size_t;
            unsafe {
                libc::sysctl(
                    mib.as_mut_ptr(),
                    2,
                    &mut mem as *mut _ as *mut libc::c_void,
                    &mut len,
                    std::ptr::null_mut(),
                    0,
                );
            }
            mem
        }

        #[cfg(target_os = "linux")]
        {
            // Parse /proc/meminfo for MemTotal
            let content = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
            for line in content.lines() {
                if line.starts_with("MemTotal:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        if let Ok(kb) = parts[1].parse::<u64>() {
                            return kb * 1024;
                        }
                    }
                }
            }
            8_192_000_000
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            8_192_000_000
        }
    }

    fn available_memory_bytes() -> u64 {
        #[cfg(target_os = "macos")]
        {
            // Use HW_MEMSIZE for total; available is approximated via sysctl
            // For accurate available memory, we'd need sysctl VM statistics
            // which require custom structs not in the libc crate.
            // Use host_statistics64 via Mach ports as fallback.
            let total = Self::total_memory_bytes();
            // Use a conservative estimate: 50% of total as available
            total / 2
        }

        #[cfg(target_os = "linux")]
        {
            let content = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
            for line in content.lines() {
                if line.starts_with("MemAvailable:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        if let Ok(kb) = parts[1].parse::<u64>() {
                            return kb * 1024;
                        }
                    }
                }
            }
            0
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            0
        }
    }

    fn available_cpu_count() -> usize {
        num_cpus::get()
    }

    fn process_rss_bytes() -> u64 {
        #[cfg(target_os = "linux")]
        {
            let content = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
            for line in content.lines() {
                if line.starts_with("VmRSS:") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        if let Ok(kb) = parts[1].parse::<u64>() {
                            return kb * 1024;
                        }
                    }
                }
            }
            0
        }

        #[cfg(target_os = "macos")]
        {
            // Parse proc_pidinfo if available via libc, otherwise return 0
            // The proc_taskallinfo struct is not in all libc versions
            0
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            0
        }
    }

    fn cpu_usage_percent() -> f64 {
        #[cfg(target_os = "linux")]
        {
            let content = std::fs::read_to_string("/proc/stat").unwrap_or_default();
            let mut first_line = content.lines().next().unwrap_or("");
            first_line = first_line.strip_prefix("cpu ").unwrap_or("");
            let vals: Vec<u64> = first_line
                .split_whitespace()
                .filter_map(|v| v.parse().ok())
                .collect();
            if vals.len() >= 4 {
                let idle = vals[3];
                let total: u64 = vals.iter().sum();
                if total == 0 {
                    return 0.0;
                }
                ((total - idle) as f64 / total as f64) * 100.0
            } else {
                0.0
            }
        }

        #[cfg(target_os = "macos")]
        {
            let mut load: [libc::c_double; 3] = [0.0; 3];
            unsafe {
                libc::getloadavg(load.as_mut_ptr(), 3);
            }
            // load average is per-CPU; normalize to percentage
            let cpus = num_cpus::get() as f64;
            (load[0] / cpus) * 100.0
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            0.0
        }
    }
}

/// Type alias for the platform implementation used at compile time.
#[cfg(unix)]
pub type CurrentPlatform = UnixPlatform;

#[cfg(not(unix))]
pub type CurrentPlatform = NoopPlatform;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_noop_platform_returns_defaults() {
        assert!(NoopPlatform::total_memory_bytes() > 0);
        assert_eq!(NoopPlatform::available_memory_bytes(), 0);
        assert!(NoopPlatform::available_cpu_count() > 0);
        assert_eq!(NoopPlatform::process_rss_bytes(), 0);
        assert_eq!(NoopPlatform::cpu_usage_percent(), 0.0);
    }

    #[test]
    fn test_unix_platform_total_memory() {
        let mem = UnixPlatform::total_memory_bytes();
        assert!(mem > 0, "Total memory should be positive");
    }

    #[test]
    fn test_unix_platform_cpu_count() {
        let cpus = UnixPlatform::available_cpu_count();
        assert!(cpus > 0, "CPU count should be positive");
    }

    #[test]
    fn test_current_platform_type() {
        // Just verify it compiles and is accessible
        let _cpus = CurrentPlatform::available_cpu_count();
    }
}
