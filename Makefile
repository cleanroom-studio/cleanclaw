# CleanClaw build pipeline.
#
# Targets (parity with the Go Makefile):
#   build          - debug build of all binaries
#   build-web      - (no-op in Rust: the SSR frontend IS the binary)
#   bundle-skills  - copy skills/* into a target dir for embed
#   release        - release build with LTO
#   install        - cargo install into ~/.cargo/bin
#   test           - cargo test --workspace
#   dev            - cargo run with hot-reload watcher
#   clean          - cargo clean
#   fmt            - cargo fmt
#   lint           - cargo clippy
#   release-local  - stamped release for the local git describe

# Stamp the build identity (mirrors Go's -ldflags).
VERSION ?= $(shell git describe --tags --always --dirty 2>/dev/null || echo dev)
COMMIT  ?= $(shell git rev-parse --short HEAD 2>/dev/null || echo unknown)
DATE    ?= $(shell date -u +"%Y-%m-%dT%H:%M:%SZ")

# Default binary name.
BIN_NAME ?= cleanclaw
CARGO    ?= cargo
TARGET   ?= target/release/$(BIN_NAME)

.PHONY: all build build-debug build-web bundle-skills clean ci dev docker docker-multi fmt install lint lint-fix release release-local test test-scripts

all: build

# `build` is a debug build of the workspace.
build:
	$(CARGO) build --workspace

build-debug:
	$(CARGO) build --workspace --profile dev

# `build-web` is a no-op in Rust: the SSR frontend lives in the
# `cleanclaw-web` crate, not a separate build artifact. Kept for
# parity with the Go Makefile.
build-web:
	@echo "build-web: no-op (cleanclaw-web is a crate, built by 'make build')"

# `bundle-skills` copies the bundled skills into
# `crates/cleanclaw-skills-bundled/skills/` so the rust-embed build
# picks them up. Run after editing any SKILL.md.
bundle-skills:
	mkdir -p crates/cleanclaw-skills-bundled/skills
	rm -rf crates/cleanclaw-skills-bundled/skills/*
	cp -r crates/cleanclaw-skills-bundled/raw/* crates/cleanclaw-skills-bundled/skills/ 2>/dev/null || true
	@echo "bundled $$(ls crates/cleanclaw-skills-bundled/skills/ | wc -l) skills"

# `release` runs the optimized build.
# `make release-local`.
release:
	CLEANCLAW_VERSION=$(VERSION) CLEANCLAW_COMMIT=$(COMMIT) CLEANCLAW_DATE=$(DATE) \
		$(CARGO) build --release --workspace

release-local: release
	strip $(TARGET)

# `install` drops the release binary on PATH via `cargo install`,
# so it picks up `~/.cargo/bin/cleanclaw` automatically.
install: release
	$(CARGO) install --path crates/cleanclaw-cli --locked

# `test` runs the full workspace suite. The Go Makefile ran
# `go test ./...` — the Rust equivalent.
test:
	$(CARGO) test --workspace

# `test-scripts` runs the bash-based script tests in
# `scripts/tests/` (arch detection + Dockerfile lint). Cheap
# to run; catches regressions in the build pipeline that the
# Rust test suite doesn't touch.
test-scripts:
	@bash scripts/tests/run_all.sh

# `test-e2e` runs the real-provider e2e tests against a live
# LLM endpoint. Without the env vars (ANTHROPIC_API_KEY +
# OPENAI_API_KEY) the suite is a no-op (each test early-returns).
#
# Usage:
#   set -a; source /path/to/.env; set +a
#   make test-e2e
#
# Or with explicit model overrides:
#   E2E_ANTHROPIC_MODEL=claude-3-5-haiku-latest \
#   E2E_OPENAI_MODEL=gpt-4o-mini \
#       make test-e2e
test-e2e:
	@echo "==> e2e: requiring ANTHROPIC_API_KEY + OPENAI_API_KEY in env"
	@test -n "$$ANTHROPIC_API_KEY" || { echo "ANTHROPIC_API_KEY not set; e2e tests will skip" >&2; }
	@test -n "$$OPENAI_API_KEY"    || { echo "OPENAI_API_KEY not set; e2e tests will skip" >&2; }
	$(CARGO) test -p cleanclaw-e2e --test provider --all-features

# `ci` is the local equivalent of CI's lint + test + test-scripts
# pipeline. Use this before pushing.
ci: lint test test-scripts
	@echo "ci: all green"

# `dev` is the dev loop. Starts the Rust gateway on 18953 and the
# SvelteKit dev server (via bun) on 5173 — HMR works for everything
# under web/. Open http://localhost:5173 in your browser; the
# dev server proxies API calls to the gateway.
#
# Ctrl-C kills both processes (INT trap cleans up the background
# gateway process so it doesn't leak).
dev:
	cd web && bun install --frozen-lockfile
	@bash -c ' \
		$(CARGO) run -p cleanclaw-cli -- gateway --port 18953 & \
		PID=$$!; \
		trap "kill $$PID 2>/dev/null; exit 0" INT TERM EXIT; \
		cd web && bun run dev -- --port 5173 --host; \
		kill $$PID 2>/dev/null \
	'

# `lint` and `fmt` are the Rust equivalents of the Go pipeline's
# `go vet` / `gofmt`.
fmt:
	$(CARGO) fmt --all

lint:
	$(CARGO) clippy --workspace --all-targets -- -D warnings

# `lint-fix` auto-fixes clippy suggestions (e.g. redundant variables,
# unnecessary references, etc.). Run `make fmt` afterwards to
# re-normalize formatting.
lint-fix:
	$(CARGO) clippy --fix --workspace --all-targets --allow-dirty --allow-staged

# `docker` builds the image for the host arch (single arch,
# loadable into the local daemon). The `TAG` env var picks
# the image tag (default: git describe).
docker:
	@TAG="$(TAG)" scripts/build-docker.sh

# `docker-multi` builds for both `linux/amd64` and `linux/arm64`
# via buildx + QEMU. Without `--push` this is mostly a no-op
# (the manifest can't be loaded into a single-arch daemon) —
# CI sets `--push` via `scripts/release.sh --docker --multi-arch --push`.
docker-multi:
	@TAG="$(TAG)" scripts/build-docker.sh --multi-arch

clean:
	$(CARGO) clean
