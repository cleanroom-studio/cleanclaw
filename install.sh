#!/bin/sh
# CleanClaw Installer.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/cleanroom-studio/cleanclaw/main/install.sh | sh
# Or:
#   CLEANCLAW_INSTALL_DIR=~/bin curl -fsSL ... | sh
#
# Resolves the latest release from GitHub, downloads the
# prebuilt tarball, extracts it into the install dir, and
# ensures the dir is on PATH (for `~/.local/bin`).
set -e

REPO="cleanroom-studio/cleanclaw"
BINARY="cleanclaw"

# Colors (only if terminal supports it)
if [ -t 1 ]; then
    RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; BOLD='\033[1m'; NC='\033[0m'
else
    RED=''; GREEN=''; YELLOW=''; BOLD=''; NC=''
fi

info()    { printf "${GREEN}[INFO]${NC} %s\n" "$*"; }
warn()    { printf "${YELLOW}[WARN]${NC} %s\n" "$*"; }
error()   { printf "${RED}[ERROR]${NC} %s\n" "$*" >&2; exit 1; }
success() { printf "${GREEN}[✓]${NC} %s\n" "$*"; }

# ── Platform detection ──────────────────────────────────────────────────────
detect_platform() {
    OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
    ARCH="$(uname -m)"
    case "$OS" in
        linux)  PLATFORM_OS="linux" ;;
        darwin) PLATFORM_OS="darwin" ;;
        *)      error "Unsupported OS: $OS (only linux + darwin are supported)" ;;
    esac
    case "$ARCH" in
        x86_64|amd64)   PLATFORM_ARCH="x86_64" ;;
        aarch64|arm64)  PLATFORM_ARCH="aarch64" ;;
        *)              error "Unsupported arch: $ARCH" ;;
    esac
    PLATFORM="${PLATFORM_OS}_${PLATFORM_ARCH}"
}

# ── Install dir ─────────────────────────────────────────────────────────────
choose_install_dir() {
    # 1. explicit override
    if [ -n "${CLEANCLAW_INSTALL_DIR:-}" ]; then
        INSTALL_DIR="$CLEANCLAW_INSTALL_DIR"
        return
    fi
    # 2. system dirs that are already on PATH
    if [ -w /usr/local/bin ]; then
        INSTALL_DIR="/usr/local/bin"
        return
    fi
    # 3. user-local fallback (no sudo needed)
    INSTALL_DIR="$HOME/.local/bin"
    mkdir -p "$INSTALL_DIR"
}

# ── Version ─────────────────────────────────────────────────────────────────
get_latest_version() {
    if [ -n "${CLEANCLAW_VERSION:-}" ]; then
        VERSION="$CLEANCLAW_VERSION"
        return
    fi
    info "Resolving latest release from GitHub…"
    VERSION=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
        | grep '"tag_name"' | head -1 | sed -E 's/.*"v?([^"]+)".*/\1/')
    if [ -z "$VERSION" ]; then
        error "Could not resolve latest version. Set CLEANCLAW_VERSION=v0.1.0 explicitly."
    fi
}

# ── Download + extract ────────────────────────────────────────────────────
install_binary() {
    TMP_DIR="$(mktemp -d)"
    TARBALL="cleanclaw_v${VERSION}_${PLATFORM}.tar.gz"
    URL="https://github.com/${REPO}/releases/download/v${VERSION}/${TARBALL}"

    info "Downloading ${BINARY} ${VERSION} (${PLATFORM})…"

    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$URL" -o "${TMP_DIR}/${TARBALL}" || error "Download failed: $URL"
    else
        wget -q "$URL" -O "${TMP_DIR}/${TARBALL}" || error "Download failed: $URL"
    fi

    info "Extracting…"
    tar -xzf "${TMP_DIR}/${TARBALL}" -C "${TMP_DIR}"

    # Atomic replace: backup old → move new → remove backup
    DEST="${INSTALL_DIR}/${BINARY}"
    if [ -f "$DEST" ]; then
        mv "$DEST" "${DEST}.bak"
    fi
    mv "${TMP_DIR}/${BINARY}" "$DEST"
    chmod +x "$DEST"
    rm -f "${DEST}.bak"

    # Plugin binaries.
    for plugin in cleanclaw-plugin-demo cleanclaw-mem0 \
                  cleanclaw-post-turn-echo cleanclaw-openclaw-demo; do
        if [ -f "${TMP_DIR}/${plugin}" ]; then
            mv "${TMP_DIR}/${plugin}" "${INSTALL_DIR}/${plugin}"
            chmod +x "${INSTALL_DIR}/${plugin}"
        fi
    done

    rm -rf "$TMP_DIR"
}

# ── PATH ───────────────────────────────────────────────────────────────────
ensure_path() {
    case ":$PATH:" in
        *":${INSTALL_DIR}:"*) NEEDS_SOURCE=0 ;;
        *)
            NEEDS_SOURCE=1
            SHELL_NAME="$(basename "${SHELL:-sh}")"
            case "$SHELL_NAME" in
                bash) SHELL_RC="$HOME/.bashrc" ;;
                zsh)  SHELL_RC="$HOME/.zshrc" ;;
                fish) SHELL_RC="$HOME/.config/fish/config.fish" ;;
                *)    SHELL_RC="$HOME/.profile" ;;
            esac
            printf "\n# Added by CleanClaw installer\nexport PATH=\"\$PATH:%s\"\n" "$INSTALL_DIR" >> "$SHELL_RC"
            ;;
    esac
}

# ── Main ────────────────────────────────────────────────────────────────────
main() {
    printf "\n${BOLD}  ⚡ CleanClaw Installer${NC}\n"
    printf "  ─────────────────────\n\n"

    detect_platform
    info "Platform: ${PLATFORM}"

    choose_install_dir
    info "Install dir: ${INSTALL_DIR}"

    get_latest_version
    info "Version: ${VERSION}"

    install_binary
    ensure_path

    printf "\n"
    success "CleanClaw ${VERSION} installed → ${INSTALL_DIR}/${BINARY}"
    printf "\n"

    if [ "${NEEDS_SOURCE:-0}" = "1" ]; then
        printf "  ${YELLOW}Run this to activate:${NC}\n"
        printf "    source %s\n\n" "$SHELL_RC"
        printf "  Or open a new terminal, then run: ${BOLD}cleanclaw${NC}\n"
    else
        printf "  Run: ${BOLD}cleanclaw daemon install && cleanclaw daemon start${NC}\n"
        printf "  Or, for foreground: ${BOLD}cleanclaw gateway${NC}\n"
    fi
    printf "\n"
}

main "$@"
