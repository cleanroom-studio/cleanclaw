#!/usr/bin/env bash
# The Go version did `cd web && pnpm build && cd ..` + `cp -r web/out
# internal/setup/web`. The Rust port is a no-op: there's no
# separate web build step (the SSR frontend is a crate, built
# by `cargo build --workspace`).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> dev-build: cargo build --workspace (debug)"
cargo build --workspace --profile dev

echo "==> dev-build: cargo test --workspace"
cargo test --workspace --no-fail-fast

echo "==> dev-build: done"
echo "    binary:    target/debug/cleanclaw"
echo "    plugins:   target/debug/cleanclaw-plugin-{demo,mem0,post-turn-echo,openclaw-demo}"
