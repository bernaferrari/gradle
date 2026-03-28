use std::collections::{BinaryHeap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;

use dashmap::DashMap;
use tokio::sync::Notify;

/// Priority levels for task scheduling. Lower value = higher priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum TaskPriority {
    /// Standard task — normal scheduling.
    #[default]
    Normal = 1,
    /// Critical path task — must execute first for build throughput.
    Critical = 0,
    /// Low-priority task — e.g. verification, reporting.
    Low = 2,
}

/// Resource estimate for a task.
#[derive(Debug, Clone, Default)]
pub struct ResourceEstimate {
    /// Estimated CPU intensity (0.0 to 1.0).
    pub cpu_weight: f64,
    /// Estimated memory usage in bytes.
    pub memory_bytes: u64,
    /// Estimated duration in milliseconds.
    pub estimated_ms: u64,
}

/// A task waiting to be scheduled, with priority ordering.
#[derive(Debug, Clone)]
struct PrioritizedTask {
    task_path: String,
    priority: TaskPriority,
    /// Higher critical path remaining time = scheduled first.
    critical_path_remaining_ms: u64,
    /// Insertion order as tiebreaker (FIFO within same priority).
    sequence: u64,
}

impl Ord for PrioritizedTask {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Higher priority = comes first (reverse enum ordering)
        other
            .priority
            .cmp(&self.priority)
            .then_with(|| {
                self.critical_path_remaining_ms
                    .cmp(&other.critical_path_remaining_ms)
            })
            .then_with(|| self.sequence.cmp(&other.sequence))
    }
}

impl PartialOrd for PrioritizedTask {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Eq for PrioritizedTask {}
impl PartialEq for PrioritizedTask {
    fn eq(&self, other: &Self) -> bool {
        self.task_path == other.task_path
    }
}

/// Statistics for the scheduler.
#[derive(Debug, Clone, Default)]
pub struct SchedulerStats {
    pub tasks_scheduled: u64,
    pub tasks_completed: u64,
    pub tasks_stolen: u64,
    pub total_wait_time_ms: u64,
    pub max_queue_depth: u64,
    pub active_workers: u32,
    pub current_memory_usage: u64,
}

/// Configuration for the parallel scheduler.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Maximum concurrent tasks across all workers.
    pub max_parallelism: usize,
    /// Maximum memory budget for concurrent tasks (bytes). 0 = unlimited.
    pub memory_budget: u64,
    /// Whether to use CPU count for parallelism.
    pub cpu_aware: bool,
    /// Number of worker tasks for work stealing.
    pub worker_count: usize,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        Self {
            max_parallelism: cpus,
            memory_budget: 0,
            cpu_aware: true,
            worker_count: cpus,
        }
    }
}

/// State tracked per task in the scheduler.
struct TrackedTask {
    task_path: String,
    priority: TaskPriority,
    resource_estimate: ResourceEstimate,
    dependencies: Vec<String>,
    dependents: Vec<String>,
    status: TaskStatus,
}

#[derive(Debug, Clone, PartialEq)]
enum TaskStatus {
    Pending,
    Ready,
    Executing,
    Succeeded,
    Failed,
    Skipped,
}

/// Work-stealing parallel scheduler for DAG-based task execution.
///
/// Multiple "worker" tasks compete to pull tasks from a shared priority queue.
/// When a worker finishes a task, it unblocks dependents and immediately tries
/// to steal more work. This maximizes parallelism without over-subscription.
pub struct ParallelScheduler {
    config: SchedulerConfig,
    tasks: DashMap<String, TrackedTask>,
    ready_queue: std::sync::Mutex<BinaryHeap<PrioritizedTask>>,
    executing: DashMap<String, Instant>,
    /// Sequence counter for FIFO ordering within same priority.
    sequence: AtomicU64,
    /// Signal workers when new tasks become ready.
    work_available: Notify,
    /// Cancellation flag.
    cancelled: AtomicBool,
    /// Resource tracking.
    current_memory: AtomicU64,
    /// Stats.
    tasks_scheduled: AtomicU64,
    tasks_completed: AtomicU64,
    tasks_stolen: AtomicU64,
    total_wait_ms: AtomicU64,
    max_queue_depth: AtomicU64,
}

impl ParallelScheduler {
    /// Create a new parallel scheduler with default configuration.
    pub fn new() -> Self {
        Self::with_config(SchedulerConfig::default())
    }
}

impl Default for ParallelScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl ParallelScheduler {
    /// Create a new parallel scheduler with custom configuration.
    pub fn with_config(config: SchedulerConfig) -> Self {
        let effective_parallelism = if config.cpu_aware {
            std::thread::available_parallelism()
                .map(|n| n.get().min(config.max_parallelism))
                .unwrap_or(config.max_parallelism)
        } else {
            config.max_parallelism
        };

        let effective_config = SchedulerConfig {
            max_parallelism: effective_parallelism,
            worker_count: effective_parallelism.min(config.worker_count),
            ..config
        };

        Self {
            config: effective_config,
            tasks: DashMap::new(),
            ready_queue: std::sync::Mutex::new(BinaryHeap::new()),
            executing: DashMap::new(),
            sequence: AtomicU64::new(0),
            work_available: Notify::new(),
            cancelled: AtomicBool::new(false),
            current_memory: AtomicU64::new(0),
            tasks_scheduled: AtomicU64::new(0),
            tasks_completed: AtomicU64::new(0),
            tasks_stolen: AtomicU64::new(0),
            total_wait_ms: AtomicU64::new(0),
            max_queue_depth: AtomicU64::new(0),
        }
    }

    /// Register a task with its dependencies, priority, and resource estimate.
    pub fn register_task(
        &self,
        task_path: &str,
        dependencies: Vec<String>,
        priority: TaskPriority,
        resource_estimate: ResourceEstimate,
    ) {
        self.tasks.insert(
            task_path.to_string(),
            TrackedTask {
                task_path: task_path.to_string(),
                priority,
                resource_estimate,
                dependencies,
                dependents: Vec::new(), // populated during build_graph
                status: TaskStatus::Pending,
            },
        );
    }

    /// Build the dependent links from the registered tasks.
    /// Must be called after all tasks are registered.
    pub fn build_graph(&self) {
        // Phase 1: Collect all dependency -> dependent relationships (iter then drop)
        let task_count = self.tasks.len();
        let mut dep_map: HashMap<String, Vec<String>> = HashMap::with_capacity(task_count);
        let mut ready: Vec<String> = Vec::with_capacity(task_count);

        {
            // Collect deps and find ready tasks in one pass
            for entry in self.tasks.iter() {
                for dep in &entry.dependencies {
                    dep_map
                        .entry(dep.clone())
                        .or_default()
                        .push(entry.task_path.clone());
                }
                if entry.dependencies.is_empty() {
                    ready.push(entry.task_path.clone());
                }
            }
        }
        // iter() guard dropped here

        // Phase 2: Update dependents in task entries
        for (dep, dependents) in dep_map {
            if let Some(mut task) = self.tasks.get_mut(&dep) {
                task.dependents = dependents;
            }
        }

        // Phase 3: Enqueue ready tasks
        let mut queue = self
            .ready_queue
            .lock()
            .expect("ready_queue lock should not be poisoned");
        for task_path in &ready {
            let priority = self
                .tasks
                .get(task_path)
                .map(|t| t.priority)
                .unwrap_or(TaskPriority::Normal);
            let seq_num = self.sequence.fetch_add(1, Ordering::Relaxed);
            queue.push(PrioritizedTask {
                task_path: task_path.clone(),
                priority,
                critical_path_remaining_ms: 0,
                sequence: seq_num,
            });
            if let Some(mut task) = self.tasks.get_mut(task_path) {
                task.status = TaskStatus::Ready;
            }
        }
    }

    /// Compute critical path remaining time for each task (reverse topological order).
    /// Tasks with longer remaining critical path get higher scheduling priority.
    pub fn compute_critical_paths(&self) {
        // For each task, critical_path_remaining = max(own_estimate, max(dependent_critical_path + dependent_estimate))
        // We iterate in reverse topological order (tasks with no dependents first)

        let task_count = self.tasks.len();
        if task_count == 0 {
            return;
        }

        // Collect all task paths
        let all_paths: Vec<String> = self.tasks.iter().map(|e| e.task_path.clone()).collect();

        // Multiple passes until stable (simple iterative approach)
        let mut critical_paths: HashMap<String, u64> = HashMap::with_capacity(task_count);

        for _ in 0..task_count {
            for path in &all_paths {
                let own_estimate = self
                    .tasks
                    .get(path)
                    .map(|t| t.resource_estimate.estimated_ms)
                    .unwrap_or(100);

                let max_dependent = self
                    .tasks
                    .get(path)
                    .map(|t| {
                        t.dependents
                            .iter()
                            .filter_map(|dep| critical_paths.get(dep).copied())
                            .max()
                            .unwrap_or(0)
                    })
                    .unwrap_or(0);

                let cp = own_estimate.max(max_dependent + own_estimate);
                critical_paths.insert(path.clone(), cp);
            }
        }

        // Update priorities in the ready queue
        let mut queue = self
            .ready_queue
            .lock()
            .expect("ready_queue lock should not be poisoned");
        let mut new_queue = BinaryHeap::new();
        while let Some(mut task) = queue.pop() {
            if let Some(cp) = critical_paths.get(&task.task_path) {
                task.critical_path_remaining_ms = *cp;
            }
            new_queue.push(task);
        }
        *queue = new_queue;
        self.work_available.notify_waiters();
    }

    /// Try to steal the next available task.
    /// Returns None if no tasks are available (queue empty, parallelism limit reached, or cancelled).
    pub async fn try_steal(&self) -> Option<StolenTask> {
        // Check cancellation
        if self.cancelled.load(Ordering::Relaxed) {
            return None;
        }

        // Check parallelism limit
        if self.executing.len() >= self.config.max_parallelism {
            return None;
        }

        // Check memory budget
        if self.config.memory_budget > 0 {
            let current = self.current_memory.load(Ordering::Relaxed);
            if current >= self.config.memory_budget {
                return None;
            }
        }

        // Try to pop from queue (single pass)
        let prioritized = {
            let mut queue = self
                .ready_queue
                .lock()
                .expect("ready_queue lock should not be poisoned");
            let mut result = None;
            let mut requeue = Vec::with_capacity(4);

            while let Some(task) = queue.pop() {
                if let Some(t) = self.tasks.get(&task.task_path) {
                    if t.status == TaskStatus::Ready {
                        // Check if dependencies are actually satisfied
                        let deps_met = t.dependencies.iter().all(|dep| {
                            self.tasks
                                .get(dep)
                                .map(|d| {
                                    matches!(d.status, TaskStatus::Succeeded | TaskStatus::Skipped)
                                })
                                .unwrap_or(true)
                        });

                        if deps_met {
                            result = Some(task);
                            break;
                        } else {
                            requeue.push(task);
                        }
                    }
                } else {
                    self.tasks_stolen.fetch_add(1, Ordering::Relaxed);
                }
            }

            // Put back tasks whose deps aren't met
            for task in requeue {
                queue.push(task);
            }
            result
        };

        let prioritized = prioritized?;

        // Claim the task
        if let Some(mut task) = self.tasks.get_mut(&prioritized.task_path) {
            task.status = TaskStatus::Executing;
        }
        self.executing
            .insert(prioritized.task_path.clone(), Instant::now());
        self.tasks_scheduled.fetch_add(1, Ordering::Relaxed);

        let resource = self
            .tasks
            .get(&prioritized.task_path)
            .map(|t| t.resource_estimate.clone())
            .unwrap_or_default();

        Some(StolenTask {
            task_path: prioritized.task_path,
            priority: prioritized.priority,
            resource_estimate: resource,
        })
    }

    /// Notify that a task has completed successfully.
    /// Unblocks dependents and makes newly ready tasks available for stealing.
    pub async fn complete_task(&self, task_path: &str, _duration_ms: u64, success: bool) {
        let outcome = if success {
            TaskStatus::Succeeded
        } else {
            TaskStatus::Failed
        };

        // Update task status
        if let Some(mut task) = self.tasks.get_mut(task_path) {
            task.status = outcome.clone();
        }

        // Remove from executing
        self.executing.remove(task_path);
        self.tasks_completed.fetch_add(1, Ordering::Relaxed);

        let mut newly_ready = Vec::with_capacity(8);

        if success {
            // Check dependents
            let dependents: Vec<String> = self
                .tasks
                .get(task_path)
                .map(|t| t.dependents.clone())
                .unwrap_or_default();

            for dep_path in &dependents {
                // Check if task is pending and deps are met
                let should_ready = {
                    let task = match self.tasks.get(dep_path) {
                        Some(t) => t,
                        None => continue,
                    };
                    if task.status != TaskStatus::Pending {
                        continue;
                    }

                    task.dependencies.iter().all(|dep| {
                        self.tasks
                            .get(dep)
                            .map(|d| {
                                matches!(d.status, TaskStatus::Succeeded | TaskStatus::Skipped)
                            })
                            .unwrap_or(true)
                    })
                };
                // Immutable borrow dropped here

                if should_ready {
                    if let Some(mut task) = self.tasks.get_mut(dep_path) {
                        task.status = TaskStatus::Ready;
                    }
                    newly_ready.push(dep_path.clone());
                }
            }
        } else {
            // Skip all transitive dependents
            self.skip_transitive_dependents(task_path);
        }

        // Enqueue newly ready tasks
        if !newly_ready.is_empty() {
            let mut queue = self
                .ready_queue
                .lock()
                .expect("ready_queue lock should not be poisoned");
            for path in &newly_ready {
                let priority = self
                    .tasks
                    .get(path)
                    .map(|t| t.priority)
                    .unwrap_or(TaskPriority::Normal);
                let seq_num = self.sequence.fetch_add(1, Ordering::Relaxed);
                queue.push(PrioritizedTask {
                    task_path: path.clone(),
                    priority,
                    critical_path_remaining_ms: 0,
                    sequence: seq_num,
                });
            }
            let depth = queue.len() as u64;
            let mut max = self.max_queue_depth.load(Ordering::Relaxed);
            if depth > max {
                max = depth;
                self.max_queue_depth.store(max, Ordering::Relaxed);
            }
            self.work_available.notify_waiters();
        }
    }

    /// Skip all transitive dependents of a failed task (BFS).
    fn skip_transitive_dependents(&self, failed_task: &str) {
        let mut to_visit: Vec<String> = Vec::with_capacity(16);
        if let Some(task) = self.tasks.get(failed_task) {
            to_visit.extend(task.dependents.iter().cloned());
        }

        let mut visited = HashSet::with_capacity(16);
        while let Some(path) = to_visit.pop() {
            if visited.contains(&path) {
                continue;
            }
            visited.insert(path.clone());

            // Get dependents first (immutable borrow), then update status (mutable borrow)
            let next_dependents = self
                .tasks
                .get(&path)
                .map(|t| t.dependents.clone())
                .unwrap_or_default();

            if let Some(mut task) = self.tasks.get_mut(&path) {
                if task.status == TaskStatus::Pending || task.status == TaskStatus::Ready {
                    task.status = TaskStatus::Skipped;
                }
            }

            to_visit.extend(next_dependents);
        }

        // Clean up ready queue (remove skipped tasks)
        if let Ok(mut queue) = self.ready_queue.lock() {
            let old_queue = std::mem::take(&mut *queue);
            for task in old_queue {
                if !visited.contains(&task.task_path) {
                    queue.push(task);
                }
            }
        }
    }

    /// Cancel the current build. All pending tasks become skipped.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
        self.work_available.notify_waiters();

        // Mark all pending/ready tasks as skipped
        for mut task in self.tasks.iter_mut() {
            if matches!(task.status, TaskStatus::Pending | TaskStatus::Ready) {
                task.status = TaskStatus::Skipped;
            }
        }

        if let Ok(mut queue) = self.ready_queue.lock() {
            queue.clear();
        }
    }

    /// Check if the build is complete (all tasks in terminal state).
    pub fn is_complete(&self) -> bool {
        let executing = self.executing.is_empty();
        let has_pending = self.tasks.iter().any(|t| {
            matches!(
                t.status,
                TaskStatus::Pending | TaskStatus::Ready | TaskStatus::Executing
            )
        });
        executing && !has_pending
    }

    /// Check if cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    /// Get current scheduler statistics.
    pub fn stats(&self) -> SchedulerStats {
        SchedulerStats {
            tasks_scheduled: self.tasks_scheduled.load(Ordering::Relaxed),
            tasks_completed: self.tasks_completed.load(Ordering::Relaxed),
            tasks_stolen: self.tasks_stolen.load(Ordering::Relaxed),
            total_wait_time_ms: self.total_wait_ms.load(Ordering::Relaxed),
            max_queue_depth: self.max_queue_depth.load(Ordering::Relaxed),
            active_workers: self.executing.len() as u32,
            current_memory_usage: self.current_memory.load(Ordering::Relaxed),
        }
    }

    /// Get the number of tasks currently being executed.
    pub fn executing_count(&self) -> usize {
        self.executing.len()
    }

    /// Get the number of tasks in the ready queue.
    pub fn ready_count(&self) -> usize {
        self.ready_queue
            .lock()
            .expect("ready_queue lock should not be poisoned")
            .len()
    }

    /// Get total task count.
    pub fn total_tasks(&self) -> usize {
        self.tasks.len()
    }

    /// Get task count by status.
    pub fn status_counts(&self) -> (usize, usize, usize, usize, usize, usize) {
        let mut pending = 0;
        let mut ready = 0;
        let mut executing = 0;
        let mut succeeded = 0;
        let mut failed = 0;
        let mut skipped = 0;

        for task in self.tasks.iter() {
            match task.status {
                TaskStatus::Pending => pending += 1,
                TaskStatus::Ready => ready += 1,
                TaskStatus::Executing => executing += 1,
                TaskStatus::Succeeded => succeeded += 1,
                TaskStatus::Failed => failed += 1,
                TaskStatus::Skipped => skipped += 1,
            }
        }

        (pending, ready, executing, succeeded, failed, skipped)
    }

    /// Wait until work is available or timeout.
    pub async fn wait_for_work(&self, timeout_ms: u64) -> bool {
        if timeout_ms == 0 {
            self.work_available.notified().await;
            true
        } else {
            tokio::time::timeout(
                std::time::Duration::from_millis(timeout_ms),
                self.work_available.notified(),
            )
            .await
            .is_ok()
        }
    }

    /// Get the max parallelism configured.
    pub fn max_parallelism(&self) -> usize {
        self.config.max_parallelism
    }
}

/// A task that was stolen from the queue by a worker.
#[derive(Debug, Clone)]
pub struct StolenTask {
    pub task_path: String,
    pub priority: TaskPriority,
    pub resource_estimate: ResourceEstimate,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(max_parallel: usize) -> SchedulerConfig {
        SchedulerConfig {
            max_parallelism: max_parallel,
            memory_budget: 0,
            cpu_aware: false,
            worker_count: max_parallel,
        }
    }

    #[tokio::test]
    async fn test_register_and_build_graph() {
        let scheduler = ParallelScheduler::with_config(make_config(4));

        scheduler.register_task(
            ":a",
            vec![],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.register_task(
            ":b",
            vec![":a".to_string()],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.register_task(
            ":c",
            vec![":a".to_string()],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.register_task(
            ":d",
            vec![":b".to_string(), ":c".to_string()],
            TaskPriority::Critical,
            ResourceEstimate::default(),
        );

        scheduler.build_graph();

        // :a should be ready immediately
        let (pending, ready, _, _, _, _) = scheduler.status_counts();
        assert_eq!(pending, 3); // :b, :c, :d
        assert_eq!(ready, 1); // :a
        assert_eq!(scheduler.total_tasks(), 4);
    }

    #[tokio::test]
    async fn test_steal_ready_task() {
        let scheduler = ParallelScheduler::with_config(make_config(4));

        scheduler.register_task(
            ":a",
            vec![],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.build_graph();

        let stolen = scheduler.try_steal().await;
        assert!(stolen.is_some());
        assert_eq!(stolen.unwrap().task_path, ":a");

        // No more tasks available (b depends on a)
        let stolen2 = scheduler.try_steal().await;
        assert!(stolen2.is_none());
    }

    #[tokio::test]
    async fn test_complete_unblocks_dependents() {
        let scheduler = ParallelScheduler::with_config(make_config(4));

        scheduler.register_task(
            ":a",
            vec![],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.register_task(
            ":b",
            vec![":a".to_string()],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.build_graph();

        // Steal :a
        let stolen = scheduler.try_steal().await.unwrap();
        assert_eq!(stolen.task_path, ":a");

        // Complete :a
        scheduler.complete_task(":a", 100, true).await;

        // Now :b should be stealable
        let stolen2 = scheduler.try_steal().await;
        assert!(stolen2.is_some());
        assert_eq!(stolen2.unwrap().task_path, ":b");
    }

    #[tokio::test]
    async fn test_complete_unblocks_multiple_dependents() {
        let scheduler = ParallelScheduler::with_config(make_config(4));

        scheduler.register_task(
            ":a",
            vec![],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.register_task(
            ":b",
            vec![":a".to_string()],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.register_task(
            ":c",
            vec![":a".to_string()],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.build_graph();

        // Steal :a
        scheduler.try_steal().await.unwrap();

        // Complete :a — should unblock both :b and :c
        scheduler.complete_task(":a", 100, true).await;

        let stolen_b = scheduler.try_steal().await.unwrap();
        let stolen_c = scheduler.try_steal().await.unwrap();

        assert!(vec![stolen_b.task_path.clone(), stolen_c.task_path.clone()]
            .contains(&":b".to_string()));
        assert!(vec![stolen_b.task_path, stolen_c.task_path].contains(&":c".to_string()));
    }

    #[tokio::test]
    async fn test_failure_skips_dependents() {
        let scheduler = ParallelScheduler::with_config(make_config(4));

        scheduler.register_task(
            ":a",
            vec![],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.register_task(
            ":b",
            vec![":a".to_string()],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.register_task(
            ":c",
            vec![":b".to_string()],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.build_graph();

        scheduler.try_steal().await.unwrap();
        scheduler.complete_task(":a", 50, false).await;

        // :b and :c should be skipped
        let (pending, ready, _, _, _, skipped) = scheduler.status_counts();
        assert_eq!(pending, 0);
        assert_eq!(ready, 0);
        assert_eq!(skipped, 2);

        assert!(scheduler.is_complete());
    }

    #[tokio::test]
    async fn test_parallelism_limit() {
        let scheduler = ParallelScheduler::with_config(make_config(2));

        // 3 independent tasks but only 2 slots
        scheduler.register_task(
            ":a",
            vec![],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.register_task(
            ":b",
            vec![],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.register_task(
            ":c",
            vec![],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.build_graph();

        let s1 = scheduler.try_steal().await.unwrap();
        let _s2 = scheduler.try_steal().await.unwrap();
        assert_eq!(scheduler.executing_count(), 2);

        // Third task should be blocked by parallelism limit
        let s3 = scheduler.try_steal().await;
        assert!(s3.is_none());

        // Complete one task
        scheduler.complete_task(&s1.task_path, 100, true).await;

        // Now third task should be available
        let s3 = scheduler.try_steal().await;
        assert!(s3.is_some());
    }

    #[tokio::test]
    async fn test_critical_path_priority() {
        let scheduler = ParallelScheduler::with_config(make_config(4));

        scheduler.register_task(
            ":fast",
            vec![],
            TaskPriority::Normal,
            ResourceEstimate {
                cpu_weight: 0.5,
                memory_bytes: 0,
                estimated_ms: 10,
            },
        );
        scheduler.register_task(
            ":slow",
            vec![],
            TaskPriority::Critical,
            ResourceEstimate {
                cpu_weight: 1.0,
                memory_bytes: 0,
                estimated_ms: 1000,
            },
        );
        scheduler.build_graph();

        // Critical path task should be stolen first
        let first = scheduler.try_steal().await.unwrap();
        assert_eq!(first.task_path, ":slow");
        assert_eq!(first.priority, TaskPriority::Critical);

        let second = scheduler.try_steal().await.unwrap();
        assert_eq!(second.task_path, ":fast");
    }

    #[tokio::test]
    async fn test_cancel() {
        let scheduler = ParallelScheduler::with_config(make_config(4));

        scheduler.register_task(
            ":a",
            vec![],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.register_task(
            ":b",
            vec![":a".to_string()],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.register_task(
            ":c",
            vec![],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.build_graph();

        scheduler.cancel();

        assert!(scheduler.is_cancelled());

        // No tasks should be stealable
        let stolen = scheduler.try_steal().await;
        assert!(stolen.is_none());

        // Pending tasks should be skipped
        let (_, _, _, _, _, skipped) = scheduler.status_counts();
        assert_eq!(skipped, 3);
    }

    #[tokio::test]
    async fn test_complex_dag() {
        let scheduler = ParallelScheduler::with_config(make_config(4));

        // Diamond: a -> b, a -> c, b -> d, c -> d
        scheduler.register_task(
            ":a",
            vec![],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.register_task(
            ":b",
            vec![":a".to_string()],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.register_task(
            ":c",
            vec![":a".to_string()],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.register_task(
            ":d",
            vec![":b".to_string(), ":c".to_string()],
            TaskPriority::Critical,
            ResourceEstimate::default(),
        );
        scheduler.build_graph();

        // Step 1: Steal :a
        let s = scheduler.try_steal().await.unwrap();
        assert_eq!(s.task_path, ":a");
        scheduler.complete_task(":a", 100, true).await;

        // Step 2: Steal :b and :c (both ready)
        let s1 = scheduler.try_steal().await.unwrap();
        let s2 = scheduler.try_steal().await.unwrap();
        let paths = vec![s1.task_path.clone(), s2.task_path.clone()];
        assert!(paths.contains(&":b".to_string()));
        assert!(paths.contains(&":c".to_string()));

        // Step 3: Complete :b (:d not ready yet)
        scheduler.complete_task(&paths[0], 50, true).await;
        assert!(scheduler.try_steal().await.is_none()); // :c still executing

        // Step 4: Complete :c
        scheduler.complete_task(&paths[1], 75, true).await;

        // Step 5: :d should be ready
        let s3 = scheduler.try_steal().await.unwrap();
        assert_eq!(s3.task_path, ":d");
        assert_eq!(s3.priority, TaskPriority::Critical);

        scheduler.complete_task(":d", 200, true).await;

        assert!(scheduler.is_complete());
        let (_, _, _, succeeded, _, _) = scheduler.status_counts();
        assert_eq!(succeeded, 4);
    }

    #[tokio::test]
    async fn test_is_complete() {
        let scheduler = ParallelScheduler::with_config(make_config(4));

        scheduler.register_task(
            ":a",
            vec![],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.build_graph();

        assert!(!scheduler.is_complete());

        let s = scheduler.try_steal().await.unwrap();
        assert!(!scheduler.is_complete()); // still executing

        scheduler.complete_task(&s.task_path, 10, true).await;
        assert!(scheduler.is_complete());
    }

    #[tokio::test]
    async fn test_stats() {
        let scheduler = ParallelScheduler::with_config(make_config(4));

        scheduler.register_task(
            ":a",
            vec![],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.register_task(
            ":b",
            vec![":a".to_string()],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.build_graph();

        let s = scheduler.try_steal().await.unwrap();
        scheduler.complete_task(&s.task_path, 50, true).await;
        scheduler.try_steal().await.unwrap();

        let stats = scheduler.stats();
        assert_eq!(stats.tasks_scheduled, 2);
        assert_eq!(stats.tasks_completed, 1);
        assert!(stats.active_workers >= 1);
    }

    #[tokio::test]
    async fn test_no_tasks() {
        let scheduler = ParallelScheduler::with_config(make_config(4));
        scheduler.build_graph();

        assert!(scheduler.is_complete());
        assert!(scheduler.try_steal().await.is_none());
    }

    #[tokio::test]
    async fn test_empty_build_complete() {
        let scheduler = ParallelScheduler::with_config(make_config(4));
        assert!(scheduler.is_complete());
        assert_eq!(scheduler.total_tasks(), 0);
    }

    #[test]
    fn test_scheduler_config_cpu_aware() {
        let config = SchedulerConfig::default();
        assert!(config.cpu_aware);
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        assert!(config.max_parallelism <= cpus);
    }

    #[test]
    fn test_task_priority_ordering() {
        assert!(TaskPriority::Critical < TaskPriority::Normal);
        assert!(TaskPriority::Normal < TaskPriority::Low);
    }

    #[test]
    fn test_prioritized_task_ordering() {
        let t1 = PrioritizedTask {
            task_path: ":critical".to_string(),
            priority: TaskPriority::Critical,
            critical_path_remaining_ms: 1000,
            sequence: 2,
        };
        let t2 = PrioritizedTask {
            task_path: ":normal".to_string(),
            priority: TaskPriority::Normal,
            critical_path_remaining_ms: 5000,
            sequence: 1,
        };

        // BinaryHeap is a max-heap. Our Ord makes Critical > Normal so Critical pops first.
        assert!(t1 > t2); // Critical has higher scheduling priority
    }

    #[tokio::test]
    async fn test_ready_count() {
        let scheduler = ParallelScheduler::with_config(make_config(4));

        scheduler.register_task(
            ":a",
            vec![],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.register_task(
            ":b",
            vec![],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.build_graph();

        assert_eq!(scheduler.ready_count(), 2);

        scheduler.try_steal().await.unwrap();
        assert_eq!(scheduler.ready_count(), 1);
    }

    #[tokio::test]
    async fn test_max_parallelism() {
        let config = make_config(2);
        let scheduler = ParallelScheduler::with_config(config);
        assert_eq!(scheduler.max_parallelism(), 2);
    }

    #[tokio::test]
    async fn test_multiple_roots() {
        let scheduler = ParallelScheduler::with_config(make_config(4));

        // All 3 tasks are independent roots
        scheduler.register_task(
            ":a",
            vec![],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.register_task(
            ":b",
            vec![],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.register_task(
            ":c",
            vec![],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.build_graph();

        let s1 = scheduler.try_steal().await.unwrap();
        let s2 = scheduler.try_steal().await.unwrap();
        let s3 = scheduler.try_steal().await.unwrap();

        let paths = vec![s1.task_path, s2.task_path, s3.task_path];
        assert!(paths.contains(&":a".to_string()));
        assert!(paths.contains(&":b".to_string()));
        assert!(paths.contains(&":c".to_string()));
    }

    #[tokio::test]
    async fn test_failure_mid_diamond() {
        let scheduler = ParallelScheduler::with_config(make_config(4));

        // a -> b -> d, a -> c -> d
        scheduler.register_task(
            ":a",
            vec![],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.register_task(
            ":b",
            vec![":a".to_string()],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.register_task(
            ":c",
            vec![":a".to_string()],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.register_task(
            ":d",
            vec![":b".to_string(), ":c".to_string()],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        scheduler.build_graph();

        // Complete :a
        let sa = scheduler.try_steal().await.unwrap();
        assert_eq!(sa.task_path, ":a");
        scheduler.complete_task(":a", 10, true).await;

        // Steal one and fail it (either :b or :c, whichever comes first)
        let first_stolen = scheduler.try_steal().await.unwrap();
        assert!(
            first_stolen.task_path == ":b" || first_stolen.task_path == ":c",
            "expected :b or :c, got {}",
            first_stolen.task_path
        );
        scheduler
            .complete_task(&first_stolen.task_path, 10, false)
            .await;

        // :d should be skipped (depends on failed task)
        let (_, _, _, _, _, skipped) = scheduler.status_counts();
        assert_eq!(skipped, 1); // :d

        // Complete the other normally
        let second_stolen = scheduler.try_steal().await.unwrap();
        assert!(
            second_stolen.task_path == ":b" || second_stolen.task_path == ":c",
            "expected :b or :c, got {}",
            second_stolen.task_path
        );
        assert_ne!(first_stolen.task_path, second_stolen.task_path);
        scheduler
            .complete_task(&second_stolen.task_path, 10, true)
            .await;

        assert!(scheduler.is_complete());
    }

    #[tokio::test]
    async fn test_deep_chain() {
        let scheduler = ParallelScheduler::with_config(make_config(4));

        // 10-task chain: t0 -> t1 -> ... -> t9
        for i in 0..10 {
            let path = format!(":t{}", i);
            let deps = if i > 0 {
                vec![format!(":t{}", i - 1)]
            } else {
                vec![]
            };
            scheduler.register_task(
                &path,
                deps,
                TaskPriority::Normal,
                ResourceEstimate::default(),
            );
        }
        scheduler.build_graph();

        // Execute in order
        for i in 0..10 {
            let path = format!(":t{}", i);
            let stolen = scheduler.try_steal().await.unwrap();
            assert_eq!(stolen.task_path, path);
            scheduler.complete_task(&path, 10, true).await;
        }

        assert!(scheduler.is_complete());
        let (_, _, _, succeeded, _, _) = scheduler.status_counts();
        assert_eq!(succeeded, 10);
    }

    #[tokio::test]
    async fn test_wide_fan_out() {
        let scheduler = ParallelScheduler::with_config(make_config(8));

        // 1 root -> 7 dependents
        scheduler.register_task(
            ":root",
            vec![],
            TaskPriority::Normal,
            ResourceEstimate::default(),
        );
        for i in 0..7 {
            scheduler.register_task(
                &format!(":dep{}", i),
                vec![":root".to_string()],
                TaskPriority::Normal,
                ResourceEstimate::default(),
            );
        }
        scheduler.build_graph();

        // Steal root
        let s = scheduler.try_steal().await.unwrap();
        assert_eq!(s.task_path, ":root");
        scheduler.complete_task(":root", 10, true).await;

        // All 7 dependents should be ready
        assert_eq!(scheduler.ready_count(), 7);

        // Steal all 7
        let mut stolen_paths = Vec::new();
        for _ in 0..7 {
            let stolen = scheduler.try_steal().await.unwrap();
            stolen_paths.push(stolen.task_path);
        }
        assert_eq!(stolen_paths.len(), 7);
    }
}
