# Tooling API Substrate Bridge

## Communication Path

```
IDE (IntelliJ/Eclipse/VS Code)
  │
  ▼
Tooling API (Java) ── GradleConnector, BuildLauncher, ProjectConnection
  │
  ▼
JVM Bridge Clients (platforms/core-execution/rust-bridge/)
  │
  ▼  gRPC over Unix sockets
  │
Rust Daemon (substrate/)
  │
  ▼  JvmHostService (reverse RPC)
  │
JVM Compatibility Host (platforms/core-execution/rust-bridge/jvmhost/)
```

## Current State

The Rust daemon does **not** directly serve Tooling API requests. All IDE communication goes through the JVM:
- IDE calls Tooling API → JVM processes request → may call Rust via bridge → returns result to IDE
- The `IdeModelService` proto defines the contract for future Rust-native model serving

## Key Tooling API Classes

- `GradleConnector` — entry point for IDE connections
- `BuildLauncher` — configure and launch builds
- `BuildController` — receive build events and model callbacks
- `ProjectConnection` — query project model
- `ProgressListener` — receive progress events
- `ModelBuilder<T>` — request typed project models (EclipseProject, IdeaProject)

## Future Direction

1. Cache project model data in Rust (`build_model_cache.rs`) populated from JVM during bootstrap
2. Serve IDE model queries from cache without JVM round-trip
3. Support incremental model updates via file watch events
