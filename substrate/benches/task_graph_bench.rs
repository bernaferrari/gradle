use criterion::{criterion_group, criterion_main, Criterion};
use gradle_substrate_daemon::proto::{
    task_graph_service_server::TaskGraphService, RegisterTaskRequest, ResolveExecutionPlanRequest,
    TaskFinishedRequest,
};
use gradle_substrate_daemon::server::task_graph::TaskGraphServiceImpl;
use tonic::Request;

fn bench_register_1000_tasks(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let svc = TaskGraphServiceImpl::new();

    // Pre-generate task data
    let tasks: Vec<RegisterTaskRequest> = (0..1000)
        .map(|i| {
            let deps = if i > 0 {
                vec![format!(":task_{}", i - 1)]
            } else {
                vec![]
            };
            RegisterTaskRequest {
                build_id: "bench-build".to_string(),
                task_path: format!(":task_{}", i),
                depends_on: deps,
                task_type: "FakeTask".to_string(),
                input_files: vec![format!("src/Task{}.java", i)],
                should_execute: true,
            }
        })
        .collect();

    c.bench_function("register_1000_tasks_chain", |b| {
        b.iter(|| {
            rt.block_on(async {
                for task in &tasks {
                    let _ = svc.register_task(Request::new(task.clone())).await.unwrap();
                }
            })
        })
    });
}

fn bench_resolve_execution_plan(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let svc = TaskGraphServiceImpl::new();

    // Seed 500 tasks in a diamond dependency pattern
    rt.block_on(async {
        for i in 0..100 {
            let _ = svc
                .register_task(Request::new(RegisterTaskRequest {
                    build_id: "bench-build".to_string(),
                    task_path: format!(":left_{}", i),
                    depends_on: if i > 0 {
                        vec![format!(":left_{}", i - 1)]
                    } else {
                        vec![]
                    },
                    task_type: "FakeTask".to_string(),
                    input_files: vec![],
                    should_execute: true,
                }))
                .await;
            let _ = svc
                .register_task(Request::new(RegisterTaskRequest {
                    build_id: "bench-build".to_string(),
                    task_path: format!(":right_{}", i),
                    depends_on: if i > 0 {
                        vec![format!(":right_{}", i - 1)]
                    } else {
                        vec![]
                    },
                    task_type: "FakeTask".to_string(),
                    input_files: vec![],
                    should_execute: true,
                }))
                .await;
        }
        let _ = svc
            .register_task(Request::new(RegisterTaskRequest {
                build_id: "bench-build".to_string(),
                task_path: ":merge".to_string(),
                depends_on: vec![":left_99".to_string(), ":right_99".to_string()],
                task_type: "FakeTask".to_string(),
                input_files: vec![],
                should_execute: true,
            }))
            .await;
    });

    c.bench_function("resolve_execution_plan_200_tasks_diamond", |b| {
        b.iter(|| {
            rt.block_on(async {
                svc.resolve_execution_plan(Request::new(ResolveExecutionPlanRequest {
                    build_id: "bench-build".to_string(),
                }))
                .await
                .unwrap()
            })
        })
    });
}

fn bench_task_finished(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let svc = TaskGraphServiceImpl::new();

    // Seed 100 tasks
    rt.block_on(async {
        for i in 0..100 {
            let _ = svc
                .register_task(Request::new(RegisterTaskRequest {
                    build_id: "bench-build".to_string(),
                    task_path: format!(":task_{}", i),
                    depends_on: vec![],
                    task_type: "FakeTask".to_string(),
                    input_files: vec![],
                    should_execute: true,
                }))
                .await;
        }
    });

    // Pre-generate finish requests
    let finishes: Vec<TaskFinishedRequest> = (0..100)
        .map(|i| TaskFinishedRequest {
            build_id: "bench-build".to_string(),
            task_path: format!(":task_{}", i),
            duration_ms: 50 + i,
            success: true,
            outcome: "SUCCESS".to_string(),
        })
        .collect();

    c.bench_function("task_finished_100_tasks", |b| {
        b.iter(|| {
            rt.block_on(async {
                for finish in &finishes {
                    let _ = svc
                        .task_finished(Request::new(finish.clone()))
                        .await
                        .unwrap();
                }
            })
        })
    });
}

criterion_group!(
    benches,
    bench_register_1000_tasks,
    bench_resolve_execution_plan,
    bench_task_finished
);
criterion_main!(benches);
