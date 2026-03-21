use std::sync::atomic::{AtomicI64, Ordering};

use dashmap::DashMap;
use tonic::{Request, Response, Status};

use crate::proto::{
    resource_management_service_server::ResourceManagementService, GetResourceLimitsRequest,
    GetResourceLimitsResponse, GetResourceUsageRequest, GetResourceUsageResponse,
    ReleaseResourcesRequest, ReleaseResourcesResponse, ReserveResourcesRequest,
    ReserveResourcesResponse, ResourceLimit, ResourceUsageEntry, SetResourceLimitsRequest,
    SetResourceLimitsResponse,
};

/// A tracked resource reservation.
struct Reservation {
    reservation_id: String,
    build_id: String,
    resources: Vec<(String, i64)>, // (resource_type, amount)
    created_at_ms: i64,
}

/// A tracked resource with current usage and limits.
struct ResourceSlot {
    total_capacity: i64,
    used: i64,
    soft_limit: i64,
}

/// Rust-native resource management service.
/// Tracks and limits build resources (memory, CPU, file descriptors).
pub struct ResourceManagementServiceImpl {
    reservations: DashMap<String, Reservation>,
    resources: DashMap<String, ResourceSlot>,
    build_limits: DashMap<String, DashMap<String, ResourceLimit>>,
    next_reservation_id: AtomicI64,
    reservations_total: AtomicI64,
}

impl ResourceManagementServiceImpl {
    pub fn new() -> Self {
        let mut resources = DashMap::new();
        resources.insert(
            "memory_mb".to_string(),
            ResourceSlot {
                total_capacity: Self::system_memory_mb(),
                used: 0,
                soft_limit: 0,
            },
        );
        resources.insert(
            "cpu_cores".to_string(),
            ResourceSlot {
                total_capacity: num_cpus::get() as i64,
                used: 0,
                soft_limit: 0,
            },
        );
        resources.insert(
            "file_descriptors".to_string(),
            ResourceSlot {
                total_capacity: Self::fd_limit(),
                used: 0,
                soft_limit: 0,
            },
        );
        resources.insert(
            "threads".to_string(),
            ResourceSlot {
                total_capacity: 256,
                used: 0,
                soft_limit: 0,
            },
        );

        Self {
            reservations: DashMap::new(),
            resources,
            build_limits: DashMap::new(),
            next_reservation_id: AtomicI64::new(1),
            reservations_total: AtomicI64::new(0),
        }
    }

    fn system_memory_mb() -> i64 {
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("sysctl")
                .args(["-n", "hw.memsize"])
                .output()
                .ok()
                .and_then(|out| String::from_utf8(out.stdout).ok())
                .and_then(|s| s.trim().parse::<u64>().ok())
                .map(|bytes| (bytes / 1024 / 1024) as i64)
                .unwrap_or(8192)
        }
        #[cfg(target_os = "linux")]
        {
            std::fs::read_to_string("/proc/meminfo")
                .ok()
                .and_then(|s| {
                    s.lines()
                        .find(|l| l.starts_with("MemTotal:"))
                        .and_then(|l| l.split_whitespace().nth(1))
                        .and_then(|v| v.parse::<i64>().ok())
                })
                .map(|kb| kb / 1024)
                .unwrap_or(8192)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            8192
        }
    }

    /// Get the system file descriptor limit using getrlimit.
    fn fd_limit() -> i64 {
        unsafe {
            let mut rlim: libc::rlimit = std::mem::zeroed();
            if libc::getrlimit(libc::RLIMIT_NOFILE, &mut rlim) == 0 {
                rlim.rlim_cur as i64
            } else {
                1024
            }
        }
    }

    /// Read current CPU usage percentage via /proc/stat (Linux) or sysctl (macOS).
    fn read_cpu_usage_percent() -> f64 {
        #[cfg(target_os = "linux")]
        {
            Self::read_cpu_usage_linux()
        }
        #[cfg(target_os = "macos")]
        {
            Self::read_cpu_usage_macos()
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            0.0
        }
    }

    #[cfg(target_os = "linux")]
    fn read_cpu_usage_linux() -> f64 {
        let cpu_line = match std::fs::read_to_string("/proc/stat") {
            Ok(content) => content.lines().next().map(|l| l.to_string()),
            Err(_) => None,
        };

        let cpu_line = match cpu_line {
            Some(line) => line,
            None => return 0.0,
        };

        // Format: cpu  user nice system idle iowait irq softirq steal guest guest_nice
        let parts: Vec<u64> = cpu_line
            .split_whitespace()
            .skip(1) // skip "cpu"
            .filter_map(|s| s.parse::<u64>().ok())
            .collect();

        if parts.len() < 4 {
            return 0.0;
        }

        let idle = parts[3];
        let iowait = if parts.len() > 4 { parts[4] } else { 0 };
        let total_idle = idle + iowait;
        let total: u64 = parts.iter().sum();

        if total == 0 {
            return 0.0;
        }

        // CPU usage = 1.0 - (idle / total)
        1.0 - (total_idle as f64 / total as f64)
    }

    #[cfg(target_os = "macos")]
    fn read_cpu_usage_macos() -> f64 {
        // Use sysctl vm.loadavg for load average (proxy for CPU pressure)
        // A load average > CPU count indicates saturation
        std::process::Command::new("sysctl")
            .args(["-n", "vm.loadavg"])
            .output()
            .ok()
            .and_then(|out| String::from_utf8(out.stdout).ok())
            .and_then(|s| {
                // Output format: { 1.23 0.98 0.76 }
                let trimmed = s.trim().trim_start_matches('{').trim_end_matches('}');
                let parts: Vec<&str> = trimmed.split_whitespace().collect();
                parts.first().and_then(|p| p.parse::<f64>().ok())
            })
            .map(|load_avg| {
                let cpu_count = num_cpus::get() as f64;
                // Convert load average to percentage (capped at 100%)
                let usage = load_avg / cpu_count;
                if usage > 1.0 { 100.0 } else { usage * 100.0 }
            })
            .unwrap_or(0.0)
    }

    /// Read current process RSS memory usage in MB.
    fn read_process_rss_mb() -> i64 {
        #[cfg(target_os = "linux")]
        {
            std::fs::read_to_string("/proc/self/status")
                .ok()
                .and_then(|s| {
                    s.lines()
                        .find(|l| l.starts_with("VmRSS:"))
                        .and_then(|l| l.split_whitespace().nth(1))
                        .and_then(|v| v.parse::<i64>().ok())
                })
                .map(|kb| kb / 1024)
                .unwrap_or(0)
        }
        #[cfg(target_os = "macos")]
        {
            // Use proc_info to get RSS on macOS
            unsafe {
                let mut info: libc::proc_taskinfo = std::mem::zeroed();
                let ret = libc::proc_pidinfo(
                    libc::getpid(),
                    libc::PROC_PIDTASKINFO,
                    0,
                    &mut info as *mut _ as *mut libc::c_void,
                    std::mem::size_of::<libc::proc_taskinfo>() as libc::c_int,
                );
                if ret > 0 {
                    (info.pti_resident_size / 1024 / 1024) as i64
                } else {
                    0
                }
            }
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            0
        }
    }

    /// Read system memory available in MB.
    fn read_available_memory_mb() -> i64 {
        #[cfg(target_os = "linux")]
        {
            std::fs::read_to_string("/proc/meminfo")
                .ok()
                .and_then(|s| {
                    s.lines()
                        .find(|l| l.starts_with("MemAvailable:"))
                        .and_then(|l| l.split_whitespace().nth(1))
                        .and_then(|v| v.parse::<i64>().ok())
                })
                .map(|kb| kb / 1024)
                .unwrap_or(0)
        }
        #[cfg(target_os = "macos")]
        {
            // On macOS, approximate available memory using vm_stat
            std::process::Command::new("vm_stat")
                .output()
                .ok()
                .and_then(|out| String::from_utf8(out.stdout).ok())
                .map(|s| {
                    let mut free_pages: u64 = 0;
                    let mut inactive_pages: u64 = 0;
                    for line in s.lines() {
                        if let Some(val) = line
                            .strip_prefix("Pages free:")
                            .or_else(|| line.strip_prefix("Pages free:"))
                        {
                            free_pages = val.trim().trim_end_matches('.').parse().unwrap_or(0);
                        }
                        if let Some(val) = line.strip_prefix("Pages inactive:") {
                            inactive_pages = val.trim().trim_end_matches('.').parse().unwrap_or(0);
                        }
                    }
                    // page_size is 4096 on all modern macOS
                    ((free_pages + inactive_pages) * 4096 / 1024 / 1024) as i64
                })
                .unwrap_or(0)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            0
        }
    }

    fn generate_reservation_id(&self) -> String {
        let id = self.next_reservation_id.fetch_add(1, Ordering::Relaxed);
        format!("res-{}", id)
    }

    /// Clean up stale reservations older than the given timeout.
    fn cleanup_stale_reservations(&self, timeout_ms: i64) -> i32 {
        let now = Self::now_ms();
        let stale_ids: Vec<String> = self
            .reservations
            .iter()
            .filter(|entry| now - entry.value().created_at_ms > timeout_ms)
            .map(|entry| entry.key().clone())
            .collect();

        let cleaned = stale_ids.len() as i32;
        for id in &stale_ids {
            if let Some((_, reservation)) = self.reservations.remove(id) {
                for (resource_type, amount) in &reservation.resources {
                    if let Some(mut slot) = self.resources.get_mut(resource_type) {
                        slot.used = (slot.used - amount).max(0);
                    }
                }
            }
        }
        cleaned
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

#[tonic::async_trait]
impl ResourceManagementService for ResourceManagementServiceImpl {
    async fn reserve_resources(
        &self,
        request: Request<ReserveResourcesRequest>,
    ) -> Result<Response<ReserveResourcesResponse>, Status> {
        let req = request.into_inner();

        if req.resources.is_empty() {
            return Err(Status::invalid_argument("No resources requested"));
        }

        let reservation_id = self.generate_reservation_id();

        // Check availability for each resource
        for resource in &req.resources {
            if let Some(mut slot) = self.resources.get_mut(&resource.resource_type) {
                let available = slot.total_capacity - slot.used;
                if resource.amount > available {
                    return Ok(Response::new(ReserveResourcesResponse {
                        granted: false,
                        reservation_id: String::new(),
                        denial_reason: format!(
                            "Insufficient {}: requested {}, available {}",
                            resource.resource_type,
                            resource.amount,
                            available
                        ),
                    }));
                }
            } else {
                // Auto-register unknown resource types with unlimited capacity
                self.resources.insert(
                    resource.resource_type.clone(),
                    ResourceSlot {
                        total_capacity: i64::MAX,
                        used: 0,
                        soft_limit: 0,
                    },
                );
            }
        }

        // Grant the reservation
        let mut reserved = Vec::new();
        for resource in &req.resources {
            if let Some(mut slot) = self.resources.get_mut(&resource.resource_type) {
                slot.used += resource.amount;
                reserved.push((resource.resource_type.clone(), resource.amount));
            }
        }

        self.reservations.insert(
            reservation_id.clone(),
            Reservation {
                reservation_id: reservation_id.clone(),
                build_id: req.build_id.clone(),
                resources: reserved,
                created_at_ms: Self::now_ms(),
            },
        );

        self.reservations_total.fetch_add(1, Ordering::Relaxed);

        tracing::debug!(
            reservation_id = %reservation_id,
            build_id = %req.build_id,
            "Resources reserved"
        );

        Ok(Response::new(ReserveResourcesResponse {
            granted: true,
            reservation_id,
            denial_reason: String::new(),
        }))
    }

    async fn release_resources(
        &self,
        request: Request<ReleaseResourcesRequest>,
    ) -> Result<Response<ReleaseResourcesResponse>, Status> {
        let req = request.into_inner();

        if let Some((_key, reservation)) = self.reservations.remove(&req.reservation_id) {
            for (resource_type, amount) in &reservation.resources {
                if let Some(mut slot) = self.resources.get_mut(resource_type) {
                    slot.used = (slot.used - amount).max(0);
                }
            }

            tracing::debug!(
                reservation_id = %req.reservation_id,
                "Resources released"
            );

            Ok(Response::new(ReleaseResourcesResponse { released: true }))
        } else {
            Ok(Response::new(ReleaseResourcesResponse { released: false }))
        }
    }

    async fn get_resource_usage(
        &self,
        request: Request<GetResourceUsageRequest>,
    ) -> Result<Response<GetResourceUsageResponse>, Status> {
        let _req = request.into_inner();

        // Clean up stale reservations (older than 30 minutes)
        self.cleanup_stale_reservations(30 * 60 * 1000);

        let mut usage = Vec::new();
        for entry in self.resources.iter() {
            usage.push(ResourceUsageEntry {
                resource_type: entry.key().clone(),
                total_capacity: entry.total_capacity,
                used: entry.used,
                reserved: entry.used, // all usage is reserved
                available: entry.total_capacity - entry.used,
                waiters: 0,
            });
        }

        let total_memory = self
            .resources
            .get("memory_mb")
            .map(|r| r.total_capacity * 1024 * 1024)
            .unwrap_or(0);
        let used_memory = self
            .resources
            .get("memory_mb")
            .map(|r| r.used * 1024 * 1024)
            .unwrap_or(0);

        let cpu_usage_percent = Self::read_cpu_usage_percent();
        let process_rss_bytes = Self::read_process_rss_mb() * 1024 * 1024;

        Ok(Response::new(GetResourceUsageResponse {
            usage,
            total_memory_bytes: total_memory,
            used_memory_bytes: used_memory,
            cpu_usage_percent,
            active_threads: self
                .resources
                .get("threads")
                .map(|r| r.used as i32)
                .unwrap_or(0),
        }))
    }

    async fn get_resource_limits(
        &self,
        request: Request<GetResourceLimitsRequest>,
    ) -> Result<Response<GetResourceLimitsResponse>, Status> {
        let _req = request.into_inner();

        let limits: Vec<ResourceLimit> = self
            .resources
            .iter()
            .map(|entry| ResourceLimit {
                resource_type: entry.key().clone(),
                max_amount: entry.total_capacity,
                soft_limit: entry.soft_limit,
            })
            .collect();

        Ok(Response::new(GetResourceLimitsResponse { limits }))
    }

    async fn set_resource_limits(
        &self,
        request: Request<SetResourceLimitsRequest>,
    ) -> Result<Response<SetResourceLimitsResponse>, Status> {
        let req = request.into_inner();

        for limit in req.limits {
            if let Some(mut slot) = self.resources.get_mut(&limit.resource_type) {
                if limit.max_amount > 0 {
                    slot.total_capacity = limit.max_amount;
                }
                if limit.soft_limit > 0 {
                    slot.soft_limit = limit.soft_limit;
                }
            }

            // Store per-build limits
            if !req.build_id.is_empty() {
                self.build_limits
                    .entry(req.build_id.clone())
                    .or_insert_with(DashMap::new)
                    .insert(limit.resource_type.clone(), limit);
            }
        }

        Ok(Response::new(SetResourceLimitsResponse { applied: true }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proto::ResourceRequest;

    fn make_resource_req(resource_type: &str, amount: i64) -> ResourceRequest {
        ResourceRequest {
            resource_type: resource_type.to_string(),
            amount,
            requester_id: "test".to_string(),
        }
    }

    #[tokio::test]
    async fn test_reserve_and_release() {
        let svc = ResourceManagementServiceImpl::new();

        let resp = svc
            .reserve_resources(Request::new(ReserveResourcesRequest {
                build_id: "build-1".to_string(),
                resources: vec![make_resource_req("memory_mb", 512)],
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.granted);
        assert!(!resp.reservation_id.is_empty());

        let usage = svc
            .get_resource_usage(Request::new(GetResourceUsageRequest {
                build_id: "build-1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        let mem = usage.usage.iter().find(|u| u.resource_type == "memory_mb").unwrap();
        assert_eq!(mem.used, 512);

        // Release
        svc.release_resources(Request::new(ReleaseResourcesRequest {
            reservation_id: resp.reservation_id,
        }))
        .await
        .unwrap();

        let usage = svc
            .get_resource_usage(Request::new(GetResourceUsageRequest {
                build_id: "build-1".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        let mem = usage.usage.iter().find(|u| u.resource_type == "memory_mb").unwrap();
        assert_eq!(mem.used, 0);
    }

    #[tokio::test]
    async fn test_over_reserve_denied() {
        let svc = ResourceManagementServiceImpl::new();

        // Set a small limit
        svc.set_resource_limits(Request::new(SetResourceLimitsRequest {
            build_id: String::new(),
            limits: vec![ResourceLimit {
                resource_type: "memory_mb".to_string(),
                max_amount: 100,
                soft_limit: 80,
            }],
        }))
        .await
        .unwrap();

        // First reservation should succeed
        let resp1 = svc
            .reserve_resources(Request::new(ReserveResourcesRequest {
                build_id: "build-2".to_string(),
                resources: vec![make_resource_req("memory_mb", 80)],
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp1.granted);

        // Second should fail (only 20 left, need 80)
        let resp2 = svc
            .reserve_resources(Request::new(ReserveResourcesRequest {
                build_id: "build-2".to_string(),
                resources: vec![make_resource_req("memory_mb", 80)],
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp2.granted);
        assert!(!resp2.denial_reason.is_empty());
    }

    #[tokio::test]
    async fn test_multi_resource_reservation() {
        let svc = ResourceManagementServiceImpl::new();

        let resp = svc
            .reserve_resources(Request::new(ReserveResourcesRequest {
                build_id: "build-3".to_string(),
                resources: vec![
                    make_resource_req("memory_mb", 256),
                    make_resource_req("cpu_cores", 2),
                    make_resource_req("file_descriptors", 16),
                ],
                timeout_ms: 5000,
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(resp.granted);

        let usage = svc
            .get_resource_usage(Request::new(GetResourceUsageRequest {
                build_id: "build-3".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert_eq!(usage.usage.len(), 4); // memory, cpu, fd, threads
    }

    #[tokio::test]
    async fn test_release_nonexistent() {
        let svc = ResourceManagementServiceImpl::new();

        let resp = svc
            .release_resources(Request::new(ReleaseResourcesRequest {
                reservation_id: "nonexistent".to_string(),
            }))
            .await
            .unwrap()
            .into_inner();

        assert!(!resp.released);
    }

    #[tokio::test]
    async fn test_empty_request() {
        let svc = ResourceManagementServiceImpl::new();

        let result = svc
            .reserve_resources(Request::new(ReserveResourcesRequest {
                build_id: "build-4".to_string(),
                resources: vec![],
                timeout_ms: 5000,
            }))
            .await;

        assert!(result.is_err());
    }
}
