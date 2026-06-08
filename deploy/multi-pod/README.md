# Multi-pod CleanClaw

Two-pod CleanClaw deployment behind nginx + Postgres.

## Quick start

```sh
cd deploy/multi-pod
docker compose up -d
open http://localhost:18953
```

## Architecture

```
   ┌─────────┐
   │  nginx  │  :80
   └────┬────┘
        │  least_conn
   ┌────┴───────────┐
   │                │
┌──▼──┐         ┌──▼──┐
│gw-1 │         │gw-2 │  (cleanclaw:dev)
└──┬──┘         └──┬──┘
   │               │
   └────┬──────────┘
        │
   ┌────▼─────┐
   │postgres  │  (cleanclaw/cleanclaw)
   └──────────┘
```

## Why two pods

Validates the multi-replica / horizontal-scale code path. The
Rust gateway is stateless beyond the Postgres connection; SSE/WS
sessions are pinned to a single pod via the load balancer's
`least_conn` strategy.

## Operations

```sh
# Scale up
docker compose up -d --scale cleanclaw-1=0 --scale cleanclaw-2=3

# Tail logs
docker compose logs -f nginx cleanclaw-1 cleanclaw-2

# Reset state
docker compose down -v
```

## Differences from

* The Go pipeline shipped a separate nginx.conf and a slightly
  different compose file. The Rust pipeline keeps the same shape
  with `proxy_http_version 1.1` + `proxy_buffering off` for
  SSE/WS compatibility.
