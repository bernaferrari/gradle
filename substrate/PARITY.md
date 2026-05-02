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
- Rust `RunBuild` dispatch now uses a native ready-task priority queue keyed by
  remaining critical-path duration instead of FIFO order, so independent ready
  tasks on the longest path are claimed first. Filtered task selections also
  treat dependencies outside the selected graph as absent when deciding initial
  readiness.
- Rust `RunBuild` now materializes transitive downstream skips in its response
  task details after a task failure, so authoritative graph results expose
  failed and skipped work consistently instead of hiding skipped nodes only in
  internal build state.
- Installed Gradle-under-test runs now use loopback TCP for the Rust daemon and
  JVM host bridge when Unix-domain socket transports are unavailable, and the
  authoritative executor refreshes the selected build-plan shadow directly from
  the finalized Gradle task plan before invoking Rust `RunBuild`.
- Authoritative Gradle invocations now register an eager task-graph listener
  through Rust bridge core services and capture native-ready selected task
  contracts at Gradle task-graph population. When those task paths exactly match
  the finalized execution plan, Rust reuses the early contracts and executes the
  finalized DAG without depending on late build-script parsing.
- The JVM bridge now persists the loopback Rust daemon endpoint under the
  substrate state directory and reconnects on later Gradle invocations. The
  endpoint is written atomically, records daemon binary path/mtime/size, and is
  ignored when the daemon binary identity changes, so local rebuilds do not
  accidentally attach to an older substrate daemon.
- The checked-in offline corpus covers Java library, Java application,
  Java multi-project, resource expansion, JavaCompile options, standalone
  Copy/Sync transforms, CopySpec duplicates, nested Copy/Sync and Zip/Tar
  CopySpec mappings, Zip/Tar/War/Ear archive tasks, a simple Exec task, and a
  JavaExec task, Javadoc task, and an OSS-style Java library slice with
  sources JAR/Javadoc/report outputs. Authoritative RunBuild corpus claims
  require a Gradle-under-test distribution built from this fork plus
  `target/debug/gradle-substrate-daemon`; the corpus runner now rejects
  bootstrap/upstream Gradle runs that do not emit a substrate run-build signal.
- A separate networked external-dependency corpus proves native Java compile and
  `TestExec` lowering for dependency-backed builds: a non-empty JUnit Platform
  test task with include/exclude tag capture plus a dependency
  constraints/exclusions sample.
- Native `TestExec` lowering captures a single Gradle include test filter and
  JUnit Platform include/exclude tags, and fails closed for unsupported filter
  shapes rather than approximating them.
- Native `JavaExec` lowering captures Java home, classpath, main class, JVM
  args, application args, working directory, and ignore-exit-value from the JVM
  task model, and fails closed unless classpath and main class are present.
- Native `Javadoc` lowering captures Java home, source files, classpath,
  destination directory, title, encoding, max memory, and timestamp behavior,
  and lowers only when source files and declared outputs are present.
- The Rust task graph enriches selected plans with graph-derived project
  classpaths and distribution dependency JARs when the JVM bridge exposes the
  dependency edge but omits the materialized file path. This keeps
  multiproject Java compile and application distribution packaging inside the
  Rust-controlled graph.
- The JVM bridge captures standard Java source-set compile/test classpaths
  from resolvable Gradle configurations and selected task input files at
  task-graph population. Rust classpath enrichment now relies on Gradle-captured
  task contracts and resolved dependency data; the previous direct
  build-script/module-cache classpath fallback has been removed from task-graph
  lowering.
- A narrow native `WriteFile` executor supports static single-output report
  tasks only when the JVM bridge can extract an exact literal `writeText(...)`
  contract from the build script. Arbitrary task actions still fail closed.
- The native-ready default gate
  `org.gradle.rust.substrate.runbuild.native-ready-default=true` tries Rust
  RunBuild first and delegates back to JVM execution when the selected plan is
  not fully native-ready. The offline corpus passes through this gate at 21/21.
- `testing/corpus/unsupported-manifest.json` tracks work that must remain
  outside approximate native execution until a complete contract exists. It now
  covers custom JVM task actions, unsupported CopySpec filter/actions,
  Copy/archive symlink inputs, and unsupported Test filter combinations. With a
  Gradle-under-test distribution built from this fork, the unsupported corpus
  passes 6/6 as expected fail-closed with no JVM task forwards.
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
- Gradle has an opt-in external resource download path
  (`org.gradle.rust.substrate.dependency.download.enabled=true`) that lets the
  Rust dependency transport serve uncached repository GETs before falling back
  to Gradle's Java transport. Successful Rust downloads still move through
  Gradle's normal external-resource cache and index. When Gradle is resolving a
  safe external module artifact by explicit coordinate, the coordinate is passed
  to Rust so the same streamed download also populates the coordinate-addressed
  Rust artifact store for future read-through hits. URL-only `.pom`, `.module`,
  `.ivy`, and `maven-metadata.xml` downloads also populate the URL-addressed
  Rust metadata store for later metadata read-through.
- Native dependency transport now persists downloaded Maven artifacts into the
  Rust artifact store, writes `.sha256` sidecars, validates cold and warm cache
  hits against requested SHA-256 values, populates the warm artifact cache
  immediately after a successful streamed download commit, and can verify
  checksums from persisted artifacts. Cold artifact and metadata read-through
  hits skip file hashing when Gradle does not request checksum validation, but
  retain fail-closed SHA-256 validation on any later checksum-checked read.
- Native dependency resolution has an opt-in `prefetch_artifacts` mode for
  static Maven artifact URLs. When requested, Rust resolves the graph, fetches
  each supported resolved artifact into the Rust artifact store, writes
  `.sha256` sidecars, returns artifact size/SHA-256 on the resolved nodes, and
  fails closed for unsupported artifact URL shapes instead of pretending parity.
  This is covered both by Rust resolver tests and by a Java bridge E2E test
  against a real Rust daemon.
- Rust artifact cache identity includes classifier and normalized extension, so
  in-memory warm hits cannot collide different artifact shapes for the same
  Maven coordinate.
- Rust dependency POM fetches are now repository-scoped cached metadata
  lookups: warm cache first, persisted Maven-layout `.pom` metadata second,
  network last. Successful network fetches commit atomically, write `.sha256`
  sidecars, and populate the warm cache for parent/BOM/transitive POM reuse.
- Rust dependency `maven-metadata.xml` fetches now use the same cache shape for
  the existing native metadata resolution path: warm cache first, persisted
  repository-scoped metadata second, URL-addressed metadata third, network last.
- Gradle has an opt-in metadata read-through path
  (`org.gradle.rust.substrate.dependency.readthrough.metadata=true`) that asks
  Rust for URL-addressed cached `.pom`, `.module`, `.ivy`, and
  `maven-metadata.xml` metadata when Gradle has no acceptable local cache entry.
  Existing Gradle cache refresh/revalidation semantics still win when a Gradle
  cache entry exists.
- A focused no-daemon integration smoke now warms Rust through a real HTTP
  Maven dynamic-version resolution, switches to a fresh Gradle user home, and
  resolves the same `1.+` dependency again with no remote expectations. This
  proves the dynamic-version `maven-metadata.xml`, selected POM, and artifact
  can read through from Rust stores before Gradle's remote path.
- The Gradle dependency shadow listener has an explicit artifact mirror mode
  (`org.gradle.rust.substrate.dependency.mirror.artifacts=true`) that observes
  real resolved external module artifacts, computes SHA-256 in the JVM bridge,
  preserves classifier and extension in the Rust cache key, and registers those
  artifacts into the Rust artifact store. The mode is opt-in because querying
  artifacts from a resolution listener can force artifact downloads earlier than
  a graph-only resolution would.
- Gradle artifact resolution has an explicit Rust read-through mode
  (`org.gradle.rust.substrate.dependency.readthrough.artifacts=true`) that
  consults the Rust artifact store after Gradle local access and before remote
  repository access for safe Maven-layout external module artifact coordinates,
  including non-JAR artifacts with explicit extensions. Cache misses fall back
  to Gradle's normal remote resolver. The Java bridge read-through path is
  covered by gated E2E tests against a real Rust daemon and artifact store.
- The first-60-seconds demo now proves static Maven artifact prefetch, artifact
  read-through, metadata read-through, and uncached resource download seams in
  focused checks, keeping the visible demo centered on Rust-backed dependency
  work instead of repeated Gradle test startup.
- Dependency read-through registries are configured from eager JVM-host bridge
  wiring, not the lazy dependency shadow listener, so normal installed Gradle
  builds can enable Rust metadata/artifact/download seams before dependency
  resolution starts.
- `org.gradle.rust.substrate.state.dir` can isolate the Rust daemon socket and
  durable stores for repeatable demos/tests without reusing
  `~/.gradle-substrate`.
- The first-60-seconds demo also includes an installed Gradle-under-test real
  build that warms Rust from a local HTTP Maven repository, deletes project
  outputs, reruns with a fresh Gradle user home, materializes the artifact
  again, and reports zero second-run remote requests.
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
- Native ZIP-compatible archive execution handles Gradle EAR contracts that
  include a not-yet-generated deployment descriptor mapping when an existing
  descriptor mapping for the same archive path is already present and
  `duplicatesStrategy = EXCLUDE`.
- Native file-transform and archive lowering fails closed when the JVM bridge
  detects symbolic links in task inputs. This avoids silently approximating
  symlink traversal/copy semantics until they are modeled explicitly.

## Gaps

- Full dependency resolution semantics are not yet parity-complete.
- Rust can now short-circuit Gradle remote artifact fetches from the Rust
  artifact store for opt-in, Maven-layout external artifact coordinates, but it
  does not yet replace metadata resolution, variant selection, conflict
  resolution, or Ivy artifact resolution.
- Real Gradle invocation does not use Rust as the unconditional default executor
  yet; the authoritative build-work gate is intentionally opt-in and fail-closed
  while the native-ready default gate delegates on incomplete plans.
- Kotlin/Groovy DSL evaluation and legacy plugin execution remain JVM-host
  compatibility islands.
- More task types need native-ready contract capture before broad no-fallback
  execution is realistic, especially arbitrary task actions beyond static
  literal file writes, arbitrary copy filters/actions beyond direct expand,
  Gradle archive metadata edge cases beyond entry inventory, and native symlink
  copy/archive semantics.
- External dependency classpaths are now covered by the external corpus through
  Gradle-captured selected task contracts. Project/dependency model capture can
  still be richer, but task execution no longer uses the removed Rust-side
  build-script dependency parser fallback for classpaths.
- Rust build-session hashing can now shadow or authoritatively replace the
  local `DefaultFileHasher` delegate while preserving Gradle's global/user-home
  VFS scopes. File-collection fingerprinting can now run authoritatively for
  direct files, missing roots, directories, and PatternSet-backed file trees by
  materializing Gradle-compatible regular-file, missing-file, and Merkle
  directory snapshot objects from Rust hashes. File-tree-backed archive inputs
  now snapshot the backing archive file via Rust, matching Gradle's existing
  fingerprinting boundary while preserving the separate archive-tree `isEmpty`
  check. It still fails closed for symlinks and special files. Value
  snapshotting can now run authoritatively for exact built-in value shapes that
  do not require Java serialization or classloader hashes: null, strings,
  booleans, integer/long/short numbers, files, enums, hash codes, lists, sets,
  maps, object arrays, and primitive arrays. Unsupported JVM-only values fail
  closed.
  Authoritative value input fingerprinting batches supported value properties
  into one Rust canonical snapshot RPC per fingerprinting pass instead of one
  RPC per property.
- User-home/global VFS and file watching remain JVM-owned in installed
  distributions. Moving those scopes requires a separate design that does not
  request build-session-only Rust services from global providers.

## Validation

- `cargo check -p gradle-substrate-daemon`
- `cargo test -p gradle-substrate-daemon --test hash_compatibility_test`
- `cargo test -p gradle-substrate-daemon dag_executor -- --nocapture`
- `cargo test -p gradle-substrate-daemon task_graph -- --nocapture`
- `cargo test --manifest-path substrate/Cargo.toml -p gradle-substrate-daemon test_ear_excludes_missing_generated_duplicate_descriptor -- --nocapture`
- `cargo test -p gradle-substrate-daemon writes_static_text_to_declared_output_file -- --nocapture`
- `cargo test -p gradle-substrate-daemon --test build_plan_shadow_test refreshed_native_ready_shadow_plan_runs_java_lifecycle_without_jvm_fallback -- --exact`
- `./gradlew :core:test --tests org.gradle.execution.RustAuthoritativeBuildExecutionActionTest -x :distributions-core:generateLicenseFile`
- `./gradlew :rust-bridge:compileJava :rust-bridge:test --tests '*SubstrateLifecycleTest*' --no-daemon --console=plain`
- `./gradlew :rust-bridge:compileJava :rust-bridge:test --tests '*ProjectModelProviderAdapterTest*' --no-daemon --console=plain`
- `./gradlew :rust-bridge:compileJava :rust-bridge:test --tests '*JvmHostServiceImplTest*' --tests '*ProjectModelProviderAdapterTest*' --no-daemon --console=plain`
- `./gradlew :rust-bridge:compileJava :rust-bridge:test --tests '*BuildPlanTaskSelectionCaptureListenerTest*' --tests '*TaskGraphShadowListenerTest*' --tests '*ProjectModelProviderAdapterTest*' --tests '*JvmHostServiceImplTest*' --no-daemon --console=plain`
- `build/gradle-under-test/bin/gradle -p testing/corpus/java-library-kotlin-dsl clean build --no-daemon --console=plain -Dorg.gradle.rust.substrate.enabled=true -Dorg.gradle.rust.substrate.mode=shadow -Dorg.gradle.rust.substrate.daemon.path=$PWD/target/debug/gradle-substrate-daemon -Dorg.gradle.rust.substrate.runbuild.authoritative=true --info` twice verifies first-launch then persisted TCP daemon reuse.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --gradle-command "$GRADLE_UNDER_TEST/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --verbose`
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-authoritative-21-daemon-reuse-final --verbose` passed 21/21, no fallback, 200/200 task parity, output/hash/archive parity, observed wall time upstream=101529ms and substrate=155998ms.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-authoritative-21-external-classpath --verbose` passed 21/21, no fallback, 200/200 task parity, output/hash/archive parity, observed wall time upstream=118407ms and substrate=169730ms.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-authoritative-21-resolved-plus-script-fallback --verbose` passed 21/21, no fallback, 200/200 task parity, output/hash/archive parity, observed wall time upstream=119109ms and substrate=169340ms.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-authoritative-21-clean-finalized-inline --verbose` passed 21/21, no fallback, 200/200 task parity, output/hash/archive parity, observed wall time upstream=111974ms and substrate=167853ms.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-authoritative-21-early-contracts-no-parser-earfix --verbose` passed 21/21, no fallback, 200/200 task parity, output/hash/archive parity, observed wall time upstream=64411ms and substrate=175554ms.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --gradle-command "$GRADLE_UNDER_TEST/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-native-ready-default --tasks clean build --timeout 300 --output-dir build/corpus-native-ready-default-21`
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/external-manifest.json --gradle-command "$GRADLE_UNDER_TEST/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --verbose`
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/external-manifest.json --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-external-rust-cache-classpath-3 --verbose` passed 2/2, no fallback, 25/25 task parity, output/hash/archive parity, observed wall time upstream=7739ms and substrate=17479ms.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/external-manifest.json --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-external-resolved-plus-script-fallback --verbose` passed 2/2, no fallback, 25/25 task parity, output/hash/archive parity, observed wall time upstream=7730ms and substrate=17758ms.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/external-manifest.json --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-external-clean-finalized-inline --verbose` passed 2/2, no fallback, 25/25 task parity, output/hash/archive parity, observed wall time upstream=18575ms and substrate=17629ms.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/external-manifest.json --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-external-early-contracts-no-parser-earfix --verbose` passed 2/2, no fallback, 25/25 task parity, output/hash/archive parity, observed wall time upstream=7919ms and substrate=19063ms.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/unsupported-manifest.json --contract-only --output-dir build/corpus-contract-unsupported`
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/unsupported-manifest.json --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-unsupported-fail-closed-under-test`
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/unsupported-manifest.json --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-unsupported-fail-closed` fails honestly without `--gradle-command` because the bootstrap wrapper does not emit substrate run-build markers.
- `cargo test -p gradle-substrate-daemon download_artifact -- --nocapture`
- `cargo test -p gradle-substrate-daemon artifact_cache -- --nocapture`
- `cargo test -p gradle-substrate-daemon fetch_pom -- --nocapture`
- `cargo test -p gradle-substrate-daemon test_download_metadata_url_populates_metadata_cache -- --nocapture`
- `cargo test -p gradle-substrate-daemon test_download_maven_metadata_url_populates_dynamic_metadata_cache -- --nocapture`
- `cargo test -p gradle-substrate-daemon test_fetch_maven_metadata_populates_store_and_reuses_persistent_cache -- --nocapture`
- `cargo test -p gradle-substrate-daemon persistent_store_roundtrip -- --nocapture`
- `cargo test -p gradle-substrate-daemon test_add_artifact_to_cache_preserves_extension`
- `cargo test -p gradle-substrate-daemon cold_path -- --nocapture`
- `./gradlew :rust-bridge:test --tests org.gradle.internal.rustbridge.dependency.RustMetadataCacheReadThroughTest`
- `./gradlew :dependency-management:test --tests org.gradle.internal.resource.transfer.DefaultCacheAwareExternalResourceAccessorTest -x :distributions-core:generateLicenseFile`
- `./gradlew :rust-bridge:test --tests org.gradle.internal.rustbridge.dependency.DependencyResolutionShadowListenerTest`
- `./gradlew :rust-bridge:test --tests org.gradle.internal.rustbridge.dependency.RustArtifactCacheReadThroughTest`
- `./gradlew :rust-bridge:test --tests org.gradle.internal.rustbridge.e2e.SubstrateE2ETest.dependencyArtifactReadThroughReturnsArtifactFromRustStore -Dsubstrate.test.binary=$PWD/target/debug/gradle-substrate-daemon`
- `./gradlew :rust-bridge:test --tests org.gradle.internal.rustbridge.e2e.SubstrateE2ETest.dependencyArtifactReadThroughPreservesArtifactExtension -Dsubstrate.test.binary=$PWD/target/debug/gradle-substrate-daemon`
- `./gradlew :rust-bridge:test --tests org.gradle.internal.rustbridge.e2e.SubstrateE2ETest.dependencyDownloadStreamsResourceThroughRustTransport -Dsubstrate.test.binary=$PWD/target/debug/gradle-substrate-daemon`
- `./gradlew :rust-bridge:test --tests org.gradle.internal.rustbridge.e2e.SubstrateE2ETest.dependencyDownloadPopulatesMetadataUrlCache -Dsubstrate.test.binary=$PWD/target/debug/gradle-substrate-daemon`
- `./gradlew :dependency-management:noDaemonIntegTest --tests "org.gradle.integtests.resolve.maven.MavenDynamicResolveIntegrationTest.rust transport warms dynamic version metadata for later no-remote read-through" -Dsubstrate.test.binary=$PWD/target/debug/gradle-substrate-daemon -x :distributions-core:generateLicenseFile --no-daemon --console=plain`
- `./gradlew :dependency-management:test --tests org.gradle.api.internal.artifacts.ivyservice.ivyresolve.RepositoryChainArtifactResolverTest -x :distributions-core:generateLicenseFile`
- `./gradlew :dependency-management:test --tests org.gradle.internal.resource.transfer.DefaultCacheAwareExternalResourceAccessorTest -x :distributions-core:generateLicenseFile`
- `./gradlew :rust-bridge:test --tests org.gradle.internal.rustbridge.e2e.SubstrateE2ETest.dependencyResolveCanPrefetchStaticMavenArtifact -Dsubstrate.test.binary=$PWD/target/debug/gradle-substrate-daemon --no-daemon --console=plain`
- `cargo test -p gradle-substrate-daemon test_no_checksum -- --nocapture`
- `cargo test -q -p gradle-substrate-daemon server::file_fingerprint::tests -- --nocapture`
- `./gradlew :rust-bridge:compileJava :core:compileJava --no-daemon --console=plain`
- `cargo build -q -p gradle-substrate-daemon`
- `./gradlew :rust-bridge:test --tests '*AuthoritativeRustValueSnapshotterTest' --tests '*ShadowingValueSnapshotterTest' --tests '*ShadowingInputFingerprinterTest' --no-daemon --console=plain`
- `./gradlew :rust-bridge:test --tests '*AuthoritativeRustValueSnapshotterTest' --no-daemon --console=plain`
- `./gradlew :rust-bridge:test --tests '*ShadowingFileHasherTest' --tests '*ShadowingFileCollectionSnapshotterTest' --tests '*ShadowingInputFingerprinterTest' --no-daemon --console=plain`
- `./gradlew :rust-bridge:test --tests '*ShadowingFileCollectionSnapshotterTest' --no-daemon --console=plain`
- `./gradlew :distributions-full:install -Pgradle_installPath=$PWD/build/gradle-under-test -Dorg.gradle.unsafe.isolated-projects=false -Dorg.gradle.configuration-cache=false --no-daemon --console=plain`
- `build/gradle-under-test/bin/gradle -p testing/corpus/java-library-kotlin-dsl clean classes --no-daemon --console=plain -Dorg.gradle.rust.substrate.enabled=true -Dorg.gradle.rust.substrate.daemon.path=$PWD/target/debug/gradle-substrate-daemon -Dorg.gradle.rust.substrate.hashing.enabled=true -Dorg.gradle.rust.substrate.fingerprint.enabled=true -Dorg.gradle.rust.substrate.shadow.report-mismatches=true --info`
- `build/gradle-under-test/bin/gradle -p testing/corpus/java-library-kotlin-dsl clean classes --no-daemon --console=plain -Dorg.gradle.rust.substrate.enabled=true -Dorg.gradle.rust.substrate.daemon.path=$PWD/target/debug/gradle-substrate-daemon -Dorg.gradle.rust.substrate.hashing.enabled=true -Dorg.gradle.rust.substrate.fingerprint.enabled=true -Dorg.gradle.rust.substrate.fingerprint.authoritative=true --info`
- `build/gradle-under-test/bin/gradle -p testing/corpus/java-library-kotlin-dsl classes --no-daemon --console=plain -Dorg.gradle.rust.substrate.enabled=true -Dorg.gradle.rust.substrate.daemon.path=$PWD/target/debug/gradle-substrate-daemon -Dorg.gradle.rust.substrate.hashing.enabled=true -Dorg.gradle.rust.substrate.fingerprint.enabled=true -Dorg.gradle.rust.substrate.fingerprint.authoritative=true --info`
- `build/gradle-under-test/bin/gradle -p testing/corpus/java-library-kotlin-dsl clean classes --no-daemon --console=plain -Dorg.gradle.rust.substrate.enabled=true -Dorg.gradle.rust.substrate.daemon.path=$PWD/target/debug/gradle-substrate-daemon -Dorg.gradle.rust.substrate.hashing.enabled=true -Dorg.gradle.rust.substrate.hashing.authoritative=true -Dorg.gradle.rust.substrate.fingerprint.enabled=true -Dorg.gradle.rust.substrate.fingerprint.authoritative=true -Dorg.gradle.rust.substrate.snapshotting.enabled=true -Dorg.gradle.rust.substrate.snapshotting.authoritative=true`
- `build/gradle-under-test/bin/gradle -p testing/corpus/java-library-kotlin-dsl classes --no-daemon --console=plain -Dorg.gradle.rust.substrate.enabled=true -Dorg.gradle.rust.substrate.daemon.path=$PWD/target/debug/gradle-substrate-daemon -Dorg.gradle.rust.substrate.hashing.enabled=true -Dorg.gradle.rust.substrate.hashing.authoritative=true -Dorg.gradle.rust.substrate.fingerprint.enabled=true -Dorg.gradle.rust.substrate.fingerprint.authoritative=true -Dorg.gradle.rust.substrate.snapshotting.enabled=true -Dorg.gradle.rust.substrate.snapshotting.authoritative=true`
- `python3 tools/performance/rust_substrate_perf_report.py build/corpus-authoritative-21/corpus_summary.json --output build/corpus-authoritative-21/performance.md`
- `python3 tools/demo/first_60_seconds.py --output build/first60-static-prefetch-final.json` passed with daemon socket ready=546.4ms, Rust dependency transport/store/checksum=774.3ms, metadata cache=294.9ms, dynamic metadata cache=265.9ms, static Maven prefetch=268.2ms, shared artifact/metadata read-through Gradle proof=8302.8ms, real-build read-through=25457.9ms with remote requests avoided=3/3 and deterministic output SHA-256 `742d753d434f2e408762a9cbcc75621c441e0ed5dea5961d65b05ff3013575fc`, file-watch first event=12ms.
- `./tools/stabilization/run_strict_stabilization.sh quick`
- `./tools/demo/rust_substrate_demo.sh --quick`

## Next Sync Actions

1. Add native-ready contracts for richer `Copy`/`Sync` specs and the next common
   process task after `Javadoc`.
2. Continue replacing dependency-resolution pieces with narrow, parity-tested
   Rust services while Gradle remains the source of truth for full variant and
   conflict semantics.
3. Extend authoritative Rust file-collection snapshotting beyond direct
   files/directories/file trees/archive-backed files to symlink semantics and
   special files, or keep those cases explicitly unsupported.
4. Design a global/user-home VFS bridge that does not leak build-session
   services into global scopes.
5. Reduce bridge source exclusions as APIs are stabilized.
6. Track upstream commit synchronization in this file for each parity push.
