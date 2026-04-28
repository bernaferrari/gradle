# Parity Status

## Module

- `module_id`: `substrate-daemon`
- `module_path`: `substrate`
- `status`: `native-kernel`

## Supported

- Rust daemon protocol services compile and run in-process tests.
- Hash compatibility suite validates cross-language digest behavior.
- Strict stabilization script validates proto sync and daemon smoke startup.
- Captured JVM-host build-plan shadows can drive a small native Rust Java
  lifecycle with JVM fallback disabled: `JavaCompile`, `ProcessResources`
  lowered to `Copy`, no-action `classes` lowered to `Lifecycle`, and `Jar`.
- Real Gradle task execution has an opt-in authoritative gate:
  `org.gradle.rust.substrate.runbuild.authoritative=true` executes the selected
  build-plan shadow through Rust `RunBuild` and skips Gradle's JVM task executor
  only when Rust reports exactly the scheduled task count with zero JVM forwards.
- The checked-in Java library, Java application, Java multi-project,
  resource-expansion, standalone Sync, and CopySpec pattern corpus builds
  complete through that explicit gate with `target/debug/gradle-substrate-daemon`.
- A separate networked JUnit corpus build proves native `TestExec` lowering for
  a non-empty JUnit Platform test task when the test runtime contains the JUnit
  Platform ConsoleLauncher.
- Native dependency resolution handles inherited Maven exclusions per dependency
  edge, so one dependency's exclusions no longer remove sibling dependencies.
- Native dependency resolution preserves Maven dependency scopes on resolved
  nodes and filters compile/runtime/test target scopes recursively instead of
  only filtering top-level requested dependencies.
- Native dependency resolution builds artifact URLs from classifier and
  extension metadata instead of assuming every artifact is an unclassified JAR.
- Archive tasks (`Jar`, `Zip`, `War`, `Ear`, `Tar`) can lower to native Rust
  archive execution when the task model provides input paths and an output
  archive path. ZIP-compatible tasks emit ZIP-compatible archives; `Tar` emits
  native POSIX tar streams and supports gzip-compressed output.
- Native ZIP-compatible archive entries use a fixed DOS timestamp and stable
  entry ordering for reproducible Rust-side archive bytes.
- Native JAR manifest generation emits sorted custom attributes and wraps long
  manifest lines to JAR-compatible continuation lines.
- Native file operations cover recursive `Copy` directory sources and
  multi-source `Sync` orphan deletion using the union of all source roots.
- Native `ProcessResources`/`Copy`/`Sync` execution can expand declared scalar
  task input properties in Gradle-style `$name` and `${name}` templates,
  validated by content-hash corpus comparison for resource processing and Sync.
- Native `Copy` and `Sync` honor captured CopySpec include/exclude patterns,
  including `**`, `*`, `?`, and case-sensitivity flags for relative paths.
- Native `Copy` and `Sync` honor captured CopySpec `includeEmptyDirs`
  behavior for creating empty directory trees.
- Native `Copy` and `Sync` honor basic duplicate destination strategies:
  `INCLUDE`/default overwrites, `EXCLUDE` keeps the first file, and `FAIL`
  reports an error.
- Native ZIP-compatible and TAR archive tasks honor captured duplicate
  destination strategies for `INCLUDE`, `EXCLUDE`, and `FAIL`.
- Native ZIP-compatible and TAR archive tasks honor captured CopySpec
  `includeEmptyDirs` behavior for explicit directory entries.

## Gaps

- Full dependency resolution semantics are not yet parity-complete.
- Real Gradle invocation does not use Rust as the default executor yet; the
  authoritative build-work gate is intentionally opt-in and fail-closed while
  coverage expands beyond the checked-in Java corpus.
- Kotlin/Groovy DSL evaluation and legacy plugin execution remain JVM-host
  compatibility islands.
- More task types need native-ready contract capture before broad no-fallback
  execution is realistic, especially arbitrary copy filters/actions, nested
  CopySpec trees, Gradle archive metadata edge cases, permission preservation,
  symlink copy semantics, and non-gzip tar compression variants.

## Validation

- `cargo check -p gradle-substrate-daemon`
- `cargo test -p gradle-substrate-daemon --test hash_compatibility_test`
- `cargo test -p gradle-substrate-daemon --test build_plan_shadow_test refreshed_native_ready_shadow_plan_runs_java_lifecycle_without_jvm_fallback -- --exact`
- `./gradlew :core:test --tests org.gradle.execution.RustAuthoritativeBuildExecutionActionTest -x :distributions-core:generateLicenseFile`
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --verbose`
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/external-manifest.json --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --verbose`
- `./tools/stabilization/run_strict_stabilization.sh quick`
- `./tools/demo/rust_substrate_demo.sh --quick`

## Next Sync Actions

1. Expand differential corpus coverage for external dependency and richer
   task-graph semantics.
2. Add authoritative coverage for resource filtering, richer archive specs, and
   copy/sync edge cases.
3. Add native-ready contracts for richer `Copy`/`Sync` specs and `Test` enough
   to complete broader Java library/application lifecycles.
4. Reduce bridge source exclusions as APIs are stabilized.
5. Track upstream commit synchronization in this file for each parity push.
