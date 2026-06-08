#!/usr/bin/env bash
# Tests for the arch-detection + version-stamping helpers in
# scripts/release.sh + scripts/install.sh. Bash's `source` lets
# us pull just the functions we need without running the full
# scripts (which would try to actually build the workspace).
#
# Run: bash scripts/tests/release_arch_test.sh
set -euo pipefail

# Number of failures tracked by assert_eq below.
FAIL=0
PASS=0

assert_eq() {
    # assert_eq <actual> <expected> <label>
    if [ "$1" = "$2" ]; then
        PASS=$((PASS + 1))
        printf "  \033[32m✓\033[0m %s\n" "$3"
    else
        FAIL=$((FAIL + 1))
        printf "  \033[31m✗\033[0m %s\n" "$3"
        printf "    expected: %q\n" "$2"
        printf "    got:      %q\n" "$1"
    fi
}

assert_nonzero() {
    # assert_nonzero <rc> <label>
    if [ "$1" -ne 0 ]; then
        PASS=$((PASS + 1))
        printf "  \033[32m✓\033[0m %s (rc=%d)\n" "$2" "$1"
    else
        FAIL=$((FAIL + 1))
        printf "  \033[31m✗\033[0m %s (rc=0, expected non-zero)\n" "$2"
    fi
}

# Pull the detect_platform function out of install.sh. We
# extract the function body and `eval` it in this script. The
# install.sh's `error/info/warn/success` helpers are stubbed
# below.
SCRIPTS_DIR="$(cd "$(dirname "$0")/.." && pwd)"
ROOT_DIR="$(cd "${SCRIPTS_DIR}/.." && pwd)"
INSTALL_SH="${ROOT_DIR}/install.sh"
DETECT_PLATFORM_SRC=$(sed -n '/^detect_platform()/,/^}/p' "$INSTALL_SH")
eval "$DETECT_PLATFORM_SRC"
error() { printf "error: %s\n" "$*" >&2; exit 1; }
info()  { :; }
warn()  { :; }
success() { :; }

# Helper that sets up a fake uname and runs detect_platform.
# Writes the resulting PLATFORM / PLATFORM_OS / PLATFORM_ARCH
# to a temp file, returns the function's exit code. Avoids
# the subshell-PASS-counting trap by having the parent read
# the result file directly.
fake_uname() {
    # fake_uname <os> <arch>
    # Sets up uname to return $1 for -s and $2 for -m, then
    # runs detect_platform. Returns the function's exit code.
    uname() {
        case "$1" in
            -s) echo "$1_S" ;;  # placeholder; replaced below
            -m) echo "$1_M" ;;  # placeholder; replaced below
        esac
    }
}

run_detect() {
    # run_detect <fake_os> <fake_arch>
    # Runs detect_platform with a stubbed uname. Echoes the
    # resulting PLATFORM (or nothing on error) to stdout. The
    # return code is detect_platform's exit code.
    set +e
    (
        OS_OUT="$1"
        ARCH_OUT="$2"
        uname() {
            case "$1" in
                -s) echo "$OS_OUT" ;;
                -m) echo "$ARCH_OUT" ;;
            esac
        }
        PLATFORM_OS=""
        PLATFORM_ARCH=""
        PLATFORM=""
        detect_platform
        echo "PLATFORM=$PLATFORM"
        echo "PLATFORM_OS=$PLATFORM_OS"
        echo "PLATFORM_ARCH=$PLATFORM_ARCH"
    )
    return $?
}

# --- happy paths ---

OUT=$(run_detect "Linux" "x86_64") || true
assert_eq "$(echo "$OUT" | grep '^PLATFORM=' | cut -d= -f2)" "linux_x86_64" "x86_64 host → linux_x86_64"
assert_eq "$(echo "$OUT" | grep '^PLATFORM_OS=' | cut -d= -f2)" "linux" "x86_64 host → OS=linux"
assert_eq "$(echo "$OUT" | grep '^PLATFORM_ARCH=' | cut -d= -f2)" "x86_64" "x86_64 host → ARCH=x86_64"

OUT=$(run_detect "Linux" "amd64") || true
assert_eq "$(echo "$OUT" | grep '^PLATFORM=' | cut -d= -f2)" "linux_x86_64" "amd64 alias → linux_x86_64"
assert_eq "$(echo "$OUT" | grep '^PLATFORM_ARCH=' | cut -d= -f2)" "x86_64" "amd64 alias → ARCH=x86_64"

OUT=$(run_detect "Linux" "aarch64") || true
assert_eq "$(echo "$OUT" | grep '^PLATFORM=' | cut -d= -f2)" "linux_aarch64" "aarch64 host → linux_aarch64"
assert_eq "$(echo "$OUT" | grep '^PLATFORM_ARCH=' | cut -d= -f2)" "aarch64" "aarch64 host → ARCH=aarch64"

OUT=$(run_detect "Linux" "arm64") || true
assert_eq "$(echo "$OUT" | grep '^PLATFORM=' | cut -d= -f2)" "linux_aarch64" "arm64 alias → linux_aarch64"
assert_eq "$(echo "$OUT" | grep '^PLATFORM_ARCH=' | cut -d= -f2)" "aarch64" "arm64 alias → ARCH=aarch64"

OUT=$(run_detect "Darwin" "x86_64") || true
assert_eq "$(echo "$OUT" | grep '^PLATFORM=' | cut -d= -f2)" "darwin_x86_64" "darwin x86_64 → darwin_x86_64"

OUT=$(run_detect "Darwin" "arm64") || true
assert_eq "$(echo "$OUT" | grep '^PLATFORM=' | cut -d= -f2)" "darwin_aarch64" "darwin arm64 → darwin_aarch64"

# --- rejection paths ---

set +e
run_detect "FreeBSD" "x86_64" >/dev/null 2>&1
RC=$?
set -e
assert_nonzero "$RC" "unsupported OS (FreeBSD) errors out"

set +e
run_detect "Linux" "i686" >/dev/null 2>&1
RC=$?
set -e
assert_nonzero "$RC" "unsupported arch (i686) errors out"

# --- version-stamping helper ---

# Pull the tarball-arch-name logic out of release.sh and check
# the file name for each host arch. The release script picks
# `x86_64` for amd64 hosts and `aarch64` for arm64 hosts.
RELEASE_SH="${SCRIPTS_DIR}/release.sh"
# The arch-mapping is the only block we want to test. The Go
# pipeline used `linux_amd64`; we use `x86_64` / `aarch64` to
# match the rust target triples + the install script's
# platform string.
ARCH_FROM_SCRIPT=$(sed -n '/^HOST_ARCH=/,/^esac$/p' "$RELEASE_SH" | sed -n 's/.*"\([a-z0-9_]*\)".*/\1/p' | head -1)
# Actually the mapping is a case block; we just check that
# "x86_64" appears in the mapping (the linux_x86_64 case) and
# "aarch64" appears (the linux_aarch64 case). The exact string
# format depends on the case branches; we just check presence.
if grep -q 'TARBALL_ARCH="x86_64"' "$RELEASE_SH" && \
   grep -q 'TARBALL_ARCH="aarch64"' "$RELEASE_SH"; then
    PASS=$((PASS + 1))
    printf "  \033[32m✓\033[0m release.sh maps amd64 → x86_64 and arm64 → aarch64\n"
else
    FAIL=$((FAIL + 1))
    printf "  \033[31m✗\033[0m release.sh TARBALL_ARCH mapping missing\n"
fi

# Confirm the tarball name format is what the install script
# expects (`cleanclaw_${VERSION}_${OS}_${ARCH}.tar.gz`).
if grep -qE 'TARBALL="cleanclaw_\$\{VERSION\}_\$\{OS\}_\$\{TARBALL_ARCH\}\.tar\.gz"' "$RELEASE_SH"; then
    PASS=$((PASS + 1))
    printf "  \033[32m✓\033[0m tarball name matches install.sh's expected pattern\n"
else
    FAIL=$((FAIL + 1))
    printf "  \033[31m✗\033[0m tarball name pattern mismatch\n"
fi

echo
echo "Passed: $PASS    Failed: $FAIL"
[ "$FAIL" -eq 0 ]
