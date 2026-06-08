// `api.ts` — single source of truth for the Rust API shape. Mirrors
//  (the React dashboard's
// client) so porting pages from CleanClaw is largely a 1:1
// translation of imports.
//
// Auth uses session cookies set by /api/login; the Resolver on
// the backend reads `cleanclaw_session` (HttpOnly) and resolves
// it to a user. We do NOT use a Bearer token in localStorage —
// the cookie is the canonical credential, so refreshes keep the
// user signed in.

export interface StatusResponse {
  configured: boolean;
  running: boolean;
  port: number;
  version: string;
  uptime: string;
  user_count?: number;
  agent_count?: number;
  channel_count?: number;
  cron_count?: number;
}

export interface UserInfo {
  id: string;
  username: string;
  email?: string;
  display_name?: string;
  role: 'super_admin' | 'admin' | 'user';
  is_admin: boolean;
  agent_quota?: number;
  avatar_url?: string;
  apikey_id?: string;
}

export interface MeResponse {
  ok: boolean;
  user?: UserInfo;
}

export interface RegisterResponse {
  ok: boolean;
  user_id?: string;
  error?: string;
}

export interface LoginResponse {
  ok: boolean;
  error?: string;
  user_id?: string;
  username?: string;
  role?: string;
  session_id?: string;
}

export interface AgentInfo {
  id: string;
  name?: string;
  description?: string;
  avatar_url?: string;
  user_id?: string;
  role?: 'owner' | 'viewer';
  is_public?: boolean;
  share_model_config?: boolean;
  model: string;
  workspace?: string;
  max_tokens?: number;
  temperature?: number;
  max_tool_iterations?: number;
  thinking?: string;
  prompt_mode?: string;
  soul?: string;
  identity?: string;
  system_prompt?: string;
  context_files?: string[];
  created_at?: string;
  updated_at?: string;
}

export interface AgentDetail extends AgentInfo {
  config?: Record<string, any>;
}

export interface ChannelInfo {
  type: string;
  account_id: string;
  bot_username?: string;
  enabled: boolean;
  source?: string;
  updated_at?: string;
}

export interface ProviderInfo {
  id: string;
  name?: string;
  type?: string;
  model: string;
  base_url?: string;
  api_type?: string;
  api_base?: string;
  has_api_key?: boolean;
  auth_type?: string;
  models?: Array<{ id: string; name?: string }>;
  scope?: 'system' | 'agent' | 'user';
  is_inherited?: boolean;
}

export interface PluginInfo {
  id: string;
  name: string;
  enabled: boolean;
  version?: string;
  description?: string;
}

export interface SkillInfo {
  name: string;
  description: string;
  layer?: string;
  gated?: boolean;
  type?: string;
  api_key?: string;
  env?: Record<string, string>;
}

export interface SkillSearchResult {
  name: string;
  description: string;
  source?: string;
  url?: string;
}

export interface ToolConfig {
  enabled: boolean;
  provider: string;
}
export type ToolsConfig = Record<string, ToolConfig>;

export interface CronJobInfo {
  id: string;
  agent_id?: string;
  name: string;
  type: string;
  schedule: string;
  message: string;
  channel: string;
  chat_id: string;
  enabled: boolean;
  next_run?: string;
  created_at?: string;
}

export interface SessionInfo {
  key: string;
  channel: string;
  account_id: string;
  chat_id: string;
  project_id: string;
  title: string;
  message_count: number;
  updated_at: string;
  preview?: string;
  thumbnail_url?: string;
}

export interface ProjectInfo {
  id: string;
  agent_id: string;
  name: string;
  description: string;
  created_at: string;
  updated_at: string;
}

export interface UsageInfo {
  agent_id: string;
  input_tokens: number;
  output_tokens: number;
  total_tokens: number;
  period: string;
}

export interface ApiKeyInfo {
  id: string;
  type: string;
  key_prefix: string;
  name: string;
  user_id?: string;
  agents?: string[];
  created_at?: string;
}

export interface ChatTurnRequest {
  agent_id: string;
  message: string;
  session_key?: string;
  model?: string;
}

export interface ChatTurnResponse {
  reply: string;
  finish_reason: string;
  usage: { input_tokens: number; output_tokens: number; cache_read_tokens?: number; cache_creation_tokens?: number };
  iterations: number;
}

export interface ChatHistoryMessage {
  role: string;
  content: string;
  thinking?: string;
  tool_calls?: any;
  tool_call_id?: string;
  name?: string;
  created_at?: string;
}

// ---- Auth ----

export async function apiFetch<T = unknown>(path: string, init: RequestInit = {}): Promise<T> {
  const headers = new Headers(init.headers);
  if (!headers.has('Content-Type') && init.body) headers.set('Content-Type', 'application/json');
  const res = await fetch(path, { ...init, headers, credentials: 'same-origin' });
  if (!res.ok) {
    const text = await res.text();
    let err: { message: string; type?: string } = { message: text || res.statusText };
    try {
      const j = JSON.parse(text);
      if (j?.error?.message) err = j.error;
      else if (j?.error) err = { message: String(j.error) };
    } catch {
      // not JSON
    }
    throw Object.assign(new Error(err.message), { status: res.status, ...err });
  }
  const ct = res.headers.get('content-type') || '';
  if (ct.includes('application/json')) return (await res.json()) as T;
  return (await res.text()) as unknown as T;
}

export async function getStatus(): Promise<StatusResponse> {
  return apiFetch<StatusResponse>('/api/status');
}

export async function getHealth(): Promise<{ ok: boolean }> {
  return apiFetch<{ ok: boolean }>('/api/health');
}

export async function getMe(): Promise<MeResponse> {
  return apiFetch<MeResponse>('/api/me');
}

export async function register(req: { username: string; email: string; password: string; display_name?: string }): Promise<RegisterResponse> {
  return apiFetch<RegisterResponse>('/api/register', { method: 'POST', body: JSON.stringify(req) });
}

export async function login(req: { login?: string; username?: string; password: string }): Promise<LoginResponse> {
  // The Rust API's `cleanclaw-api` `/api/login` accepts either
  // `login` (setup-style) or `username` (api-style). The
  // `CleanClaw` dashboard uses `username` — keep that as the
  // primary field.
  return apiFetch<LoginResponse>('/api/login', { method: 'POST', body: JSON.stringify(req) });
}

export async function logout(): Promise<{ ok: boolean }> {
  return apiFetch<{ ok: boolean }>('/api/logout', { method: 'POST' });
}

export async function updateMe(patch: { display_name?: string; avatar_url?: string; email?: string }): Promise<{ ok: boolean }> {
  return apiFetch('/api/me', { method: 'PUT', body: JSON.stringify(patch) });
}

export async function changeMyPassword(oldPassword: string, newPassword: string): Promise<{ ok: boolean; error?: string }> {
  return apiFetch('/api/me/password', { method: 'POST', body: JSON.stringify({ old_password: oldPassword, password: newPassword }) });
}

// ---- Agents ----

export async function listAgents(): Promise<{ agents: AgentInfo[] }> {
  return apiFetch<{ agents: AgentInfo[] }>('/api/agents');
}

export async function createAgent(req: { name: string; model: string; soul?: string; identity?: string; description?: string; is_public?: boolean; share_model_config?: boolean }): Promise<{ agent: AgentInfo }> {
  return apiFetch<{ agent: AgentInfo }>('/api/agents', { method: 'POST', body: JSON.stringify(req) });
}

export async function getAgent(id: string): Promise<{ agent: AgentDetail }> {
  return apiFetch<{ agent: AgentDetail }>(`/api/agents/${id}`);
}

export async function updateAgent(id: string, patch: Partial<AgentDetail>): Promise<{ ok: boolean; agent: AgentInfo }> {
  return apiFetch(`/api/agents/${id}`, { method: 'PUT', body: JSON.stringify(patch) });
}

export async function deleteAgent(id: string): Promise<{ ok: boolean }> {
  return apiFetch(`/api/agents/${id}`, { method: 'DELETE' });
}

// ---- Agent files (workspace + system files) ----

export interface AgentFileEntry { filename: string; }

export async function listAgentFiles(agentId: string): Promise<{ files: AgentFileEntry[] }> {
  return apiFetch<{ files: AgentFileEntry[] }>(`/api/agents/${agentId}/files`);
}

export async function getAgentFile(agentId: string, filename: string): Promise<{ filename: string; content: string }> {
  return apiFetch(`/api/agents/${agentId}/files/${encodeURIComponent(filename)}`);
}

export async function putAgentFile(agentId: string, filename: string, content: string): Promise<{ ok: boolean }> {
  return apiFetch(`/api/agents/${agentId}/files/${encodeURIComponent(filename)}`, { method: 'PUT', body: JSON.stringify({ content }) });
}

export async function deleteAgentFile(agentId: string, filename: string): Promise<{ ok: boolean }> {
  return apiFetch(`/api/agents/${agentId}/files/${encodeURIComponent(filename)}`, { method: 'DELETE' });
}

// ---- Agent config + tools registered ----

export async function getAgentConfig(agentId: string): Promise<{ config: any }> {
  // W1 alias: cleanclaw-api keeps the agent's per-row config in
  // the agents row; expose it as a thin wrapper. Real parity
  // (with stored-config scopes) lands in a follow-up.
  const a = await getAgent(agentId);
  return { config: a.agent };
}

export async function listAgentRegisteredTools(agentId: string): Promise<{ tools: Array<{ name: string; description?: string; category?: string }> }> {
  // cleanclaw currently doesn't track per-agent tool registry
  // separately — fall back to the global /api/tools endpoint
  // and tag them as the global set so the dashboard can render
  // something. (A real per-agent registry is on the backlog.)
  return apiFetch<{ tools: any[] }>('/api/tools').then((r) => ({ tools: [] }));
}

// ---- Projects ----

export async function listProjects(agentId: string): Promise<{ projects: ProjectInfo[] }> {
  return apiFetch<{ projects: ProjectInfo[] }>(`/api/agents/${agentId}/projects`);
}

export async function createProject(agentId: string, req: { name: string; description?: string }): Promise<{ id: string; ok: boolean }> {
  return apiFetch(`/api/agents/${agentId}/projects`, { method: 'POST', body: JSON.stringify(req) });
}

export async function updateProject(agentId: string, projectId: string, patch: { name?: string; description?: string }): Promise<{ ok: boolean }> {
  return apiFetch(`/api/agents/${agentId}/projects/${projectId}`, { method: 'PATCH', body: JSON.stringify(patch) });
}

export async function deleteProject(agentId: string, projectId: string): Promise<{ ok: boolean }> {
  return apiFetch(`/api/agents/${agentId}/projects/${projectId}`, { method: 'DELETE' });
}

// ---- Chat (SSE) ----

export interface ChatStreamEvent {
  type: 'content_delta' | 'thinking_delta' | 'tool_call' | 'tool_call_delta' | 'tool_result' | 'done' | 'error';
  data: {
    delta?: string;
    id?: string;
    name?: string;
    arguments?: any;
    arguments_delta?: string;
    index?: number;
    result?: string;
    is_error?: boolean;
    finish_reason?: string;
    usage?: { input_tokens: number; output_tokens: number; cache_read_tokens?: number; cache_creation_tokens?: number };
    message?: string;
  };
}

/** POST /api/chat/stream and parse the SSE frames. */
export async function sendChatStream(
  agentId: string,
  sessionKey: string,
  message: string,
  onEvent: (e: ChatStreamEvent) => void,
  opts: { model?: string; signal?: AbortSignal } = {}
): Promise<void> {
  const res = await fetch('/api/chat/stream', {
    method: 'POST',
    credentials: 'same-origin',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      agent_id: agentId,
      message,
      session_key: sessionKey,
      model: opts.model,
    }),
    signal: opts.signal,
  });
  if (!res.ok || !res.body) {
    const text = await res.text().catch(() => res.statusText);
    onEvent({ type: 'error', data: { message: text } });
    return;
  }
  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';
  while (true) {
    const { value, done } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });
    // SSE frames end with a blank line (\n\n).
    let idx: number;
    while ((idx = buffer.indexOf('\n\n')) >= 0) {
      const frame = buffer.slice(0, idx);
      buffer = buffer.slice(idx + 2);
      // Parse `event: <name>\ndata: <json>`.
      const evMatch = frame.match(/^event:\s*(\S+)/m);
      const dataMatch = frame.match(/^data:\s*(\{.*\})/m);
      if (!evMatch || !dataMatch) continue;
      let payload: any = {};
      try { payload = JSON.parse(dataMatch[1]); } catch { /* ignore */ }
      onEvent({ type: evMatch[1] as ChatStreamEvent['type'], data: payload });
    }
  }
}

// ---- Chat history + sessions ----

export async function getChatHistory(agentId: string, sessionKey: string): Promise<{ messages: ChatHistoryMessage[] }> {
  const q = new URLSearchParams({ agent_id: agentId, session_key: sessionKey });
  return apiFetch<{ messages: ChatHistoryMessage[] }>(`/api/chat/history?${q}`);
}

export async function listChatSessions(agentId: string): Promise<{ sessions: SessionInfo[] }> {
  const q = new URLSearchParams({ agent_id: agentId });
  return apiFetch<{ sessions: SessionInfo[] }>(`/api/chat/sessions?${q}`);
}

export async function renameChatSession(agentId: string, key: string, title: string): Promise<{ ok: boolean }> {
  const q = new URLSearchParams({ agent_id: agentId });
  return apiFetch(`/api/chat/sessions/${encodeURIComponent(key)}?${q}`, { method: 'PUT', body: JSON.stringify({ title }) });
}

export async function deleteChatSession(agentId: string, key: string): Promise<{ ok: boolean }> {
  const q = new URLSearchParams({ agent_id: agentId });
  return apiFetch(`/api/chat/sessions/${encodeURIComponent(key)}?${q}`, { method: 'DELETE' });
}

export async function moveChatSessionToProject(agentId: string, key: string, projectId: string): Promise<{ ok: boolean }> {
  const q = new URLSearchParams({ agent_id: agentId });
  return apiFetch(`/api/chat/sessions/${encodeURIComponent(key)}/project?${q}`, { method: 'PATCH', body: JSON.stringify({ project_id: projectId }) });
}

// ---- Cron ----

export async function listCron(agentId?: string): Promise<{ jobs: CronJobInfo[] }> {
  if (agentId) return apiFetch<{ jobs: CronJobInfo[] }>(`/api/agents/${agentId}/cron`);
  return apiFetch<{ jobs: CronJobInfo[] }>('/api/cron');
}

export async function createCron(req: Omit<CronJobInfo, 'id'> & { agent_id: string }): Promise<{ job: CronJobInfo }> {
  return apiFetch(`/api/agents/${req.agent_id}/cron`, { method: 'POST', body: JSON.stringify(req) });
}

export async function deleteCron(agentId: string, jobId: string): Promise<{ ok: boolean }> {
  return apiFetch(`/api/cron/${jobId}`, { method: 'DELETE' });
}

export async function toggleCron(agentId: string, jobId: string, enabled: boolean): Promise<{ ok: boolean }> {
  return apiFetch(`/api/cron/${jobId}`, { method: 'PATCH', body: JSON.stringify({ enabled }) });
}

// ---- Channels (per-agent) ----

export async function listChannels(agentId: string): Promise<{ channels: ChannelInfo[] }> {
  return apiFetch<{ channels: ChannelInfo[] }>(`/api/agents/${agentId}/channels`);
}

export async function connectChannel(
  agentId: string,
  type: string,
  accountId: string,
  botToken: string,
  config?: Record<string, any>
): Promise<{ ok: boolean; error?: string }> {
  // Each platform has its own /api/agents/:id/channels/<type>
  // endpoint in cleanclaw-setup. Map a few common ones.
  const endpoint = (() => {
    switch (type) {
      case 'telegram': return `/api/agents/${agentId}/channels/telegram`;
      case 'discord': return `/api/agents/${agentId}/channels/discord`;
      case 'slack': return `/api/agents/${agentId}/channels/slack`;
      case 'line': return `/api/agents/${agentId}/channels/line`;
      case 'feishu': return `/api/agents/${agentId}/channels/feishu`;
      default: return `/api/agents/${agentId}/channels`;
    }
  })();
  const body: any = { type, account_id: accountId, bot_token: botToken, config };
  return apiFetch(endpoint, { method: 'POST', body: JSON.stringify(body) });
}

export async function disconnectChannel(agentId: string, type: string, accountId: string): Promise<{ ok: boolean }> {
  return apiFetch(`/api/agents/${agentId}/channels/${type}/${accountId}`, { method: 'DELETE' });
}

export async function listGlobalChannels(): Promise<{ channels: Array<{ type: string; label: string; configured: boolean; logo: string }> }> {
  return apiFetch<{ channels: any[] }>('/api/channels');
}

export async function startWechatLogin(agentId: string, accountId: string): Promise<{ qrcode_url?: string; ticket?: string }> {
  return apiFetch(`/api/agents/${agentId}/channels/wechat/login`, { method: 'POST', body: JSON.stringify({ account_id: accountId }) });
}

export async function pollWechatLoginStatus(agentId: string, ticket: string): Promise<{ status: 'pending' | 'scanned' | 'connected' | 'expired' }> {
  return apiFetch(`/api/agents/${agentId}/channels/wechat/login/status?ticket=${encodeURIComponent(ticket)}`);
}

// ---- Providers ----

export async function listProviders(): Promise<{ providers: ProviderInfo[] }> {
  return apiFetch<{ providers: ProviderInfo[] }>('/api/providers');
}

export async function createProvider(req: { type: string; model: string; base_url?: string; api_base?: string; api_key: string; auth_type?: string; api_type?: string }): Promise<{ ok: boolean; provider?: ProviderInfo }> {
  return apiFetch('/api/providers', { method: 'POST', body: JSON.stringify(req) });
}

export async function updateProvider(id: string, patch: Partial<ProviderInfo> & { api_key?: string }): Promise<{ ok: boolean }> {
  return apiFetch(`/api/providers/${id}`, { method: 'PUT', body: JSON.stringify(patch) });
}

export async function deleteProvider(id: string): Promise<{ ok: boolean }> {
  return apiFetch(`/api/providers/${id}`, { method: 'DELETE' });
}

export async function testStoredProvider(id: string): Promise<{ ok: boolean; message?: string }> {
  return apiFetch(`/api/providers/${id}/test`, { method: 'POST' });
}

export async function testProvider(req: { type: string; api_base: string; api_key: string; model: string }): Promise<{ ok: boolean; message?: string }> {
  return apiFetch('/api/test-provider', { method: 'POST', body: JSON.stringify(req) });
}

// ---- Skills ----

export async function listSkills(agentId?: string): Promise<{ skills: SkillInfo[] }> {
  if (agentId) return apiFetch<{ skills: SkillInfo[] }>(`/api/agents/${agentId}/skills`);
  return apiFetch<{ skills: SkillInfo[] }>('/api/skills');
}

export async function searchSkills(q: string, source = 'skillssh'): Promise<{ results: SkillSearchResult[] }> {
  const url = `/api/skills/search?q=${encodeURIComponent(q)}&source=${encodeURIComponent(source)}`;
  return apiFetch<{ results: SkillSearchResult[] }>(url);
}

export async function installSkill(req: { source: string; spec: string; name?: string }): Promise<{ ok: boolean; name?: string; error?: string }> {
  return apiFetch('/api/skills/install', { method: 'POST', body: JSON.stringify(req) });
}

export async function uploadSkill(file: File, agentId?: string): Promise<{ ok: boolean; name?: string; error?: string }> {
  const url = agentId ? `/api/skills/upload?agent=${encodeURIComponent(agentId)}` : '/api/skills/upload';
  const fd = new FormData();
  fd.append('file', file);
  const res = await fetch(url, { method: 'POST', body: fd, credentials: 'same-origin' });
  if (!res.ok) {
    const t = await res.text();
    throw new Error(t || res.statusText);
  }
  return res.json();
}

export async function deleteSkill(name: string, agentId?: string): Promise<{ ok: boolean }> {
  if (agentId) return apiFetch(`/api/agents/${agentId}/skills/${encodeURIComponent(name)}`, { method: 'DELETE' });
  return apiFetch(`/api/skills/${encodeURIComponent(name)}`, { method: 'DELETE' });
}

// ---- Plugins ----

export async function listPlugins(): Promise<{ plugins: PluginInfo[] }> {
  return apiFetch<{ plugins: PluginInfo[] }>('/api/plugins');
}

export async function togglePlugin(id: string, enabled: boolean): Promise<{ ok: boolean }> {
  return apiFetch(`/api/plugins/${id}`, { method: 'PUT', body: JSON.stringify({ enabled }) });
}

// ---- Tools (global config) ----

export async function listTools(): Promise<{ tools: ToolsConfig }> {
  return apiFetch<{ tools: ToolsConfig }>('/api/tools');
}

export async function saveTools(tools: ToolsConfig): Promise<{ ok: boolean }> {
  return apiFetch('/api/tools', { method: 'PUT', body: JSON.stringify({ tools }) });
}

// ---- Models ----

export async function listModels(): Promise<{ models: Array<{ id: string; provider: string; label: string }> }> {
  return apiFetch<{ models: Array<{ id: string; provider: string; label: string }> }>('/api/models');
}

// ---- API keys ----

export async function listApikeys(): Promise<{ keys: ApiKeyInfo[] }> {
  return apiFetch<{ keys: ApiKeyInfo[] }>('/api/apikeys');
}

export async function createApikey(req: { name: string; type?: 'admin' | 'user' | 'agent' }): Promise<{ ok: boolean; key?: ApiKeyInfo; secret?: string; token?: string }> {
  return apiFetch('/api/apikeys', { method: 'POST', body: JSON.stringify(req) });
}

export async function deleteApikey(id: string): Promise<{ ok: boolean }> {
  return apiFetch(`/api/apikeys/${id}`, { method: 'DELETE' });
}

export async function rotateApikey(id: string): Promise<{ ok: boolean; secret?: string }> {
  return apiFetch(`/api/apikeys/${id}/rotate`, { method: 'POST' });
}

export async function setApikeyAgents(id: string, agents: string[]): Promise<{ ok: boolean }> {
  return apiFetch(`/api/apikeys/${id}/agents`, { method: 'PUT', body: JSON.stringify({ agents }) });
}

// ---- Admin ----

export async function adminListUsers(): Promise<{ users: UserInfo[] }> {
  return apiFetch<{ users: UserInfo[] }>('/api/admin/users');
}

export async function adminUpdateUserRole(userId: string, role: 'user' | 'admin' | 'super_admin'): Promise<{ ok: boolean }> {
  return apiFetch(`/api/admin/users/${userId}/role`, { method: 'POST', body: JSON.stringify({ role }) });
}

export async function adminDeleteUser(userId: string): Promise<{ ok: boolean }> {
  return apiFetch(`/api/admin/users/${userId}`, { method: 'DELETE' });
}

export async function adminListChats(): Promise<{ sessions: SessionInfo[] }> {
  return apiFetch<{ sessions: SessionInfo[] }>('/api/admin/chats');
}

export async function adminListUsage(): Promise<{ usage: UsageInfo[] }> {
  return apiFetch<{ usage: UsageInfo[] }>('/api/admin/usage');
}

export async function agentUsage(agentId: string): Promise<{ usage: UsageInfo[] }> {
  return apiFetch<{ usage: UsageInfo[] }>(`/api/agents/${agentId}/usage`);
}

export async function adminGetRegistration(): Promise<{ open: boolean }> {
  return apiFetch<{ open: boolean }>('/api/admin/registration');
}

export async function adminSetRegistration(open: boolean): Promise<{ ok: boolean; open: boolean }> {
  return apiFetch('/api/admin/registration', { method: 'PUT', body: JSON.stringify({ open }) });
}

// ---- Onboard (bootstrap) ----

export async function onboard(req: { admin: { username: string; email: string; password: string; display_name?: string }; provider?: any; agent?: any }): Promise<{ ok: boolean; error?: string }> {
  return apiFetch('/api/onboard', { method: 'POST', body: JSON.stringify(req) });
}

// Re-export under the cleaner name the dashboard uses.
export { adminGetRegistration as getRegistrationOpen };

// ---- Channel config (platform defaults) ----

export async function getChannelConfig(): Promise<{ config: Record<string, any> }> {
  return apiFetch<{ config: any }>('/api/channels-config');
}

export async function saveChannelConfig(config: Record<string, any>): Promise<{ ok: boolean }> {
  return apiFetch('/api/channels-config', { method: 'PUT', body: JSON.stringify(config) });
}

// ---- Scoped config (system / user / agent) ----

export async function readConfig(scope: 'system' | 'user' | 'agent', key: string, agentId?: string): Promise<{ value: any }> {
  const body: any = { scope, key };
  if (agentId) body.agent_id = agentId;
  return apiFetch('/api/config', { method: 'POST', body: JSON.stringify(body) });
}

export async function writeConfig(scope: 'system' | 'user' | 'agent', key: string, value: any, agentId?: string): Promise<{ ok: boolean }> {
  const body: any = { scope, key, value };
  if (agentId) body.agent_id = agentId;
  return apiFetch('/api/config', { method: 'POST', body: JSON.stringify(body) });
}

// ---- Tasks (admin) ----

export async function listTasks(): Promise<{ tasks: any[] }> {
  return apiFetch<{ tasks: any[] }>('/api/tasks');
}

// ---- Global usage ----

export async function getGlobalUsage(): Promise<{ usage: UsageInfo[] }> {
  return apiFetch<{ usage: UsageInfo[] }>('/api/usage');
}

// ---- Scoped channels (multi-tenant) ----

export async function listScopedChannels(): Promise<{ channels: any[] }> {
  return apiFetch<{ channels: any[] }>('/api/scoped-channels');
}

export async function createScopedChannel(req: any): Promise<{ ok: boolean; id?: string }> {
  return apiFetch('/api/scoped-channels', { method: 'POST', body: JSON.stringify(req) });
}

export async function updateScopedChannel(id: string, patch: any): Promise<{ ok: boolean }> {
  return apiFetch(`/api/scoped-channels/${id}`, { method: 'PUT', body: JSON.stringify(patch) });
}

export async function deleteScopedChannel(id: string): Promise<{ ok: boolean }> {
  return apiFetch(`/api/scoped-channels/${id}`, { method: 'DELETE' });
}

// ---- v1 (OpenAI-compat) ----

export async function v1ListAgents(): Promise<{ data: AgentInfo[] }> {
  return apiFetch<{ data: AgentInfo[] }>('/v1/agents');
}
