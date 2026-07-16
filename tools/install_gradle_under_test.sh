#!/usr/bin/env bash
# Build a local Gradle-under-test image with the Rust substrate daemon binaries
# and an up-to-date rust-bridge jar.
#
# Default mode runs a full :distributions-full:install into
# build/gradle-under-test (preferred).
#
# Fast iteration mode refreshes only the bridge jar + daemon binaries on top of
# an existing install:
#   INSTALL_MODE=bridge-only ./tools/install_gradle_under_test.sh
#
# Options:
#   --release              Build Rust binaries in release mode
#   --mode full|bridge-only
#   -h, --help             Show usage
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

INSTALL_DIR="${GRADLE_UNDER_TEST:-$ROOT/build/gradle-under-test}"
INSTALL_MODE="${INSTALL_MODE:-full}"
CARGO_PROFILE="debug"
CARGO_FLAGS=()
GRADLE_EXTRA_ARGS=()

usage() {
  cat <<EOF
Usage: $(basename "$0") [--release] [--mode full|bridge-only]

Install a local Gradle distribution under build/gradle-under-test (or
\$GRADLE_UNDER_TEST) with the rust-bridge jar and substrate daemon binaries
that DaemonLauncher discovers.

Modes:
  full         (default) cargo build + ./gradlew :rust-bridge:jar
               :distributions-full:install, then overlay daemon binaries.
               Preferred — produces a complete install image.
  bridge-only  Requires an existing install. Rebuilds rust-bridge jar and
               daemon binaries, then copies them into the install. Faster
               iteration; full install is still preferred for CI/dogfood.

Environment:
  INSTALL_MODE=full|bridge-only   Same as --mode
  GRADLE_UNDER_TEST=DIR           Install root (default: \$ROOT/build/gradle-under-test)
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --release)
      CARGO_PROFILE="release"
      CARGO_FLAGS+=(--release)
      shift
      ;;
    --mode)
      INSTALL_MODE="${2:-}"
      if [[ -z "$INSTALL_MODE" ]]; then
        echo "error: --mode requires full or bridge-only" >&2
        exit 2
      fi
      shift 2
      ;;
    --mode=*)
      INSTALL_MODE="${1#--mode=}"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

case "$INSTALL_MODE" in
  full|bridge-only) ;;
  *)
    echo "error: INSTALL_MODE must be 'full' or 'bridge-only' (got: $INSTALL_MODE)" >&2
    exit 2
    ;;
esac

if [[ ! -x "$ROOT/gradlew" && ! -f "$ROOT/gradlew" ]]; then
  echo "error: gradlew not found at $ROOT/gradlew" >&2
  echo "error: run this script from a full gradle-fork checkout that includes the Gradle wrapper." >&2
  exit 1
fi
if [[ ! -x "$ROOT/gradlew" ]]; then
  chmod +x "$ROOT/gradlew" 2>/dev/null || true
fi
if [[ ! -x "$ROOT/gradlew" ]]; then
  echo "error: gradlew exists but is not executable: $ROOT/gradlew" >&2
  exit 1
fi

die() {
  echo "error: $*" >&2
  exit 1
}

log() {
  echo "== $*"
}

detect_platform() {
  local os_name os_arch os arch
  os_name="$(uname -s | tr '[:upper:]' '[:lower:]')"
  os_arch="$(uname -m | tr '[:upper:]' '[:lower:]')"
  case "$os_name" in
    darwin*) os="macos" ;;
    linux*) os="linux" ;;
    mingw*|msys*|cygwin*) os="windows" ;;
    *) os="$os_name" ;;
  esac
  case "$os_arch" in
    arm64|aarch64) arch="aarch64" ;;
    x86_64|amd64) arch="x86_64" ;;
    *) arch="$os_arch" ;;
  esac
  printf '%s-%s\n' "$os" "$arch"
}

build_rust_bins() {
  log "Building gradle-substrate-daemon bins ($CARGO_PROFILE)"
  cargo build -p gradle-substrate-daemon --bins "${CARGO_FLAGS[@]+"${CARGO_FLAGS[@]}"}"
}

rust_target_dir() {
  printf '%s/target/%s\n' "$ROOT" "$CARGO_PROFILE"
}

require_rust_bins() {
  local dir daemon runbuild
  dir="$(rust_target_dir)"
  daemon="$dir/gradle-substrate-daemon"
  runbuild="$dir/gradle-substrate-runbuild"
  case "$(uname -s | tr '[:upper:]' '[:lower:]')" in
    mingw*|msys*|cygwin*)
      daemon="${daemon}.exe"
      runbuild="${runbuild}.exe"
      ;;
  esac
  [[ -f "$daemon" ]] || die "missing daemon binary: $daemon"
  [[ -f "$runbuild" ]] || die "missing runbuild binary: $runbuild"
  DAEMON_SRC="$daemon"
  RUNBUILD_SRC="$runbuild"
}

# DaemonLauncher.resolveBinary prefers:
#   $install/lib/substrate/gradle-substrate-daemon-$platform[.exe]
# and falls back to:
#   $install/lib/gradle-substrate-daemon
# RustDaemonSidecarLauncher also looks at lib/gradle-substrate-daemon.
install_daemon_binaries() {
  local daemon_src="$1"
  local runbuild_src="$2"
  local lib_dir="$INSTALL_DIR/lib"
  local substrate_dir="$lib_dir/substrate"
  local platform
  platform="$(detect_platform)"

  mkdir -p "$substrate_dir"

  local daemon_name runbuild_name
  daemon_name="$(basename "$daemon_src")"
  runbuild_name="$(basename "$runbuild_src")"

  # Generic fallback path used by DaemonLauncher / sidecar launcher.
  cp -f "$daemon_src" "$lib_dir/$daemon_name"
  chmod +x "$lib_dir/$daemon_name"
  cp -f "$runbuild_src" "$lib_dir/$runbuild_name"
  chmod +x "$lib_dir/$runbuild_name"

  # Platform-specific path preferred by DaemonLauncher.resolveBinary.
  local platform_daemon="$substrate_dir/gradle-substrate-daemon-${platform}"
  if [[ "$daemon_name" == *.exe ]]; then
    platform_daemon="${platform_daemon}.exe"
  fi
  cp -f "$daemon_src" "$platform_daemon"
  chmod +x "$platform_daemon"

  # Keep runbuild beside the platform-specific daemon for tooling discovery.
  local platform_runbuild="$substrate_dir/gradle-substrate-runbuild-${platform}"
  if [[ "$runbuild_name" == *.exe ]]; then
    platform_runbuild="${platform_runbuild}.exe"
  fi
  cp -f "$runbuild_src" "$platform_runbuild"
  chmod +x "$platform_runbuild"

  log "Installed daemon binaries:"
  echo "  $lib_dir/$daemon_name"
  echo "  $platform_daemon"
  echo "  $lib_dir/$runbuild_name"
  echo "  $platform_runbuild"
}

latest_bridge_jar() {
  local libs="$ROOT/platforms/core-execution/rust-bridge/build/libs"
  local jar=""
  local candidate
  # Prefer the version that matches version.txt when present.
  if [[ -f "$ROOT/version.txt" ]]; then
    local ver
    ver="$(tr -d '[:space:]' <"$ROOT/version.txt")"
    candidate="$libs/gradle-rust-bridge-${ver}.jar"
    if [[ -f "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  fi
  # Otherwise take the newest non-test-fixtures jar.
  # shellcheck disable=SC2012
  jar="$(ls -t "$libs"/gradle-rust-bridge-*.jar 2>/dev/null | grep -v test-fixtures | head -1 || true)"
  [[ -n "$jar" && -f "$jar" ]] || die "no gradle-rust-bridge-*.jar found under $libs (run :rust-bridge:jar first)"
  printf '%s\n' "$jar"
}

install_bridge_jar() {
  local src_jar dest_name dest_jar props lib_dir old
  src_jar="$(latest_bridge_jar)"
  dest_name="$(basename "$src_jar")"
  lib_dir="$INSTALL_DIR/lib"
  dest_jar="$lib_dir/$dest_name"
  props="$lib_dir/gradle-rust-bridge.properties"

  [[ -d "$lib_dir" ]] || die "install lib dir missing: $lib_dir"

  # Remove previous bridge jars so the install does not keep a stale version.
  shopt -s nullglob
  for old in "$lib_dir"/gradle-rust-bridge-*.jar; do
    if [[ "$(basename "$old")" != "$dest_name" ]]; then
      rm -f "$old"
    fi
  done
  shopt -u nullglob

  cp -f "$src_jar" "$dest_jar"
  log "Installed rust-bridge jar: $dest_jar"

  if [[ -f "$props" ]]; then
    if grep -q '^jarFile=' "$props"; then
      # Rewrite jarFile= while preserving the rest of the properties file.
      local tmp
      tmp="$(mktemp)"
      sed "s|^jarFile=.*|jarFile=${dest_name}|" "$props" >"$tmp"
      mv "$tmp" "$props"
    else
      printf 'jarFile=%s\n' "$dest_name" >>"$props"
    fi
    log "Updated $props -> jarFile=${dest_name}"
  else
    printf 'jarFile=%s\n' "$dest_name" >"$props"
    log "Wrote $props -> jarFile=${dest_name}"
  fi
}

verify_install_layout() {
  local bin="$INSTALL_DIR/bin/gradle"
  [[ -x "$bin" || -f "$bin" ]] || die "install incomplete: missing $bin"
  [[ -f "$INSTALL_DIR/lib/gradle-substrate-daemon" || -f "$INSTALL_DIR/lib/gradle-substrate-daemon.exe" ]] \
    || die "install incomplete: daemon binary missing under $INSTALL_DIR/lib"
  local bridge_count
  bridge_count="$(ls "$INSTALL_DIR"/lib/gradle-rust-bridge-*.jar 2>/dev/null | grep -v test-fixtures | wc -l | tr -d ' ')"
  [[ "$bridge_count" -ge 1 ]] || die "install incomplete: no gradle-rust-bridge-*.jar under $INSTALL_DIR/lib"
}

run_full_install() {
  log "Full install into $INSTALL_DIR (preferred)"
  build_rust_bins

  # Match docs/rust-substrate-first-60-seconds.md install flags.
  log "Running :rust-bridge:jar :distributions-full:install"
  ./gradlew \
    :rust-bridge:jar \
    :distributions-full:install \
    -Pgradle_installPath="$INSTALL_DIR" \
    -Dorg.gradle.unsafe.isolated-projects=false \
    -Dorg.gradle.configuration-cache=false \
    --no-daemon \
    --console=plain \
    ${GRADLE_EXTRA_ARGS[@]+"${GRADLE_EXTRA_ARGS[@]}"}

  require_rust_bins

  # Always overlay freshly built daemon bins — packaging copySubstrateBinary
  # only picks up release binaries under substrate/target/<triple>/release.
  install_daemon_binaries "$DAEMON_SRC" "$RUNBUILD_SRC"
  # Ensure the jar that landed in the install matches the just-built bridge.
  install_bridge_jar
}

run_bridge_only() {
  log "bridge-only refresh of $INSTALL_DIR"
  echo "note: full install is preferred; bridge-only only refreshes jar + daemon binaries."
  [[ -d "$INSTALL_DIR" ]] || die "bridge-only requires an existing install at $INSTALL_DIR (run without INSTALL_MODE=bridge-only first)"
  [[ -x "$INSTALL_DIR/bin/gradle" || -f "$INSTALL_DIR/bin/gradle" ]] \
    || die "bridge-only requires $INSTALL_DIR/bin/gradle — run a full install first"

  build_rust_bins

  log "Building :rust-bridge:jar"
  ./gradlew \
    :rust-bridge:jar \
    -Dorg.gradle.unsafe.isolated-projects=false \
    -Dorg.gradle.configuration-cache=false \
    --no-daemon \
    --console=plain \
    ${GRADLE_EXTRA_ARGS[@]+"${GRADLE_EXTRA_ARGS[@]}"}

  require_rust_bins
  install_bridge_jar
  install_daemon_binaries "$DAEMON_SRC" "$RUNBUILD_SRC"
}

case "$INSTALL_MODE" in
  full) run_full_install ;;
  bridge-only) run_bridge_only ;;
esac

verify_install_layout

GRADLE_UNDER_TEST_BIN="$INSTALL_DIR/bin/gradle"
export GRADLE_UNDER_TEST_BIN
export GRADLE_UNDER_TEST="$INSTALL_DIR"

echo
echo "Gradle under test ready."
echo "  GRADLE_UNDER_TEST=$INSTALL_DIR"
echo "  GRADLE_UNDER_TEST_BIN=$GRADLE_UNDER_TEST_BIN"
echo "  daemon=$INSTALL_DIR/lib/gradle-substrate-daemon"
echo "  mode=$INSTALL_MODE profile=$CARGO_PROFILE"
echo
echo "Example:"
echo "  export GRADLE_UNDER_TEST_BIN=$GRADLE_UNDER_TEST_BIN"
echo "  \"\$GRADLE_UNDER_TEST_BIN\" -p testing/corpus/java-library-kotlin-dsl help --no-daemon"
