# Rust Substrate: First 60 Seconds

The visible wins should be things people can feel immediately, before they
inspect a profiler:

- daemon readiness: how quickly the Rust sidecar can accept work
- dependency transport: whether Maven artifact and metadata bytes stream through
  Rust fast and land in the Rust stores with checksum evidence
- dependency read-through: whether Gradle can skip remote artifact access when
  Rust already has the requested external module artifact
- metadata read-through: whether Gradle can skip remote POM, Gradle module, and
  Maven version-list metadata access when Rust already has the metadata
- hashing/fingerprinting: whether installed Gradle can run with Rust
  build-session hashing plus file-collection snapshot/fingerprint support
- file watching: how quickly an edit becomes observable

Run:

```bash
python3 tools/demo/first_60_seconds.py --output build/first60.json
```

The script reports:

- `daemon_socket_ready`: process launch until the Unix socket exists
- `dependency_transport_store_checksum`: local HTTP artifact streaming through the Rust dependency service, persisted store write, cache hit, and checksum verification
- `dependency_metadata_transport_cache`: URL-only POM streaming through the Rust
  transport, persisted metadata-store write, and later metadata cache hit
- `dependency_dynamic_metadata_transport_cache`: URL-only
  `maven-metadata.xml` streaming through the Rust transport, persisted
  metadata-store write, and later dynamic-version metadata cache hit
- `dependency_artifact_readthrough`: focused Gradle resolver test proving the Rust read-through hook resolves before remote access
- `dependency_metadata_readthrough`: focused Gradle resource-cache test proving the Rust POM, Gradle module, and Maven version-list metadata read-through hook resolves before remote access
- `file_watch_first_event`: native file watcher latency from write to event

A broader dependency-management integration smoke also exercises a real Maven
dynamic-version flow: the first run resolves `1.+` over HTTP and warms Rust,
then a second run with a fresh Gradle user home resolves with no remote
expectations by reading `maven-metadata.xml`, the selected POM, and the artifact
from Rust stores.

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
