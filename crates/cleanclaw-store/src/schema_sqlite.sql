-- CleanClaw initial schema (SQLite).

CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL UNIQUE,
    email TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL DEFAULT '',
    display_name TEXT NOT NULL DEFAULT '',
    role TEXT NOT NULL DEFAULT 'user',
    status TEXT NOT NULL DEFAULT 'active',
    apikey_id TEXT NOT NULL DEFAULT '',
    external_id TEXT NOT NULL DEFAULT '',
    avatar_url TEXT NOT NULL DEFAULT '',
    agent_quota INTEGER NOT NULL DEFAULT -1,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_users_apikey_external ON users (apikey_id, external_id) WHERE apikey_id <> '' AND external_id <> '';

CREATE TABLE IF NOT EXISTS web_sessions (
    sid TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TIMESTAMP NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_web_sessions_user ON web_sessions (user_id);
CREATE INDEX IF NOT EXISTS idx_web_sessions_expires ON web_sessions (expires_at);

CREATE TABLE IF NOT EXISTS apikeys (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    name TEXT NOT NULL DEFAULT '',
    key_hash TEXT NOT NULL,
    key_prefix TEXT NOT NULL DEFAULT '',
    type TEXT NOT NULL DEFAULT 'agent',
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    prev_hash TEXT,
    prev_hash_set_at TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_apikeys_user ON apikeys (user_id);
CREATE INDEX IF NOT EXISTS idx_apikeys_key_hash ON apikeys (key_hash);

CREATE TABLE IF NOT EXISTS apikey_agents (
    apikey_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    PRIMARY KEY (apikey_id, agent_id)
);
CREATE INDEX IF NOT EXISTS idx_apikey_agents_agent ON apikey_agents (agent_id);

CREATE TABLE IF NOT EXISTS agents (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    name TEXT NOT NULL DEFAULT '',
    config TEXT NOT NULL DEFAULT '{}',
    is_public INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_agents_user ON agents (user_id);

CREATE TABLE IF NOT EXISTS sessions (
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    session_key TEXT NOT NULL,
    channel TEXT NOT NULL DEFAULT '',
    account_id TEXT NOT NULL DEFAULT '',
    chat_id TEXT NOT NULL DEFAULT '',
    project_id TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    messages TEXT NOT NULL DEFAULT '[]',
    message_count INTEGER NOT NULL DEFAULT 0,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    chatter_user_id TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (user_id, agent_id, session_key)
);
CREATE INDEX IF NOT EXISTS idx_sessions_chat_active ON sessions (user_id, agent_id, channel, account_id, chat_id, updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_sessions_by_chatter ON sessions (chatter_user_id, agent_id, updated_at DESC) WHERE chatter_user_id <> '';

CREATE TABLE IF NOT EXISTS session_messages (
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    session_key TEXT NOT NULL,
    seq INTEGER NOT NULL,
    role TEXT NOT NULL,
    content TEXT NOT NULL DEFAULT '',
    content_parts TEXT NOT NULL DEFAULT '',
    tool_calls TEXT NOT NULL DEFAULT '',
    tool_call_id TEXT NOT NULL DEFAULT '',
    name TEXT NOT NULL DEFAULT '',
    metadata TEXT NOT NULL DEFAULT '',
    thinking TEXT NOT NULL DEFAULT '',
    raw_assistant TEXT NOT NULL DEFAULT '',
    origin TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    chatter_user_id TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (user_id, agent_id, session_key, seq)
);
CREATE INDEX IF NOT EXISTS idx_session_messages_lookup ON session_messages (user_id, agent_id, session_key, seq);

CREATE TABLE IF NOT EXISTS session_events (
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    session_key TEXT NOT NULL,
    seq INTEGER NOT NULL,
    type TEXT NOT NULL,
    data TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    chatter_user_id TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (user_id, agent_id, session_key, seq)
);
CREATE INDEX IF NOT EXISTS idx_session_events_lookup ON session_events (user_id, agent_id, session_key, seq);

CREATE TABLE IF NOT EXISTS agent_files (
    agent_id TEXT NOT NULL,
    user_id TEXT NOT NULL DEFAULT '',
    filename TEXT NOT NULL,
    content TEXT NOT NULL DEFAULT '',
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (agent_id, user_id, filename)
);

CREATE TABLE IF NOT EXISTS configs (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    scope TEXT NOT NULL DEFAULT '',
    user_id TEXT NOT NULL DEFAULT '',
    agent_id TEXT NOT NULL DEFAULT '',
    name TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1,
    credential_key TEXT NOT NULL DEFAULT '',
    data TEXT NOT NULL DEFAULT '{}',
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE (kind, user_id, agent_id, name)
);
CREATE INDEX IF NOT EXISTS idx_configs_lookup ON configs (kind, user_id, agent_id);
CREATE INDEX IF NOT EXISTS idx_configs_credential ON configs (kind, credential_key);

CREATE TABLE IF NOT EXISTS cron_jobs (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL DEFAULT '',
    agent_id TEXT NOT NULL,
    name TEXT NOT NULL DEFAULT '',
    type TEXT NOT NULL DEFAULT 'cron',
    schedule TEXT NOT NULL,
    message TEXT NOT NULL,
    channel TEXT NOT NULL,
    chat_id TEXT NOT NULL,
    account_id TEXT NOT NULL DEFAULT '',
    timezone TEXT NOT NULL DEFAULT 'UTC',
    enabled INTEGER NOT NULL DEFAULT 1,
    last_run TIMESTAMP,
    next_run TIMESTAMP,
    locked_by TEXT,
    locked_at TIMESTAMP,
    failure_count INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_cron_jobs_user ON cron_jobs (user_id, agent_id);
CREATE INDEX IF NOT EXISTS idx_cron_jobs_schedule ON cron_jobs (enabled, next_run);
CREATE INDEX IF NOT EXISTS idx_cron_jobs_agent ON cron_jobs (agent_id);

CREATE TABLE IF NOT EXISTS projects (
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    name TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (user_id, agent_id, project_id)
);
CREATE INDEX IF NOT EXISTS idx_projects_listing ON projects (user_id, agent_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS agent_goals (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    session_key TEXT NOT NULL,
    owner_user_id TEXT NOT NULL,
    channel TEXT NOT NULL DEFAULT '',
    account_id TEXT NOT NULL DEFAULT '',
    chat_id TEXT NOT NULL DEFAULT '',
    project_id TEXT NOT NULL DEFAULT '',
    objective TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    token_budget BIGINT,
    tokens_used BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_agent_goals_session ON agent_goals (agent_id, session_key);

CREATE TABLE IF NOT EXISTS token_usage_daily (
    day DATE NOT NULL,
    user_id TEXT NOT NULL DEFAULT '',
    agent_id TEXT NOT NULL DEFAULT '',
    session_key TEXT NOT NULL DEFAULT '',
    provider TEXT NOT NULL DEFAULT '',
    model TEXT NOT NULL DEFAULT '',
    input_tokens BIGINT NOT NULL DEFAULT 0,
    output_tokens BIGINT NOT NULL DEFAULT 0,
    cache_read_tokens BIGINT NOT NULL DEFAULT 0,
    cache_create_tokens BIGINT NOT NULL DEFAULT 0,
    request_count BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (day, user_id, agent_id, session_key, provider, model)
);
CREATE INDEX IF NOT EXISTS idx_token_usage_agent ON token_usage_daily (agent_id, day);
CREATE INDEX IF NOT EXISTS idx_token_usage_user ON token_usage_daily (user_id, day);

CREATE TABLE IF NOT EXISTS channel_leases (
    channel TEXT NOT NULL,
    account_id TEXT NOT NULL,
    holder_id TEXT NOT NULL,
    expires_at TIMESTAMP NOT NULL,
    PRIMARY KEY (channel, account_id)
);

-- Per-user channel credential/config row. Holds the bot token,
-- webhook secret, etc., for one (channel, account_id) tuple. The
-- config data column carries a JSON blob whose shape depends on
-- the channel (Telegram bot token, Discord bot token + guild ids,
-- LINE channel secret + access token, ...). Mirrors the Go
-- `scoped_configs` table.
CREATE TABLE IF NOT EXISTS channel_configs (
    user_id TEXT NOT NULL,
    channel TEXT NOT NULL,
    account_id TEXT NOT NULL,
    name TEXT NOT NULL DEFAULT '',
    enabled INTEGER NOT NULL DEFAULT 1,
    credential_key TEXT NOT NULL DEFAULT '',
    data TEXT NOT NULL DEFAULT '{}',
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (user_id, channel, account_id)
);
CREATE INDEX IF NOT EXISTS idx_channel_configs_lookup ON channel_configs (user_id, channel);

-- Per-user LLM provider credential. The credential_key is an
-- opaque, never-decrypted-by-CLI identifier that the gateway uses
-- to look up the AES-encrypted blob in `provider_credentials`. Mirrors
-- the `provider_credentials` table.
CREATE TABLE IF NOT EXISTS provider_credentials (
    user_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    name TEXT NOT NULL DEFAULT '',
    api_base TEXT NOT NULL DEFAULT '',
    model TEXT NOT NULL DEFAULT '',
    credential_key TEXT NOT NULL DEFAULT '',
    enabled INTEGER NOT NULL DEFAULT 1,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (user_id, provider, name)
);
CREATE INDEX IF NOT EXISTS idx_provider_credentials_lookup ON provider_credentials (user_id, provider);

-- Per-provider health probe log. The gateway writes one row per
-- successful probe; stale rows are dropped on a rolling window.
-- Mirrors the Go `provider_health` table.
CREATE TABLE IF NOT EXISTS provider_health (
    provider TEXT NOT NULL,
    ok INTEGER NOT NULL,
    latency_ms INTEGER NOT NULL DEFAULT 0,
    message TEXT NOT NULL DEFAULT '',
    probed_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (provider, probed_at)
);
CREATE INDEX IF NOT EXISTS idx_provider_health_recent ON provider_health (provider, probed_at DESC);

-- Agent file metadata. The `agent_files` table holds the full body
-- for small files; for files larger than 1 MB the body is sharded
-- across `agent_file_blobs` keyed by (agent_id, user_id, filename,
-- offset). Mirrors the Go sharding scheme.
CREATE TABLE IF NOT EXISTS agent_file_blobs (
    agent_id TEXT NOT NULL,
    user_id TEXT NOT NULL DEFAULT '',
    filename TEXT NOT NULL,
    offset INTEGER NOT NULL,
    content BLOB NOT NULL,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (agent_id, user_id, filename, offset)
);

-- Goal continuation prompts. Persists the rendered continuation /
-- budget-limit prompt per agent so the loop can resume after a
-- restart without losing its place in the cascade.
CREATE TABLE IF NOT EXISTS goal_prompts (
    agent_id TEXT NOT NULL,
    session_key TEXT NOT NULL,
    kind TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (agent_id, session_key, kind)
);
