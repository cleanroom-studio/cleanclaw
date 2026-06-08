#!/usr/bin/env bash
# Multi-arch Docker build helper. The actual release script
# (`scripts/release.sh --docker --multi-arch --push`) does the
# same thing; this script is the dev-friendly shortcut that
# defaults to `--load` (so `docker run` can use the resulting
# image on the same host) and is a no-op when buildx isn't
# installed.
#
# Usage:
#   scripts/build-docker.sh                          # host arch
#   scripts/build-docker.sh --multi-arch             # linux/amd64 + linux/arm64
#   scripts/build-docker.sh --multi-arch --push      # build + push
#   TAG=dev scripts/build-docker.sh                  # custom tag (default: git describe)
set -euo pipefail

TAG="${TAG:-$(git describe --tags --always --dirty 2>/dev/null || echo dev)}"
PLATFORMS="${PLATFORKS:-}"  # explicit override
PUSH=0
MULTI_ARCH=0

for arg in "$@"; do
    case "$arg" in
        --multi-arch) MULTI_ARCH=1 ;;
        --push)       PUSH=1 ;;
        --platform)   shift; PLATFORMS="${1:-}" ;;
        -h|--help)    sed -n '2,12p' "$0"; exit 0 ;;
        *)            echo "unknown arg: $arg" >&2; exit 1 ;;
    esac
done

if [ -z "$PLATFORMS" ]; then
    if [ "$MULTI_ARCH" -eq 1 ]; then
        PLATFORMS="linux/amd64,linux/arm64"
    else
        # Single-arch: use the build host's arch so the loaded
        # image is actually runnable on this host.
        HOST_ARCH="$(uname -m)"
        case "$HOST_ARCH" in
            x86_64|amd64)   PLATFORMS="linux/amd64" ;;
            aarch64|arm64)  PLATFORMS="linux/arm64" ;;
            *)              PLATFORMS="linux/amd64" ;;  # safe default
        esac
    fi
fi

IMAGE="cleanclaw/cleanclaw:${TAG}"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> build-docker"
echo "    tag:       $TAG"
echo "    image:     $IMAGE"
echo "    platforms: $PLATFORMS"
echo "    push:      $PUSH"

if [ "$PUSH" -eq 1 ]; then
    docker buildx build --push --platform "$PLATFORMS" --tag "$IMAGE" .
else
    # `--load` only works for single-platform builds. Multi-arch
    # without `--push` is mostly a no-op (you can't `docker run`
    # a multi-arch manifest on the same host); the buildx action
    # will warn and the test will be invalid. CI must use --push
    # for multi-arch; locally, omit --multi-arch or accept the
    # warning.
    if [ "$MULTI_ARCH" -eq 1 ]; then
        echo "warning: multi-arch + no --push means the image is not"
        echo "         loaded into the local docker. re-run with --push"
        echo "         to publish, or drop --multi-arch for local builds."
        docker buildx build --platform "$PLATFORMS" --tag "$IMAGE" .
    else
        docker buildx build --load --platform "$PLATFORMS" --tag "$IMAGE" .
    fi
fi
