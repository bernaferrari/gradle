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
  `org.gradle.rust.substrate.execution.kernel=true` executes the selected
  build-plan shadow through Rust `RunBuild`, admits the whole selected plan into
  the Rust execution kernel, and skips Gradle's JVM task executor only when Rust
  reports exactly the scheduled task count with zero JVM forwards. The older
  `org.gradle.rust.substrate.runbuild.authoritative=true` flag remains a
  compatibility alias for this strict no-fallback mode.
- The explicit Rust `RunBuild` path no longer enables unrelated Java-side
  bootstrap, build-result, metrics, history, or JVM-host lifecycle services.
  It keeps only the early selected-task contract capture needed to execute the
  finalized Gradle DAG from Rust, while umbrella `mode=shadow` still enables the
  broader listener set for subsystem shadowing.
- The Rust wrapper binary now has first-minute substrate entry points:
  `--rust-substrate` strips the wrapper-only flag, injects the minimal Rust DAG
  flags plus safe dependency transport/read-through flags, locates
  `gradle-substrate-daemon`, and uses native-ready-default execution;
  `--rust-substrate-authoritative` injects
  `org.gradle.rust.substrate.execution.kernel=true`, the stricter no-fallback
  Rust execution-kernel gate for validation. `GRADLEW_DISTRIBUTION_DIR` can
  point the Rust wrapper at a local install image from this fork, and launcher
  discovery now supports both ZIP-extracted and direct install-image `lib/`
  layouts.
- The Rust wrapper now enforces `validateDistributionUrl=true` before fetching,
  supports `http(s)://` and `file:/` distribution URLs, copies local file
  distributions into the wrapper ZIP store, and still applies SHA-256
  verification when `distributionSha256Sum` is present.
- The Rust wrapper now prewarms `gradle-substrate-daemon` for substrate runs:
  it creates the same state directories as the JVM bridge, starts the daemon on
  loopback TCP, writes the Java-compatible `substrate.tcp-endpoint` identity
  file, and injects the matching `org.gradle.rust.substrate.state.dir`. The JVM
  bridge can then connect to the existing Rust daemon instead of owning sidecar
  startup.
- Rust `RunBuild` dispatch now uses a native ready-task priority queue keyed by
  remaining critical-path duration instead of FIFO order, so independent ready
  tasks on the longest path are claimed first. Filtered task selections also
  treat dependencies outside the selected graph as absent when deciding initial
  readiness.
- Rust `RunBuild` with JVM forwarding disabled now has a build-level execution
  kernel admission gate. After the task graph is materialized, Rust validates
  that every selected task has a native executor and that captured task
  contracts do not carry explicit unsupported markers before dispatching any
  work. Unsupported no-fallback builds now fail closed at admission time instead
  of discovering fallback requirements mid-DAG.
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
  CopySpec mappings, simple Copy/Zip file-symlink inputs that Gradle follows as
  target bytes, a static `eachFile` relative-path rewrite captured as explicit
  copy mappings, a static `filter { line.replace(...) }` literal replacement,
  Zip/Tar/War/Ear archive tasks, a simple Exec task, and a JavaExec task,
  Javadoc task, and an OSS-style Java library slice with sources
  JAR/Javadoc/report outputs. Authoritative RunBuild corpus claims require a
  Gradle-under-test distribution built from this fork plus
  `target/debug/gradle-substrate-daemon`; the corpus runner now rejects
  bootstrap/upstream Gradle runs that do not emit a substrate run-build signal.
- A separate networked external-dependency corpus proves native Java compile and
  `TestExec` lowering for dependency-backed builds: a non-empty JUnit Platform
  test task with include/exclude tag and class-name filter capture plus a dependency
  constraints/exclusions sample.
- Native `TestExec` lowering captures Gradle class-name include/exclude filters
  and JUnit Platform include/exclude tags, and fails closed for method-level or
  otherwise unsupported filter shapes rather than approximating them.
- Native `JavaExec` lowering captures Java home, classpath, main class, max
  heap size, JVM args, system properties, application args, environment,
  working directory, and ignore-exit-value from the JVM task model, and fails
  closed unless the main class and either a captured classpath or an inferable
  standard Java-plugin output classpath are present.
- Native `Exec` lowering captures executable, args, environment, working
  directory, and ignore-exit-value from the JVM task model.
- Native `Javadoc` lowering captures Java home, source files, classpath,
  destination directory, title, encoding, max memory, and timestamp behavior,
  honors max-memory JVM options in the Rust executor, and lowers only when
  source files and declared outputs are present.
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
  not fully native-ready. The authoritative offline no-fallback corpus now
  passes 26/26.
- `testing/corpus/unsupported-manifest.json` tracks work that must remain
  outside approximate native execution until a complete contract exists. It now
  covers custom JVM task actions and method-level Test filters. Contract-only
  validation covers these 2 known unsupported shapes; the normal
  parity runner intentionally reports
  mismatches for unsupported projects rather than treating approximate native
  execution as acceptable.
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
  retain fail-closed SHA-256 validation on any later checksum-checked read; the
  native resolver's persisted text metadata cache follows the same lazy-SHA
  rule for warm POM/module/metadata reuse. Mirrored local artifacts are copied
  into the Rust store with a single streaming copy+SHA pass and atomic commit,
  and Rust rejects caller-provided SHA-256 mismatches or missing local files
  instead of caching them. Warm artifact-cache entries fail closed and are
  evicted when the persisted file has disappeared; URL metadata warm-cache
  entries follow the same stale-file eviction behavior.
- Rust dependency transport now serves `DownloadArtifact` requests from the
  warm Rust artifact cache first, then the persisted Rust artifact/metadata
  stores, before opening HTTP. Stale warm entries are evicted before fallback.
  This moves repeated Gradle external-resource downloads onto the native cache
  path even when the JVM-side caller reaches the download seam instead of the
  explicit read-through seam.
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
  real resolved external module artifacts, preserves classifier and extension
  in the Rust cache key, and registers those artifacts into the Rust artifact
  store. The JVM bridge no longer pre-hashes mirrored artifact files; Rust owns
  the single streaming copy+SHA pass. The mode is opt-in because querying
  artifacts from a resolution listener can force artifact downloads earlier
  than a graph-only resolution would.
- The Gradle dependency shadow listener also has an opt-in static Maven
  artifact prefetch mode
  (`org.gradle.rust.substrate.dependency.prefetch.artifacts=true`) that runs
  only after Java dependency resolution succeeds. The JVM bridge captures a
  native-ready contract for exactly one HTTP(S) Maven repository plus direct
  external module dependencies with static versions, no excludes, no target
  configuration, no changing/SNAPSHOT/dynamic versions, and at most one normal
  artifact shape. Unsupported shapes fail closed by skipping Rust prefetch
  rather than approximating Gradle semantics. Prefetch is gated by the resolved
  component graph instead of low-level metadata-attempt failures, so harmless
  repository misses such as absent Gradle module metadata do not block static
  Maven artifact warming. Once the declared dependencies pass that contract,
  the listener prefetches every static Maven module in Gradle's resolved graph
  as an exact non-transitive artifact request, so graph-only resolution can
  warm direct and transitive artifacts without re-solving dependency semantics
  in Rust. A no-daemon integration smoke now proves this through a real Gradle
  build: a graph-only first run warms Rust through the listener, then a second
  run with a fresh Gradle user home retrieves direct and transitive artifacts
  with no remote repository expectations.
- Gradle artifact resolution has an explicit Rust read-through mode
  (`org.gradle.rust.substrate.dependency.readthrough.artifacts=true`) that
  consults the Rust artifact store after Gradle local access and before remote
  repository access for safe Maven-layout external module artifact coordinates,
  including non-JAR artifacts with explicit extensions. Cache misses fall back
  to Gradle's normal remote resolver. The Java bridge read-through path is
  covered by gated E2E tests against a real Rust daemon and artifact store.
- The first-60-seconds demo has a fast mode for user-visible wins and a proof
  mode for heavier subsystem checks. Fast mode reports daemon readiness,
  cold and warm explicit authoritative Rust `RunBuild` on a copied
  Java-library corpus project with Gradle configuration-cache reuse, Rust
  up-to-date skip counts, and zero JVM task forwards, real-build dependency
  read-through with static direct/transitive prefetch, and native file-watch
  latency. Proof mode keeps static Maven artifact prefetch, artifact
  read-through, metadata read-through, and uncached resource download seam
  checks available without slowing the visible path.
- Authoritative Rust `RunBuild` refreshes the finalized selected task graph
  from early captured immutable task contracts instead of reading live Gradle
  `Task.project` state during execution. The inline build-plan refresh derives
  project entries from captured contract project paths, so the warm path is
  compatible with Gradle configuration-cache storage/reuse for the covered
  Java-library corpus slice.
- Rust task graph contexts now include work identity, scalar execution inputs,
  declared outputs, and content fingerprints for native execution inputs. The
  execution-plan history is flushed through the Rust history store so repeated
  authoritative `RunBuild` invocations can make up-to-date decisions even when
  the sidecar process is restarted between Gradle invocations.
- Dependency read-through registries are configured from eager JVM-host bridge
  wiring, not the lazy dependency shadow listener, so normal installed Gradle
  builds can enable Rust metadata/artifact/download seams before dependency
  resolution starts. The same eager wiring also registers the static prefetch
  listener, and repository capture makes internal `ProjectState` reflection
  accessible so installed builds can capture the root project's Maven
  repositories.
- `org.gradle.rust.substrate.state.dir` can isolate the Rust daemon socket and
  durable stores for repeatable demos/tests without reusing
  `~/.gradle-substrate`.
- The first-60-seconds demo also includes an installed Gradle-under-test real
  build that warms Rust from a local HTTP Maven repository, deletes project
  outputs, reruns with a fresh Gradle user home, materializes the artifact
  again, and reports zero second-run remote requests. That metric now combines
  dynamic metadata/artifact read-through with listener-driven static Maven
  direct/transitive artifact prefetch, proving the common first-minute
  dependency path without JVM task forwarding.
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
- Native `Copy` can execute a narrow static `eachFile` relative-path rewrite
  when the JVM bridge recognizes `relativePath = RelativePath(true, "prefix",
  name)` and emits explicit source-to-destination file mappings. Other
  arbitrary copy actions and content filters still fail closed.
- Native `Copy` can execute a narrow static `filter { line: String ->
  line.replace("from", "to") }` literal replacement when the JVM bridge can
  encode the replacement exactly. Other arbitrary filter closures still fail
  closed.
- Native `TestExec` can execute class-name include/exclude filters through JUnit
  Platform `--include-classname`/`--exclude-classname` arguments. Method-level
  and display-name-style filters still fail closed because they are not
  equivalent to JUnit class-name filtering.
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
- Native file-transform and archive lowering supports simple file symlink
  inputs by following target bytes, matching the observed Gradle Copy/Zip
  behavior in the corpus. Directory symlink inputs still fail closed in the Rust
  executors.

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
  method-level Test filters, Gradle archive metadata edge cases beyond entry
  inventory, and native symlink copy/archive semantics.
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
- `cargo test -p gradle-substrate-daemon warm_cache_local_path -- --nocapture`
- `cargo test -p gradle-substrate-daemon test_download_ -- --nocapture`
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
- `./gradlew :rust-bridge:test --tests org.gradle.internal.rustbridge.dependency.DependencyResolutionShadowListenerTest --no-daemon --console=plain`
- `./gradlew :dependency-management:noDaemonIntegTest --tests "org.gradle.integtests.resolve.maven.MavenDynamicResolveIntegrationTest.rust listener prefetches static maven artifact for later no-remote read-through" -Dsubstrate.test.binary=$PWD/target/debug/gradle-substrate-daemon -x :distributions-core:generateLicenseFile --no-daemon --console=plain`
- `./gradlew :distributions-full:install -Pgradle_installPath=$PWD/build/gradle-under-test --no-daemon --console=plain`
- `python3 tools/demo/first_60_seconds.py --output build/first60-listener-prefetch.json` passed with daemon socket ready=16.9ms, Rust dependency transport/store/checksum=636.3ms, metadata cache=260.0ms, dynamic metadata cache=257.8ms, static Maven prefetch=266.8ms, shared artifact/metadata read-through Gradle proof=15849.8ms, real-build read-through=8161.2ms with remote requests avoided=5/5 and deterministic dynamic/static output SHA-256 values `742d753d434f2e408762a9cbcc75621c441e0ed5dea5961d65b05ff3013575fc` / `88f9a4aa9c2579d0caa524dac61d45a114f381572629c6645fa28979d2501196`, file-watch first event=12ms.
- `python3 tools/demo/first_60_seconds.py --output build/first60-transitive-prefetch.json` passed with daemon socket ready=639.5ms, Rust dependency transport/store/checksum=716.0ms, metadata cache=263.5ms, dynamic metadata cache=264.6ms, static Maven prefetch=269.5ms, shared artifact/metadata read-through Gradle proof=15586.8ms, real-build read-through=7964.4ms with remote requests avoided=7/7 and deterministic dynamic/direct-static/transitive-static output SHA-256 values `742d753d434f2e408762a9cbcc75621c441e0ed5dea5961d65b05ff3013575fc` / `88f9a4aa9c2579d0caa524dac61d45a114f381572629c6645fa28979d2501196` / `ecffccc8258a7d1647e4f911a8b9c6874d86f3ef8a88ddf0faad23d3e1cd148b`, file-watch first event=12ms.
- `python3 tools/demo/first_60_seconds.py --output build/first60-cache-first-transport.json` passed with daemon socket ready=1397.8ms (daemon self-report 10ms), Rust dependency transport/store/checksum/cache-first reuse=21757.6ms, metadata cache=500.0ms, dynamic metadata cache=266.8ms, static Maven prefetch=272.9ms, shared artifact/metadata read-through Gradle proof=15497.8ms, real-build read-through=8854.7ms with remote requests avoided=7/7 and deterministic dynamic/direct-static/transitive-static output SHA-256 values `742d753d434f2e408762a9cbcc75621c441e0ed5dea5961d65b05ff3013575fc` / `88f9a4aa9c2579d0caa524dac61d45a114f381572629c6645fa28979d2501196` / `ecffccc8258a7d1647e4f911a8b9c6874d86f3ef8a88ddf0faad23d3e1cd148b`, file-watch first event=12ms.
- `python3 tools/demo/first_60_seconds.py --mode fast --skip-build --output build/first60-fast.json` passed with daemon socket ready=712.3ms (daemon self-report 9ms), authoritative Rust DAG=21291.6ms with cold 18284.2ms/11 Rust tasks, warm 3003.0ms/11 Rust tasks, Gradle configuration-cache reuse on the warm run, 5 Rust up-to-date skips, 0 JVM forwards, real-build dependency read-through=7688.4ms with remote requests avoided=7/7 and deterministic dynamic/direct-static/transitive-static output SHA-256 values `742d753d434f2e408762a9cbcc75621c441e0ed5dea5961d65b05ff3013575fc` / `88f9a4aa9c2579d0caa524dac61d45a114f381572629c6645fa28979d2501196` / `ecffccc8258a7d1647e4f911a8b9c6874d86f3ef8a88ddf0faad23d3e1cd148b`, and file-watch first event=12ms.
- `cargo test -p gradle-substrate-daemon test_declared_outputs_present -- --nocapture`
- `cargo test -p gradle-substrate-daemon test_run_build_skips_no_source -- --nocapture`
- `cargo test -p gradle-substrate-daemon test_process_resources_context_marks_no_source -- --nocapture`
- `cargo test -p gradle-substrate-daemon test_refreshed_work_metadata -- --nocapture`
- `cargo test -p gradle-substrate-daemon test_archive_context -- --nocapture`
- `cargo build -q -p gradle-substrate-daemon`
- `python3 -m py_compile tools/demo/first_60_seconds.py`
- `./gradlew :core:compileJava :rust-bridge:compileJava --no-daemon --console=plain`
- `./gradlew :distributions-full:install -Pgradle_installPath=$PWD/build/gradle-under-test --no-daemon --console=plain`
- `python3 tools/demo/first_60_seconds.py --mode fast --skip-build --output build/first60-fast.json` passed with daemon socket ready=1389.6ms (daemon self-report 11ms), authoritative Rust DAG=71364.4ms with cold 68640.2ms/11 Rust tasks after reinstall, warm 2720.8ms/11 Rust tasks, Gradle configuration-cache reuse on the warm run, 6 Rust up-to-date skips, 4 Rust no-source/skipped tasks, 0 JVM forwards, real-build dependency read-through=7557.3ms with remote requests avoided=7/7 and deterministic dynamic/direct-static/transitive-static output SHA-256 values `742d753d434f2e408762a9cbcc75621c441e0ed5dea5961d65b05ff3013575fc` / `88f9a4aa9c2579d0caa524dac61d45a114f381572629c6645fa28979d2501196` / `ecffccc8258a7d1647e4f911a8b9c6874d86f3ef8a88ddf0faad23d3e1cd148b`, and file-watch first event=13ms.
- `python3 tools/demo/first_60_seconds.py --mode proof --skip-build --output build/first60-proof.json` passed with Rust dependency transport/store/checksum=269.4ms, metadata cache=253.3ms, dynamic metadata cache=256.5ms, static Maven prefetch=262.4ms, and shared artifact/metadata read-through Gradle proof=20506.9ms.
- `cargo test -p gradle-substrate-daemon test_no_checksum -- --nocapture`
- `cargo test -p gradle-substrate-daemon test_cached_text_metadata_read_does_not_freeze_stale_sha -- --nocapture`
- `cargo test -p gradle-substrate-daemon test_work_input_properties_ -- --nocapture`
- `cargo test -p gradle-substrate-daemon test_archive_context -- --nocapture`
- `cargo build -q -p gradle-substrate-daemon`
- `python3 tools/demo/first_60_seconds.py --mode fast --skip-build --output build/first60-fast.json` passed with daemon socket ready=1260.3ms (daemon self-report 12ms), authoritative Rust DAG=71683.7ms with cold 68896.3ms/11 Rust tasks, warm 2783.3ms/11 Rust tasks, Gradle configuration-cache reuse on the warm run, 7 Rust up-to-date skips, 4 Rust no-source/skipped tasks, 0 JVM forwards, real-build dependency read-through=7337.6ms with remote requests avoided=7/7 and deterministic dynamic/direct-static/transitive-static output SHA-256 values `742d753d434f2e408762a9cbcc75621c441e0ed5dea5961d65b05ff3013575fc` / `88f9a4aa9c2579d0caa524dac61d45a114f381572629c6645fa28979d2501196` / `ecffccc8258a7d1647e4f911a8b9c6874d86f3ef8a88ddf0faad23d3e1cd148b`, authoritative output SHA-256 `92cf8132cd7364798a080c27ca6c160811e962cab18bb92f2872f061878a8490`, and file-watch first event=14ms.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-authoritative-after-jar-warm --verbose` passed 21/21, no fallback, 200/200 task parity, output inventory/hash/archive parity, observed wall time upstream=118454ms and substrate=165137ms.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/external-manifest.json --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-external-after-jar-warm --verbose` passed 2/2, no fallback, 25/25 task parity, output inventory/hash/archive parity, observed wall time upstream=12004ms and substrate=16097ms.
- `./gradlew :rust-bridge:test --tests org.gradle.internal.rustbridge.SubstrateLifecycleTest --no-daemon --console=plain` passed after making the JVM compatibility host opt-in instead of implicitly enabled by `org.gradle.rust.substrate.mode=shadow`.
- `./gradlew :distributions-full:install -Pgradle_installPath=$PWD/build/gradle-under-test --no-daemon --console=plain` rebuilt the Gradle-under-test distribution with the bridge lifecycle change.
- `python3 tools/demo/first_60_seconds.py --mode fast --skip-build --output build/first60-no-jvm-host.json` passed with daemon socket ready=38.4ms (daemon self-report 22ms), authoritative Rust DAG=20413.1ms with cold 17664.2ms/11 Rust tasks, warm 2745.5ms/11 Rust tasks, Gradle configuration-cache reuse on the warm run, 7 Rust up-to-date skips, 4 Rust no-source/skipped tasks, 0 JVM forwards, authoritative output SHA-256 `92cf8132cd7364798a080c27ca6c160811e962cab18bb92f2872f061878a8490`, real-build dependency read-through=7510.9ms with remote requests avoided=7/7, and file-watch first event=12ms.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-authoritative-no-implicit-jvm-host --verbose` passed 21/21, no fallback, 200/200 task parity, output inventory/hash/archive parity, observed wall time upstream=119248ms and substrate=168059ms.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/external-manifest.json --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-external-no-implicit-jvm-host --verbose` passed 2/2, no fallback, 25/25 task parity, output inventory/hash/archive parity, observed wall time upstream=13085ms and substrate=17216ms.
- `python3 -m unittest tools.corpus_runner.test_run && python3 -m py_compile tools/demo/first_60_seconds.py` passed after switching no-fallback RunBuild commands from umbrella `mode=shadow` to explicit minimal `enabled=true`, `taskgraph.enabled=true`, `runbuild.enabled=true`, and `execution.kernel=true` flags.
- `python3 tools/demo/first_60_seconds.py --mode fast --skip-build --output build/first60-minimal-runbuild.json` passed with authoritative Rust DAG=18165.2ms, cold 15179.1ms/11 Rust tasks, warm 2983.1ms/11 Rust tasks, 7 Rust up-to-date skips, 4 Rust no-source/skipped tasks, 0 JVM forwards, authoritative output SHA-256 `92cf8132cd7364798a080c27ca6c160811e962cab18bb92f2872f061878a8490`, real-build dependency read-through=7875.8ms with remote requests avoided=7/7, and file-watch first event=11ms.
- `python3 tools/corpus_runner/run.py --project "$PWD/testing/corpus/java-library-kotlin-dsl" --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-single-minimal-runbuild --verbose` passed 1/1, no fallback, 12/12 task parity, output inventory/hash/archive parity, observed wall time upstream=2854ms and substrate=3311ms.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-authoritative-minimal-runbuild --verbose` passed 21/21, no fallback, 200/200 task parity, output inventory/hash/archive parity, observed wall time upstream=64023ms and substrate=68744ms.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/external-manifest.json --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-external-minimal-runbuild --verbose` passed 2/2, no fallback, 25/25 task parity, output inventory/hash/archive parity, observed wall time upstream=7015ms and substrate=7948ms.
- `./gradlew :rust-bridge:test --tests org.gradle.internal.rustbridge.SubstrateLifecycleTest --no-daemon --console=plain` passed after gating unrelated bridge lifecycle listeners off the explicit Rust `RunBuild` path.
- `./gradlew :distributions-full:install -Pgradle_installPath=$PWD/build/gradle-under-test --no-daemon --console=plain` rebuilt the Gradle-under-test distribution with the explicit `RunBuild` lifecycle gating.
- `python3 tools/demo/first_60_seconds.py --mode fast --skip-build --output build/first60-lifecycle-gated-installed.json` passed against the rebuilt distribution with daemon socket ready=1451.5ms (daemon self-report 6ms), authoritative Rust DAG=16797.9ms with cold 13977.5ms/11 Rust tasks, warm 2810.6ms/11 Rust tasks, Gradle configuration-cache reuse on the warm run, 7 Rust up-to-date skips, 4 Rust no-source/skipped tasks, 0 JVM forwards, authoritative output SHA-256 `92cf8132cd7364798a080c27ca6c160811e962cab18bb92f2872f061878a8490`, real-build dependency read-through=8150.7ms with remote requests avoided=7/7, and file-watch first event=13ms.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-authoritative-lifecycle-gated --verbose` passed 21/21, no fallback, 200/200 task parity, output inventory/hash/archive parity, observed wall time upstream=122649ms and substrate=67367ms.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/external-manifest.json --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-external-lifecycle-gated --verbose` passed 2/2, no fallback, 25/25 task parity, output inventory/hash/archive parity, observed wall time upstream=20507ms and substrate=8968ms.
- `cargo test -p gradle-substrate-daemon symlink -- --nocapture` passed after promoting simple file symlink Copy/Zip inputs and adding directory-symlink fail-closed executor coverage.
- `python3 -m unittest tools.corpus_runner.test_run` passed after moving the file-symlink Copy/Zip corpus projects from unsupported to supported.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --contract-only --output-dir build/corpus-contract-symlink-supported` passed 23/23 supported corpus contract checks.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/unsupported-manifest.json --contract-only --output-dir build/corpus-contract-unsupported-after-symlink` passed 4/4 unsupported corpus contract checks.
- `python3 tools/corpus_runner/run.py --project "$PWD/testing/corpus/copy-symlink-unsupported-kotlin-dsl" --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-copy-symlink-native --verbose` passed 1/1, no fallback, 5/5 task parity, output inventory/hash parity, observed wall time upstream=3435ms and substrate=3556ms.
- `python3 tools/corpus_runner/run.py --project "$PWD/testing/corpus/archive-symlink-unsupported-kotlin-dsl" --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-archive-symlink-native --verbose` passed 1/1, no fallback, 5/5 task parity, output inventory/hash/archive parity, observed wall time upstream=3799ms and substrate=3564ms.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --timeout 300 --output-dir build/corpus-authoritative-symlink-native --verbose` passed 23/23, no fallback, 210/210 task parity, output inventory/hash/archive parity, observed wall time upstream=62712ms and substrate=69354ms.
- `./gradlew :rust-bridge:test --tests org.gradle.internal.rustbridge.jvmhost.ProjectModelProviderAdapterTest.capturesStaticEachFileRelativePathRewriteAsNativeReadyMappings --no-daemon --console=plain` passed after adding the narrow static `eachFile` relative-path mapping capture.
- `python3 -m unittest tools.corpus_runner.test_run` passed after moving the static `eachFile` corpus project from unsupported to supported.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --contract-only --output-dir build/corpus-contract-eachfile-supported` passed 24/24 supported corpus contract checks.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/unsupported-manifest.json --contract-only --output-dir build/corpus-contract-unsupported-after-eachfile` passed 3/3 unsupported corpus contract checks.
- `./gradlew :distributions-full:install -Pgradle_installPath=$PWD/build/gradle-under-test --no-daemon --console=plain` rebuilt the Gradle-under-test install image with the static `eachFile` bridge contract.
- `python3 tools/corpus_runner/run.py --project "$PWD/testing/corpus/copy-eachfile-unsupported-kotlin-dsl" --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-copy-eachfile-native-final --verbose` passed 1/1, no fallback, 5/5 task parity, output inventory/hash parity, observed wall time upstream=9862ms and substrate=2713ms.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --timeout 300 --output-dir build/corpus-authoritative-eachfile-native-final --verbose` passed 24/24, no fallback, 215/215 task parity, output inventory/hash/archive parity, observed wall time upstream=122001ms and substrate=72184ms.
- `./gradlew :rust-bridge:test --tests org.gradle.internal.rustbridge.jvmhost.ProjectModelProviderAdapterTest.capturesStaticLineReplaceFilterAsNativeReady --no-daemon --console=plain` passed after adding the narrow static `filter { line.replace(...) }` capture.
- `cargo test -p gradle-substrate-daemon test_copy_applies_static_line_replace_filter -- --nocapture` passed after adding Rust-side literal replacement in the Copy executor.
- `python3 -m unittest tools.corpus_runner.test_run` passed after moving the static copy-filter corpus project from unsupported to supported.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --contract-only --output-dir build/corpus-contract-copy-filter-supported` passed 25/25 supported corpus contract checks.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/unsupported-manifest.json --contract-only --output-dir build/corpus-contract-unsupported-after-copy-filter` passed 2/2 unsupported corpus contract checks.
- `./gradlew :distributions-full:install -Pgradle_installPath=$PWD/build/gradle-under-test --no-daemon --console=plain` rebuilt the Gradle-under-test install image with the static copy-filter bridge contract.
- `python3 tools/corpus_runner/run.py --project "$PWD/testing/corpus/copy-filter-unsupported-kotlin-dsl" --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-copy-filter-native --verbose` passed 1/1, no fallback, 5/5 task parity, output inventory/hash parity, observed wall time upstream=10245ms and substrate=3285ms.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --timeout 300 --output-dir build/corpus-authoritative-copy-filter-native --verbose` passed 25/25, no fallback, 220/220 task parity, output inventory/hash/archive parity, observed wall time upstream=127333ms and substrate=75563ms.
- `python3 tools/corpus_runner/run.py --project "$PWD/testing/corpus/copy-filter-unsupported-kotlin-dsl" --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-copy-filter-native-final --verbose` passed 1/1 after rebuilding the install image from the final source, no fallback, 5/5 task parity, output inventory/hash parity, observed wall time upstream=10153ms and substrate=2738ms.
- `./gradlew :rust-bridge:test --tests org.gradle.internal.rustbridge.jvmhost.ProjectModelProviderAdapterTest.capturesNativeReadyTestExecContractFromTaskModel --tests org.gradle.internal.rustbridge.jvmhost.ProjectModelProviderAdapterTest.capturesClassNameTestIncludeAndExcludeFiltersAsNativeReady --no-daemon --console=plain` passed after promoting class-name Test include/exclude filters to native-ready contracts.
- `cargo test -p gradle-substrate-daemon test_build_command_test_filter -- --nocapture`, `cargo test -p gradle-substrate-daemon test_build_command_include_and_exclude_test_filters -- --nocapture`, `cargo test -p gradle-substrate-daemon test_gradle_test_pattern_to_regex_escapes_regex_metacharacters -- --nocapture`, and `cargo test -p gradle-substrate-daemon test_test_contract_lowers_to_native_test_exec_with_context_options -- --nocapture` passed after mapping Gradle class-name filters to JUnit Platform include/exclude classname regexes.
- `python3 -m unittest tools.corpus_runner.test_run` passed after moving the class-only Test filter corpus project from unsupported to supported and adding a method-level Test filter project that remains fail-closed.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --contract-only --output-dir build/corpus-contract-test-filters-supported` passed 26/26 supported corpus contract checks.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/unsupported-manifest.json --contract-only --output-dir build/corpus-contract-unsupported-after-test-filters` passed 2/2 unsupported corpus contract checks.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/external-manifest.json --contract-only --output-dir build/corpus-contract-external-test-filters` passed 2/2 external corpus contract checks.
- `./gradlew :distributions-full:install -Pgradle_installPath=$PWD/build/gradle-under-test --no-daemon --console=plain` rebuilt the Gradle-under-test install image with the Test filter bridge contract.
- `python3 tools/corpus_runner/run.py --project "$PWD/testing/corpus/test-filters-unsupported-kotlin-dsl" --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-test-filters-native --verbose` passed 1/1, no fallback, 12/12 task parity, output inventory/hash/archive parity, observed wall time upstream=14584ms and substrate=4892ms.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/external-manifest.json --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --tasks clean build --timeout 300 --output-dir build/corpus-external-test-filters-native --verbose` passed 2/2, no fallback, 25/25 task parity, output inventory/hash/archive parity, observed wall time upstream=23368ms and substrate=9476ms. The JUnit sample includes a failing legacy test class that must be excluded by the native class-name filter.
- `python3 tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --gradle-command "$PWD/build/gradle-under-test/bin/gradle" --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --timeout 300 --output-dir build/corpus-authoritative-test-filters-native --verbose` passed 26/26, no fallback, 232/232 task parity, output inventory/hash/archive parity, observed wall time upstream=147796ms and substrate=83649ms.
- `cargo test -p gradle-wrapper` passed after adding wrapper-level Rust substrate CLI modes and minimal DAG plus dependency transport/read-through flag injection tests.
- `cargo build -p gradle-wrapper` built the native wrapper binary, and `GRADLEW_DISTRIBUTION_DIR=$PWD/build/gradle-under-test target/debug/gradlew --rust-substrate-authoritative -p testing/corpus/java-library-kotlin-dsl clean build --no-daemon --console=plain --info` plus the same command with `--rust-substrate` both executed 12 Gradle tasks through Rust RunBuild, skipped the JVM task executor, and finished successfully. `GRADLEW_DISTRIBUTION_DIR=$PWD/build/gradle-under-test target/debug/gradlew --rust-substrate -p testing/corpus/java-junit-kotlin-dsl clean build --no-daemon --console=plain --info` also executed 12 external-dependency-backed tasks through Rust RunBuild with the JVM task executor skipped.
- `cargo test -p gradle-wrapper` passed after adding Rust wrapper distribution URL validation and local `file:/` distribution copy support.
- `cargo test -p gradle-wrapper` passed after adding Rust wrapper daemon prewarm and endpoint-file identity tests.
- `cargo build -p gradle-wrapper` passed, and `GRADLE_SUBSTRATE_STATE_DIR=$PWD/build/wrapper-prewarm-state GRADLEW_DISTRIBUTION_DIR=$PWD/build/gradle-under-test target/debug/gradlew --rust-substrate-authoritative -p testing/corpus/java-library-kotlin-dsl clean build --no-daemon --console=plain --info` showed the JVM bridge `Connecting to existing daemon at tcp://...`, then Rust RunBuild executed 12 tasks with JVM fallback disabled and the JVM task executor skipped.
- `cargo test -p gradle-substrate-daemon test_persistent_store_roundtrip -- --nocapture`
- `cargo test -p gradle-substrate-daemon test_add_artifact_to_cache -- --nocapture`
- `cargo test -p gradle-substrate-daemon test_artifact_cache_rejects_missing_local_file -- --nocapture`
- `cargo test -p gradle-substrate-daemon test_warm_artifact_cache_miss_when_file_disappears -- --nocapture`
- `cargo test -p gradle-substrate-daemon test_warm_metadata_cache_evicts_missing_file -- --nocapture`
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

## Post-Roadmap Hardening (nki)

Scope-registry regression fixed: `BootstrapServiceImpl.init_build` now owns
build registration in `ScopeRegistry` and its `ScopeGuard` cleanup lifecycle,
synthesizing a `__synth__{build_id}` session only for Java clients that do not
provide one. RunBuild modes now enable the bootstrap lifecycle automatically so
`BuildIdHolder` is populated before task-graph capture when that lifecycle is
available; the authoritative `BuildWorkExecutor` also lazily initializes a
scoped Rust build id when the lifecycle holder is empty, refreshes the finalized
plan under that id, and completes the scope after Rust execution. `DagExecutorServiceImpl.start_build`
rejects unregistered builds when scope validation is enabled, rather than
auto-registering synthetic scopes, and `BuildInitServiceImpl` no longer creates
DAG-visible scope entries without a completion guard.

Authoritative parity gates after fix (2026-05-03):

| Gate | Result | Command |
|------|--------|---------|
| Supported corpus | 26/26 matched, 232/232 task parity | `python3 tools/corpus_runner/run.py --manifest testing/corpus/manifest.json --gradle-command $GRADLE_UNDER_TEST/bin/gradle --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --output-dir build/corpus-authoritative-scope-contract-2` |
| External corpus | 2/2 matched, 25/25 task parity | `python3 tools/corpus_runner/run.py --manifest testing/corpus/external-manifest.json --gradle-command $GRADLE_UNDER_TEST/bin/gradle --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --output-dir build/corpus-external-scope-contract-2` |
| Unsupported contract | 2/2 passed | `python3 tools/corpus_runner/run.py --manifest testing/corpus/unsupported-manifest.json --contract-only --output-dir build/corpus-contract-unsupported-scope-contract-2` |
| Unit tests | 1600 passed, 0 failed, 3 ignored | `cargo test -p gradle-substrate-daemon --lib` |
| Focused JVM tests | Passed | `./gradlew :core:test --tests org.gradle.execution.RustAuthoritativeBuildExecutionActionTest :rust-bridge:test --tests org.gradle.internal.rustbridge.taskgraph.TaskGraphShadowListenerTest --no-daemon --console=plain` |
| Integration tests | 49 passed, 2 pre-existing failures | `cargo test --test integration_test` |

Audit of closed roadmap (dyy.1–dyy.20): 18/21 verified against committed code and
corpus parity, 3/21 partially verified (dependency solver complex scenarios,
publication/signing completeness, Tooling API/IDE shim). No critical overclaims.
Partially verified areas are documented as non-hot-path in the native-ready contract policy.

## Dependency Resolution Hardening (sh7)

Dependency resolution audit (sh7.1): the Rust daemon has executable support for
static Maven metadata/artifact transport, persistent artifact/metadata cache,
checksum verification, cache-first/read-through transport, static prefetch, selected
dynamic metadata lookups, and a growing POM transitive-resolution model. Full Gradle
solver parity is not claimed. Version conflict behavior, exclusions, dependency
constraints, BOM/platform semantics, rich versions, SNAPSHOTs, repository/auth/proxy
edge cases, dependency substitution, component metadata rules, capabilities, and
variants remain partially verified or unsupported unless covered by a checked-in gate.

External dependency corpus expanded from 2 to 5 projects (sh7.2): added
`dependency-version-conflict-kotlin-dsl` (slf4j-api+logback-classic version conflict),
`dependency-transitive-chain-kotlin-dsl` (commons-compress+gson deep transitive chains),
`dependency-platform-bom-kotlin-dsl` (okhttp-bom platform version management).
These projects are now checked into the manifest; their final no-fallback task/output
parity is recorded below.

Dependency transport cache proof (sh7.3): `first_60_seconds.py --mode proof`
remains the required proof command for transport/store/checksum, metadata cache,
dynamic metadata cache, static Maven prefetch, and read-through behavior.

Fail-closed dependency feature gates (sh7.4): Rust now rejects SNAPSHOT artifact
prefetch, incomplete Maven artifact coordinates, empty version selectors, malformed
range selectors, and unsupported `+` wildcard selectors instead of treating them as
exact versions. Broader unsupported Gradle semantics remain gated by follow-up tests
before native-ready coverage may expand.

Dependency graph observability (sh7.5): the corpus runner can emit a lightweight
declared dependency graph for upstream/substrate runs and fail on declared-graph drift
with `--dependency-graph-parity`. This is intentionally not full resolved solver graph
parity; it records requested coordinates, selected static versions when declared,
opaque managed-version entries, unsupported feature markers, and explicit limitations.

Resolved dependency graph observability (solver roadmap): the corpus runner can also
emit Gradle public `ResolutionResult` graph artifacts with
`--resolved-dependency-graph-parity`. This compares reference and substrate
invocations at the resolved Gradle graph level: configurations, selected components,
transitive edges, selection reasons, variant attributes, and artifact file names/IDs.
This is the gate to use while reading Gradle dependency-management code and porting
bounded semantics into Rust. It still does not prove direct Rust solver parity because
the substrate graph is exported from Gradle's public resolution API during the
substrate invocation; repository source/checksum policy are not exposed by that API.

Execution-kernel dependency admission (kernel roadmap): `KernelBuildPlan` now has
an optional `KernelDependencyGraph` with configurations, repositories, dependency
requests, constraints, and explicit unsupported-feature markers. Admission rejects
empty coordinates, unsupported repository URLs, dynamic `+`/`latest.*` selectors,
SNAPSHOT selectors, duplicate/unnamed configurations, and unsupported feature
markers before task scheduling. Task-graph resolution now carries canonical
`BuildPlanDependency` entries from the build-plan shadow into `StartBuildResponse`,
and `DagExecutor` groups them into the kernel dependency graph before no-fallback
RunBuild admission. Repository capture, constraints, attributes, variants, and
resolved-artifact ownership are still narrower than full Gradle solver parity, so
dependency parity/read-through remain separate gates.

| Gate | Result | Command |
|------|--------|---------|
| External corpus (expanded + declared graph parity) | 5/5 matched, 64/64 task parity, 5/5 no-fallback, graph diff files emitted under `/tmp/corpus-external-sh7-final/dependency-graphs` | `python3 tools/corpus_runner/run.py --manifest testing/corpus/external-manifest.json --gradle-command /Users/bernardoferrari/Downloads/gradle-refactor/gradle-fork/build/gradle-under-test/bin/gradle --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --dependency-graph-parity --output-dir /tmp/corpus-external-sh7-final` |
| External corpus (resolved Gradle graph parity) | 5/5 matched, 64/64 task parity, 5/5 no-fallback, 5 resolved graph diffs emitted under `/tmp/corpus-external-resolved-graph-final/resolved-dependency-graphs` | `python3 tools/corpus_runner/run.py --manifest testing/corpus/external-manifest.json --gradle-command /Users/bernardoferrari/Downloads/gradle-refactor/gradle-fork/build/gradle-under-test/bin/gradle --daemon-binary target/debug/gradle-substrate-daemon --runbuild-authoritative --resolved-dependency-graph-parity --output-dir /tmp/corpus-external-resolved-graph-final` |
| First-60s fast mode | 4/4 checks passed: daemon socket ready 622.0ms, authoritative Rust DAG 17093.4ms with 0 JVM forwards, real-build remote requests avoided 7/7, file-watch first event 13ms | `python3 tools/demo/first_60_seconds.py --mode fast --skip-build --output /tmp/first60s-fast-sh7-final.json` |
| First-60s proof mode | 6/6 dependency checks passed; metrics JSON at `/tmp/first60s-proof-sh7-final.json` | `python3 tools/demo/first_60_seconds.py --mode proof --skip-build --output /tmp/first60s-proof-sh7-final.json` |
| Fail-closed dependency tests | 4/4 focused tests passed | `cargo test -p gradle-substrate-daemon --lib -- test_prefetch_rejects_snapshot_artifacts test_incomplete_maven_coordinate_rejected test_resolve_version_range_rejects_unsupported_patterns test_resolve_dependencies_fails_closed_for_unsupported_version_selector` |
| Kernel dependency admission tests | 5/5 focused kernel tests passed | `cargo test -p gradle-substrate-daemon execution_kernel --lib` |
| Kernel dependency graph bridge tests | 2/2 focused DAG conversion tests passed | `cargo test -p gradle-substrate-daemon kernel_dependency_graph --lib` |
| Unit tests | 1588 passed, 0 failed, 3 ignored | `cargo test -p gradle-substrate-daemon --lib` |

## Next Sync Actions

1. Execute Beads roadmap `gradle-fork-dyy`: Rust owns CLI launch, daemon state,
   plan-cache hit execution, DAG scheduling, VFS/snapshots, caches, dependency
   transport/resolution, worker orchestration, and standard task execution.
2. Keep Groovy/Kotlin DSL evaluation, arbitrary `buildSrc`/JVM plugin code, and
   reflection-heavy Gradle APIs as compatibility islands behind typed,
   fail-closed contracts.
3. Prioritize first-60s visible wins: native CLI startup/prewarm, no-op VFS
   responsiveness, Rust dependency fetch/read-through, and default Rust DAG for
   native-ready standard builds.
4. Continue reducing the bridge to an IR/compatibility boundary rather than an
   execution path, with every fallback reason observable and tracked.
5. Track upstream commit synchronization in this file for each parity push.
