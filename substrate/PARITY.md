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
- The checked-in offline corpus covers Java library, Java application,
  Java multi-project, resource expansion, JavaCompile options, standalone
  Copy/Sync transforms, CopySpec duplicates, nested Copy/Sync and Zip/Tar
  CopySpec mappings, Zip/Tar/War/Ear archive tasks, a simple Exec task, and a
  JavaExec task, Javadoc task, and an OSS-style Java library slice with
  sources JAR/Javadoc/report outputs through that explicit gate with
  `target/debug/gradle-substrate-daemon`.
- A separate networked JUnit corpus build proves native `TestExec` lowering for
  a non-empty JUnit Platform test task, including include/exclude tag capture,
  when the test runtime contains the JUnit Platform ConsoleLauncher.
- Native `TestExec` lowering captures a single Gradle include test filter and
  JUnit Platform include/exclude tags, and fails closed for unsupported filter
  shapes rather than approximating them.
- Native `JavaExec` lowering captures Java home, classpath, main class, JVM
  args, application args, working directory, and ignore-exit-value from the JVM
  task model, and fails closed unless classpath and main class are present.
- Native `Javadoc` lowering captures Java home, source files, classpath,
  destination directory, title, encoding, max memory, and timestamp behavior,
  and lowers only when source files and declared outputs are present.
- The native-ready default gate
  `org.gradle.rust.substrate.runbuild.native-ready-default=true` tries Rust
  RunBuild first and delegates back to JVM execution when the selected plan is
  not fully native-ready. The offline corpus passes through this gate at 21/21.
- `testing/corpus/unsupported-manifest.json` tracks work that must remain
  outside approximate native execution until a complete contract exists.
- Native dependency resolution handles inherited Maven exclusions per dependency
  edge, so one dependency's exclusions no longer remove sibling dependencies.
- Native dependency resolution preserves Maven dependency scopes on resolved
  nodes and filters compile/runtime/test target scopes recursively instead of
  only filtering top-level requested dependencies.
- Native dependency resolution builds artifact URLs from classifier and
  extension metadata instead of assuming every artifact is an unclassified JAR.
- Native dependency transport can stream artifact bytes over HTTP from a
  repository-derived Maven artifact URL, with retry/error handling in the Rust
  dependency-resolution service.
- Native dependency transport now persists downloaded Maven artifacts into the
  Rust artifact store, writes `.sha256` sidecars, validates cold and warm cache
  hits against requested SHA-256 values, and can verify checksums from persisted
  artifacts.
- The Gradle dependency shadow listener has an explicit artifact mirror mode
  (`org.gradle.rust.substrate.dependency.mirror.artifacts=true`) that observes
  real resolved external module JARs, computes SHA-256 in the JVM bridge, and
  registers those artifacts into the Rust artifact store. The mode is opt-in
  because querying artifacts from a resolution listener can force artifact
  downloads earlier than a graph-only resolution would.
- Archive tasks (`Jar`, `Zip`, `War`, `Ear`, `Tar`) can lower to native Rust
  archive execution when the task model provides input paths and an output
  archive path. ZIP-compatible tasks emit ZIP-compatible archives; `Tar` emits
  native POSIX tar streams and supports gzip/bzip2-compressed output.
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
- Native `Copy` and `Sync` honor captured root CopySpec file and directory
  permissions on Unix when Gradle exposes `filePermissions`/`dirPermissions`.
- Native `Copy` and `Sync` honor basic duplicate destination strategies:
  `INCLUDE`/default overwrites, `EXCLUDE` keeps the first file, and `FAIL`
  reports an error.
- Native `Copy` and `Sync` can consume JVM-captured nested CopySpec file
  mappings for child `into(...)` destinations instead of flattening every
  source at the output root.
- Native ZIP-compatible and TAR archive tasks honor captured duplicate
  destination strategies for `INCLUDE`, `EXCLUDE`, and `FAIL`.
- Native ZIP-compatible and TAR archive tasks honor captured CopySpec
  `includeEmptyDirs` behavior for explicit directory entries.
- Native ZIP-compatible and TAR archive tasks honor captured root CopySpec file
  and directory permissions in archive entry metadata.
- Native ZIP-compatible and TAR archive tasks can consume JVM-captured nested
  CopySpec file mappings for child `into(...)` destinations; the corpus gate
  now compares archive entry inventories, not just archive output filenames.
- Native file-transform and archive lowering fails closed when the JVM bridge
  detects symbolic links in task inputs. This avoids silently approximating
  symlink traversal/copy semantics until they are modeled explicitly.

## Gaps

- Full dependency resolution semantics are not yet parity-complete.
- Rust does not yet short-circuit Gradle's artifact resolver from the Rust
  artifact store; the current bridge integration mirrors real Gradle-resolved
  artifacts into Rust so a later resolver replacement has trustworthy data.
- Real Gradle invocation does not use Rust as the unconditional default executor
  yet; the authoritative build-work gate is intentionally opt-in and fail-closed
  while the native-ready default gate delegates on incomplete plans.
- Kotlin/Groovy DSL evaluation and legacy plugin execution remain JVM-host
  compatibility islands.
- More task types need native-ready contract capture before broad no-fallback
  execution is realistic, especially arbitrary copy filters/actions beyond
  direct expand, Gradle archive metadata edge cases beyond entry inventory, and
  native symlink copy/archive semantics.

## Validation

- `cargo check -p gradle-substrate-daemon`
- `cargo test -p gradle-substrate-daemon --test hash_compatibility_test`
- `cargo test -p gradle-substrate-daemon --test build_plan_shadow_test refreshed_native_ready_shadow_plan_runs_java_lifecycle_without_jvm_fallback -- --exact`
- `./gradlew :core:test --tests org.gradle.execution.RustAuthoritativeBuildExecutionActionTest -x :distributions-core:generateLicenseFile`
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --verbose`
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --daemon-binary target/debug/gradle-substrate-daemon --runbuild-native-ready-default --tasks clean build --timeout 300 --output-dir build/corpus-native-ready-default-21`
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/external-manifest.json --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --verbose`
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/unsupported-manifest.json --contract-only --output-dir build/corpus-contract-unsupported`
- `cargo test -p gradle-substrate-daemon download_artifact -- --nocapture`
- `./gradlew :rust-bridge:test --tests org.gradle.internal.rustbridge.dependency.DependencyResolutionShadowListenerTest`
- `python3 tools/performance/rust_substrate_perf_report.py build/corpus-authoritative-21/corpus_summary.json --output build/corpus-authoritative-21/performance.md`
- `./tools/stabilization/run_strict_stabilization.sh quick`
- `./tools/demo/rust_substrate_demo.sh --quick`

## Next Sync Actions

1. Expand differential corpus coverage for external dependency and richer
   task-graph semantics.
2. Expand the unsupported corpus from custom JVM work into symlink CopySpec,
   arbitrary CopySpec actions, unsupported copy filters, and unsupported test
   filters as those signals become observable in the bridge.
3. Add native-ready contracts for richer `Copy`/`Sync` specs and the next common
   process task after `Javadoc`.
4. Reduce bridge source exclusions as APIs are stabilized.
5. Track upstream commit synchronization in this file for each parity push.
