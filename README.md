# CleanClaw

A multi-tenant AI agent runtime with a SvelteKit web UI — built in Rust.

CleanClaw lets you deploy, manage, and chat with AI agents at scale. Agents can execute shell commands, read/write files, search the web, generate images, install skills from the open registry, and integrate with messaging channels (Telegram, Discord, Slack, etc.).

## Features

- **Multi-tenant** — users, agents, channels, projects, all with scoped permissions
- **Agent runtime** — tool-calling loop with dynamic skill loading, context compaction, and session management
- **Sandbox execution** — Docker, E2B, or local executor for safe code execution
- **Skill system** — discover and install skills from the open [skills.sh](https://skills.sh) registry, or author your own in SKILL.md format
- **Web UI** — SvelteKit dashboard for managing agents, sessions, and configuration
- **CLI** — full-featured command-line interface for admins and automation
- **Channels** — connect agents to Telegram, Discord, Slack, LINE, Feishu, WeChat, and custom webhooks
- **Storage** — SQLite (single-user) or PostgreSQL (multi-pod)
- **Sandboxed plugins** — JSON-RPC plugin protocol for extending the runtime
- **MCP support** — Model Context Protocol clients for stdio and HTTP transports

## Quick Start

### Single-machine (SQLite, no sandbox)

```bash
# Install
curl -fsSL https://raw.githubusercontent.com/cleanroom-studio/cleanclaw/main/install.sh | sh

# Or build from source
make build

# Run the gateway
cleanclaw gateway

# Open http://localhost:18953 and follow the onboard wizard
```

### Docker Compose (PostgreSQL)

```bash
cd deploy/docker
docker compose up -d
open http://localhost:18953
```

### Kubernetes

```bash
kubectl apply -f deploy/k8s/cleanclaw.yaml
```

## Project Layout

```
crates/              Rust workspace (39 crates)
├── cleanclaw-cli             CLI entry point
├── cleanclaw-agent           Agent runtime (tool loop, skills, memory)
├── cleanclaw-api             HTTP API handlers
├── cleanclaw-gateway         Gateway server
├── cleanclaw-store           Storage (SQLite / PostgreSQL)
├── cleanclaw-session         Session manager
├── cleanclaw-sandbox         Sandbox executors
├── cleanclaw-mcp             MCP client
├── cleanclaw-plugins/        Built-in plugin examples
├── cleanclaw-web             SSR web frontend
└── ...
web/                 SvelteKit frontend source
deploy/              Docker / K8s / Helm manifests
scripts/             Build and release scripts
```

## Build & Test

```bash
make build          # Debug build (workspace)
make release        # Release build with LTO
make test           # Full test suite
make lint           # Clippy check
make dev            # Dev mode (gateway on :18953)
```

### Makefile targets reference

| Target | Description |
|---|---|
| `make build` | Debug build of the entire workspace |
| `make release` | Optimized release build with LTO, stamped with version info |
| `make release-local` | `release` + `strip` for a smaller binary |
| `make install` | `cargo install` the CLI to `~/.cargo/bin` |
| `make dev` | Dev loop: starts gateway on `:18953` + SvelteKit HMR on `:5173` (Ctrl-C cleans up both) |
| `make fmt` | Auto-format all Rust code (`cargo fmt --all`) |
| `make lint` | Run clippy with `-D warnings` (strict, does not auto-fix) |
| `make lint-fix` | Auto-fix clippy suggestions (`cargo clippy --fix`); run `make fmt` afterwards |
| `make test` | Run the full Rust workspace test suite |
| `make test-scripts` | Run shell-based script tests (arch detection, Dockerfile lint) |
| `make test-e2e` | Run e2e tests against live LLM endpoints (requires `ANTHROPIC_API_KEY` + `OPENAI_API_KEY`) |
| `make ci` | Lint + test + script tests, equivalent to local CI |

| `make bundle-skills` | Copy bundled skills for rust-embed (run after editing any `SKILL.md`) |
| `make docker` | Build a single-arch Docker image for the host |
| `make docker-multi` | Build multi-arch (`linux/amd64` + `linux/arm64`) Docker images via buildx |
| `make clean` | `cargo clean` |

## Architecture

```
                    ┌──────────┐
                    │  nginx   │  (optional)
                    └────┬─────┘
                         │
              ┌──────────▼──────────┐
              │   cleanclaw-gateway │  Rust binary (SSR UI + API)
              └──────────┬──────────┘
                         │
         ┌───────────────┼───────────────┐
         │               │               │
   ┌─────▼─────┐  ┌──────▼──────┐  ┌────▼────┐
   │ Postgres  │  │   Sandbox   │  │  Hooks  │
   │  (store)  │  │  (Docker)   │  │ Server  │
   └───────────┘  └─────────────┘  └─────────┘
```

## License

[MIT](LICENSE)
