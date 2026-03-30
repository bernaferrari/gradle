#!/usr/bin/env bash
# Launch wrapper for the Gradle substrate daemon.
# Detects OS and architecture, finds the platform-specific binary,
# and falls back gracefully if missing.

set -euo pipefail

detect_platform() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)   os="linux" ;;
        Darwin)  os="macos" ;;
        MINGW*|MSYS*|CYGWIN*) os="windows" ;;
        *)
            echo "[substrate] Unsupported OS: $os" >&2
            exit 0
            ;;
    esac

    case "$arch" in
        x86_64|amd64)  arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *)
            echo "[substrate] Unsupported architecture: $arch" >&2
            exit 0
            ;;
    esac

    echo "${os}-${arch}"
}

find_binary() {
    local platform="$1"

    # 1. Try platform-specific path in GRADLE_HOME
    if [ -n "${GRADLE_HOME:-}" ]; then
        local candidate="${GRADLE_HOME}/lib/substrate/gradle-substrate-daemon-${platform}"
        if [ -f "$candidate" ]; then
            echo "$candidate"
            return 0
        fi
    fi

    # 2. Try generic path in GRADLE_HOME
    if [ -n "${GRADLE_HOME:-}" ]; then
        local candidate="${GRADLE_HOME}/lib/gradle-substrate-daemon"
        if [ -f "$candidate" ]; then
            echo "$candidate"
            return 0
        fi
    fi

    # 3. Try alongside this script
    local script_dir
    script_dir="$(cd "$(dirname "$0")" && pwd)"
    local candidate="${script_dir}/../lib/substrate/gradle-substrate-daemon-${platform}"
    if [ -f "$candidate" ]; then
        echo "$candidate"
        return 0
    fi

    # 4. Try PATH
    if command -v gradle-substrate-daemon >/dev/null 2>&1; then
        command -v gradle-substrate-daemon
        return 0
    fi

    return 1
}

main() {
    local platform binary

    platform="$(detect_platform)"
    if ! binary="$(find_binary "$platform")"; then
        echo "[substrate] Daemon binary not found for ${platform}. Running without substrate." >&2
        exit 0
    fi

    if [ ! -x "$binary" ]; then
        chmod +x "$binary"
    fi

    exec "$binary" "$@"
}

main "$@"
