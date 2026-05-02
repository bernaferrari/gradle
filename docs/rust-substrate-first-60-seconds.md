# Rust Substrate: First 60 Seconds

The visible wins should be things people can feel immediately, before they
inspect a profiler:

- daemon readiness: how quickly the Rust sidecar can accept work
- authoritative Rust DAG: whether a real Gradle invocation can hand a small
  build to Rust `RunBuild` with zero JVM task forwards
- dependency transport: whether Maven artifact and metadata bytes stream through
  Rust fast, land in the Rust stores with checksum evidence, and stream back
  from those stores before HTTP on repeated requests
- static Maven prefetch: whether Rust can resolve a simple static Maven module
  and fetch its artifact into the Rust store before Gradle asks for it
- dependency read-through: whether Gradle can skip remote artifact access when
  Rust already has the requested external module artifact
- metadata read-through: whether Gradle can skip remote POM, Gradle module, and
  Maven version-list metadata access when Rust already has the metadata
- real build read-through: whether an installed Gradle build can warm Rust from
  HTTP once, including listener static direct/transitive artifact prefetch,
  then rerun from a fresh Gradle user home with zero remote requests
- hashing/fingerprinting: whether installed Gradle can run with Rust
  build-session hashing plus file-collection snapshot/fingerprint support
- file watching: how quickly an edit becomes observable

Run:

```bash
python3 tools/demo/first_60_seconds.py --mode fast --output build/first60.json
```

The Rust wrapper can also be used as the first-minute entry point after the
workspace binaries are built. `--rust-substrate` injects the Rust DAG flags plus
safe dependency transport/read-through flags and uses the native-ready-default
gate, so unsupported builds delegate back to Gradle instead of claiming native
parity. `--rust-substrate-authoritative` uses the stricter no-fallback gate for
corpus/demo validation.

```bash
cargo build -p gradle-wrapper -p gradle-substrate-daemon

GRADLEW_DISTRIBUTION_DIR=$PWD/build/gradle-under-test \
  target/debug/gradlew --rust-substrate \
  -p testing/corpus/java-library-kotlin-dsl clean build

GRADLEW_DISTRIBUTION_DIR=$PWD/build/gradle-under-test \
  target/debug/gradlew --rust-substrate-authoritative \
  -p testing/corpus/java-library-kotlin-dsl clean build
```

The wrapper still honors `gradle-wrapper.properties` for normal distribution
download/install, including SHA-256 verification when
`distributionSha256Sum` is present. When `validateDistributionUrl=true`, it
fails closed for unsupported URL schemes before fetching.

On Rust substrate runs, the wrapper also prewarms `gradle-substrate-daemon`
before launching Gradle and writes the same loopback endpoint file the JVM
bridge understands. That makes the normal Rust path `Rust wrapper -> Rust
daemon -> Gradle compatibility/configuration -> Rust RunBuild`, instead of
having the JVM bridge own sidecar startup. Set `GRADLE_SUBSTRATE_STATE_DIR` to
isolate daemon state, or `GRADLEW_RUST_PREWARM=false` to disable wrapper
prewarm while debugging.

Fast mode reports:

- `daemon_socket_ready`: process launch until the Unix socket exists
- `authoritative_rust_dag`: a copied Java-library corpus project built through
  explicit authoritative Rust `RunBuild`, then invoked again over the same
  Rust state and Gradle configuration-cache state with no source changes,
  including cold/warm timings, configuration-cache reuse, Rust task counts,
  Rust up-to-date skips, Rust no-source/skipped counts, JVM forwards, and
  deterministic output hash
- `real_build_dependency_readthrough`: an installed Gradle-under-test build resolving a local HTTP Maven `1.+` dependency plus a static Maven graph-only dependency with a transitive child and isolated Rust state; the first run warms dynamic metadata/artifact stores and listener-prefetches the static direct/transitive artifacts, while the second run deletes `build/`, uses a fresh Gradle user home, materializes all artifacts again, and reports remote requests avoided
- `file_watch_first_event`: native file watcher latency from write to event

Proof mode keeps the heavier subsystem checks out of the default visible demo:

```bash
python3 tools/demo/first_60_seconds.py --mode proof --output build/first60-proof.json
```

Proof mode reports:

- `dependency_transport_store_checksum`: local HTTP artifact streaming through the Rust dependency service, persisted store write, warm-cache and persisted-store reuse before HTTP, cache hit, checksum verification, and cache-first transport reuse
- `dependency_metadata_transport_cache`: URL-only POM streaming through the Rust
  transport, persisted metadata-store write, and later metadata cache hit
- `dependency_dynamic_metadata_transport_cache`: URL-only
  `maven-metadata.xml` streaming through the Rust transport, persisted
  metadata-store write, and later dynamic-version metadata cache hit
- `dependency_static_maven_prefetch`: opt-in Rust `ResolveDependencies`
  artifact prefetch for a static Maven module, including persisted JAR bytes,
  artifact cache hit, total download size, and SHA-256 evidence
- `dependency_artifact_readthrough`: focused Gradle resolver tests proving the Rust read-through hook resolves JAR and non-JAR artifacts before remote access
- `dependency_metadata_readthrough`: focused Gradle resource-cache tests proving the Rust POM, Gradle module, Maven version-list metadata, and uncached download hooks resolve before Java remote transport

The artifact and metadata read-through checks share one Gradle invocation so
the first-minute demo measures the Rust-backed seams instead of paying repeated
Gradle test startup overhead.

Use `--mode all` when you want both the fast path and proof harness in one
command.

The real-build read-through metric runs when `build/gradle-under-test/bin/gradle`
exists, or when `GRADLE_UNDER_TEST_BIN`/`GRADLE_UNDER_TEST` points at a local
install image from this fork. It uses `org.gradle.rust.substrate.state.dir` so
the proof is isolated from `~/.gradle-substrate`.

A broader dependency-management integration smoke also exercises real Maven
flows: dynamic-version read-through warms `maven-metadata.xml`, the selected
POM, and the artifact; static listener prefetch warms direct and transitive
artifacts from a graph-only first run so a fresh Gradle user home can
materialize them without remote repository access.

These are not full Gradle replacement claims. Gradle DSL and unsupported plugin
semantics still go through the JVM compatibility island. This harness exists to
keep the Rust work focused on perceptible first-minute wins while the broader
authoritative corpus continues to guard correctness.

For the installed-distribution hash/fingerprint smoke, build the local install
image and run a small corpus project with only the scope-safe flags enabled.
Shadow mode compares Java and Rust hashes without changing Gradle's returned
snapshots:

```bash
./gradlew :distributions-full:install \
  -Pgradle_installPath=$PWD/build/gradle-under-test \
  -Dorg.gradle.unsafe.isolated-projects=false \
  -Dorg.gradle.configuration-cache=false \
  --no-daemon --console=plain

build/gradle-under-test/bin/gradle \
  -p testing/corpus/java-library-kotlin-dsl clean classes \
  --no-daemon --console=plain --info \
  -Dorg.gradle.rust.substrate.enabled=true \
  -Dorg.gradle.rust.substrate.daemon.path=$PWD/target/debug/gradle-substrate-daemon \
  -Dorg.gradle.rust.substrate.hashing.enabled=true \
  -Dorg.gradle.rust.substrate.fingerprint.enabled=true \
  -Dorg.gradle.rust.substrate.shadow.report-mismatches=true
```

The supported authoritative slice can also materialize Gradle-compatible
file-collection snapshots from Rust hashes for direct files, missing roots,
directories, PatternSet-backed file trees, and file-tree-backed archive inputs
by hashing the backing archive file. Value snapshotting can also be made
authoritative for exact built-in value shapes while failing closed for JVM-only
serialization/classloader cases. Supported value inputs are batched into one
Rust canonical snapshot RPC per fingerprinting pass:

```bash
build/gradle-under-test/bin/gradle \
  -p testing/corpus/java-library-kotlin-dsl clean classes \
  --no-daemon --console=plain --info \
  -Dorg.gradle.rust.substrate.enabled=true \
  -Dorg.gradle.rust.substrate.daemon.path=$PWD/target/debug/gradle-substrate-daemon \
  -Dorg.gradle.rust.substrate.hashing.enabled=true \
  -Dorg.gradle.rust.substrate.hashing.authoritative=true \
  -Dorg.gradle.rust.substrate.fingerprint.enabled=true \
  -Dorg.gradle.rust.substrate.fingerprint.authoritative=true \
  -Dorg.gradle.rust.substrate.snapshotting.enabled=true \
  -Dorg.gradle.rust.substrate.snapshotting.authoritative=true
```
