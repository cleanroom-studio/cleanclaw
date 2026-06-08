# cleanclaw-sandbox — runtime image for the agent's exec sandbox.
#
# Why this exists: the default `python:3.12-slim` image only has
# Python. Bundled skills (like camoufox-cli) drive a real browser,
# so we need Python + Node + a browser binary pre-installed so
# the agent can run `camoufox-cli open <url>` on the first turn
# with zero round-trips.
#
# Base: `node:22-bookworm-slim` — small Debian image with Node
# already wired up. We add Python on top.

FROM node:22-bookworm-slim

ENV DEBIAN_FRONTEND=noninteractive \
    PIP_NO_CACHE_DIR=1 \
    PIP_DISABLE_PIP_VERSION_CHECK=1

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        python3 \
        python3-pip \
        python3-venv \
        ca-certificates \
        curl \
        git \
        sudo \
        fonts-noto-cjk \
        fonts-noto-color-emoji \
 && ln -sf /usr/bin/python3 /usr/local/bin/python

RUN pip install --break-system-packages --no-cache-dir \
        camoufox-cli \
        uv \
        requests \
        pillow \
        beautifulsoup4 \
        lxml

RUN camoufox-cli install --with-deps

RUN rm -rf /var/lib/apt/lists/*

RUN mv /usr/local/bin/camoufox-cli /usr/local/bin/camoufox-cli-bin \
 && printf '%s\n' \
        '#!/bin/sh' \
        'proxy="${HTTPS_PROXY:-${https_proxy:-${HTTP_PROXY:-${http_proxy:-}}}}"' \
        'if [ -n "$proxy" ]; then' \
        '    for a in "$@"; do' \
        '        case "$a" in' \
        '            --proxy|--proxy=*) exec /usr/local/bin/camoufox-cli-bin "$@" ;;' \
        '        esac' \
        '    done' \
        '    exec /usr/local/bin/camoufox-cli-bin --proxy "$proxy" "$@"' \
        'fi' \
        'exec /usr/local/bin/camoufox-cli-bin "$@"' \
    > /usr/local/bin/camoufox-cli \
 && chmod +x /usr/local/bin/camoufox-cli

WORKDIR /workspace
