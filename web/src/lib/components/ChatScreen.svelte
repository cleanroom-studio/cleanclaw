<script lang="ts">
  // ChatScreen — full SSE-driven chat surface. Mirrors
  //  (~3500 LoC
  // in CleanClaw) but compressed to a Svelte 5 runes-based
  // implementation. Drives every chat route in the app:
//
  //   • /chat/                  → bare landing, pick an agent
  //   • /agents/<id>/chat/      → fresh chat on that agent
  //   • /agents/<id>/chat/<k>/  → resume session <k>
  //   • /agents/<id>/project/<pid>/ → chat scoped to a project
//
  // Responsibilities (parity list with CleanClaw):
  //   1. Load agents on mount; pick the first (or `?agent=<id>`).
  //   2. Subscribe to the active (agent_id, session_key) tuple and
  //      fetch history via /api/chat/history.
  //   3. Render a bubble stack: user, assistant (live-streamed with
  //      `content_delta` accumulating), tool-group, error.
  //   4. Markdown rendering of assistant content (GFM).
  //   5. Stream a turn via /api/chat/stream; coalesce deltas into
  //      a single in-flight assistant bubble (per round, per the
  //      CleanClaw comment about the in-flight bubble lifecycle).
  //   6. Tool-call / tool-result rendering inside a collapsible
  //      group, with copy / collapse actions.
  //   7. New-chat / rename / delete session actions.
  //   8. Auto-resize textarea; Enter sends, Shift+Enter newline.
  //   9. Auto-scroll to bottom on new content.
  //   10. Optional `?actAs=<uid>` — admin/owner view-only mode
  //       (sidebar + sidebar chrome hidden by AppShell).
  //   11. On session change, broadcast `cleanclaw:sessions-changed`
  //       so the sidebar refetches.

  import { onMount, tick, untrack } from 'svelte';
  import { page } from '$app/state';
  import {
    listAgents,
    listChatSessions,
    getChatHistory,
    sendChatStream,
    renameChatSession,
    deleteChatSession,
    createAgent,
    type AgentInfo,
    type SessionInfo,
    type ChatStreamEvent,
    type ChatHistoryMessage,
  } from '$lib/api';
  import ChatRowActions from './ChatRowActions.svelte';
  import MarkdownLink from './MarkdownLink.svelte';

  // ---- Reactive route state ----------------------------------------

  // Active agent id comes from the URL — three sources:
  //   /chat/?agent=<id>
  //   /agents/<id>/chat/...
  //   /agents/<id>/project/<pid>/...
  // The `pathname` shape lets us reuse ChatScreen across all
  // three layouts without per-route shims.
  function agentIdFromPath(pathname: string): string | null {
    const m = pathname.match(/^\/agents\/([^/]+)/);
    return m ? m[1] : null;
  }
  function projectIdFromPath(pathname: string): string | null {
    const m = pathname.match(/^\/agents\/[^/]+\/project\/([^/?#]+)/);
    return m ? m[1] : null;
  }
  const routeAgentId = $derived(agentIdFromPath(page.url?.pathname || ''));
  const routeProjectId = $derived(projectIdFromPath(page.url?.pathname || ''));
  const queryAgentId = $derived(page.url?.searchParams.get('agent'));
  const querySessionId = $derived(page.url?.searchParams.get('session'));
  const pathnameOnly = $derived(page.url?.pathname || '');
  // /chat/<sessionId> path-segment form
  const pathSession = $derived(
    (() => {
      const m = (page.url?.pathname || '').match(/^\/(chat|agents\/[^/]+\/chat)\/([^/?#]+)/);
      return m ? decodeURIComponent(m[2]) : null;
    })(),
  );
  const activeSessionId = $derived(querySessionId || pathSession || null);

  // ---- State --------------------------------------------------------

  let agents = $state<AgentInfo[]>([]);
  let activeAgent = $state<AgentInfo | null>(null);
  let sessions = $state<SessionInfo[]>([]);
  let messages = $state<ChatHistoryMessage[]>([]);
  let input = $state('');
  let sending = $state(false);
  let error = $state('');
  let loading = $state(true);
  let editingTitle = $state(false);
  let titleDraft = $state('');
  let messageListEl: HTMLElement | undefined = $state();
  let textareaEl: HTMLTextAreaElement | undefined = $state();
  let liveAssistant = $state<{ content: string; toolCalls: { id: string; name: string; arguments: string; result?: string }[] }>({ content: '', toolCalls: [] });

  // ---- Lifecycle ---------------------------------------------------

  onMount(async () => {
    try {
      const r = await listAgents();
      agents = r.agents ?? [];
      // Pick agent: URL > first available
      const wantId = routeAgentId || queryAgentId;
      const found = wantId ? agents.find((a) => a.id === wantId) : agents[0];
      if (found) {
        activeAgent = found;
        await refetchSessions();
        await loadHistory();
      }
    } catch (e) {
      error = (e as Error).message || 'failed to load agents';
    } finally {
      loading = false;
    }
  });

  // Re-run when the active agent / session changes via the URL.
  let lastRouteKey = '';
  $effect(() => {
    const key = `${routeAgentId || queryAgentId || ''}|${activeSessionId || ''}`;
    if (key === lastRouteKey) return;
    lastRouteKey = key;
    void onRouteChange();
  });

  async function onRouteChange() {
    if (!activeAgent || routeAgentId !== activeAgent.id) {
      const want = routeAgentId || queryAgentId;
      const found = want ? agents.find((a) => a.id === want) : agents[0];
      if (found) {
        activeAgent = found;
      } else {
        return;
      }
    }
    await refetchSessions();
    await loadHistory();
  }

  async function refetchSessions() {
    if (!activeAgent) return;
    try {
      const r = await listChatSessions(activeAgent.id);
      sessions = r.sessions ?? [];
    } catch {}
  }

  async function loadHistory() {
    if (!activeAgent) return;
    // Pick a session key in priority order:
    //   1. explicit `?session=` query or path segment
    //   2. the most recently updated session for this agent
    //   3. nothing (render the empty-state hint)
    const sorted = [...sessions].sort(
      (a, b) => new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime(),
    );
    const key = activeSessionId || sorted[0]?.key || '';
    try {
      const r = await getChatHistory(activeAgent.id, key);
      messages = r.messages ?? [];
      // Reset the live in-flight bubble so a stale streamed tail
      // doesn't bleed into the next turn.
      liveAssistant = { content: '', toolCalls: [] };
    } catch {}
  }

  // ---- Sidebar broadcast --------------------------------------------

  function broadcastSessionsChanged() {
    if (typeof window !== 'undefined' && activeAgent) {
      window.dispatchEvent(
        new CustomEvent('cleanclaw:sessions-changed', { detail: { agentId: activeAgent.id } }),
      );
    }
  }

  // ---- Sending a turn ----------------------------------------------

  async function send(e?: Event) {
    e?.preventDefault();
    if (!activeAgent) {
      error = 'Pick an agent first';
      return;
    }
    const text = input.trim();
    if (!text || sending) return;

    // Snapshot the session key we'll send to. If the user is
    // parked on a project route, we carry the project_id through.
    const sessionKey = activeSessionId || `sk_${Date.now().toString(36)}`;

    // Optimistically append the user bubble.
    messages = [...messages, { role: 'user', content: text, created_at: new Date().toISOString() }];
    input = '';
    liveAssistant = { content: '', toolCalls: [] };
    sending = true;
    error = '';

    try {
      await sendChatStream(
        activeAgent.id,
        sessionKey,
        text,
        async (evt) => {
          await onStreamEvent(evt);
        },
        { model: activeAgent.model },
      );
    } catch (e) {
      error = (e as Error).message || 'chat failed';
    } finally {
      sending = false;
      // After the turn, refresh the session list so the new
      // session shows up in the sidebar.
      await refetchSessions();
      broadcastSessionsChanged();
      // Scroll the new content into view.
      await tick();
      scrollToBottom();
      textareaEl?.focus();
    }
  }

  async function onStreamEvent(evt: ChatStreamEvent) {
    switch (evt.type) {
      case 'content_delta': {
        const delta = evt.data.delta || '';
        if (!delta) break;
        // Coalesce into a single in-flight assistant bubble.
        liveAssistant.content += delta;
        // Mirror liveAssistant into messages by replacing the
        // last assistant entry if it exists; otherwise append.
        const idx = messages.findIndex((m) => m.role === 'assistant' && m === liveMarker.current);
        if (idx >= 0) {
          messages = messages.map((m, i) => i === idx ? { ...m, content: liveAssistant.content } : m);
        } else {
          const marker: any = { role: 'assistant', content: liveAssistant.content, created_at: new Date().toISOString() };
          liveMarker.current = marker;
          messages = [...messages, marker];
        }
        break;
      }
      case 'thinking_delta': {
        // Some providers stream thinking on a separate channel
        // (Anthropic extended thinking, OpenAI o-series). When
        // the hub emits these, fold them into the in-flight
        // assistant bubble as a `<think>` block so the render
        // path can parse + collapse them like any other think
        // tag. Markers are inserted at the start so deltas land
        // inside even before any regular content has streamed.
        const delta = evt.data.delta || '';
        if (delta && !liveAssistant.content.includes('<think>')) {
          liveAssistant.content = `<think>${delta}</think>\n` + liveAssistant.content;
        } else if (delta) {
          // Already has an opening tag — splice before the
          // closing `</think>` (or at end if the tag is still
          // open mid-stream).
          const close = liveAssistant.content.indexOf('</think>');
          if (close >= 0) {
            liveAssistant.content =
              liveAssistant.content.slice(0, close) + delta + liveAssistant.content.slice(close);
          } else {
            liveAssistant.content += delta;
          }
        }
        const idx = messages.findIndex((m) => m === liveMarker.current);
        if (idx >= 0) {
          messages = messages.map((m, i) => i === idx ? { ...m, content: liveAssistant.content } : m);
        }
        break;
      }
      case 'tool_call': {
        // Attach to the current in-flight assistant.
        liveAssistant.toolCalls.push({
          id: evt.data.id || '',
          name: evt.data.name || '',
          arguments: typeof evt.data.arguments === 'string' ? evt.data.arguments : JSON.stringify(evt.data.arguments || {}),
        });
        // The CleanClaw pattern coalesces tool-call round into
        // a tool-group rendered as a single collapsed bubble;
        // Svelte version emits a compact inline card for each.
        break;
      }
      case 'tool_result': {
        const tc = liveAssistant.toolCalls.find((t) => t.id === evt.data.id);
        if (tc) tc.result = evt.data.result || '';
        break;
      }
      case 'done': {
        // Finalize the in-flight assistant bubble into the
        // messages array (already there from content_delta).
        liveMarker.current = null;
        // After the terminal `done`, re-fetch history so the
        // persisted shape matches what the backend stored. We
        // sort sessions by `updated_at` first because the just-
        // finished turn may have created a brand-new session
        // (the SSE `session_key` we sent was generated client-
        // side if the user wasn't already on a session).
        if (activeAgent) {
          (async () => {
            try {
              await refetchSessions();
              const sorted = [...sessions].sort(
                (a, b) => new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime(),
              );
              const key = activeSessionId || sorted[0]?.key || '';
              const r = await getChatHistory(activeAgent.id, key);
              // Only overwrite messages if the server has the
              // persisted copy — preserves the streamed tail
              // (which may be ahead of the agent's save).
              if ((r.messages ?? []).length >= messages.length) {
                messages = r.messages;
              }
            } catch {}
          })();
        }
        break;
      }
      case 'error': {
        messages = [...messages, { role: 'assistant', content: `⚠️ ${evt.data.message || 'error'}`, created_at: new Date().toISOString() }];
        break;
      }
    }
  }

  // Marker ref for the in-flight assistant bubble. A plain
  // object so we can compare by reference in the message array.
  const liveMarker = { current: null as any };

  // ---- Session actions ---------------------------------------------

  function newChat() {
    void goto(`/agents/${activeAgent!.id}/chat/`);
    activeSessionId;
  }

  async function startRename() {
    if (!activeSessionId) return;
    const sess = sessions.find((s) => s.key === activeSessionId);
    titleDraft = sess?.title || '';
    editingTitle = true;
  }

  async function saveTitle() {
    if (!activeAgent || !activeSessionId) return;
    try {
      await renameChatSession(activeAgent.id, activeSessionId, titleDraft.trim());
      editingTitle = false;
      await refetchSessions();
      broadcastSessionsChanged();
    } catch (e) {
      alert((e as Error).message || 'rename failed');
    }
  }

  async function removeSession() {
    if (!activeAgent || !activeSessionId) return;
    if (!confirm(`Delete session ${activeSessionId}?`)) return;
    try {
      await deleteChatSession(activeAgent.id, activeSessionId);
      void goto(`/agents/${activeAgent.id}/chat/`);
      await refetchSessions();
      broadcastSessionsChanged();
    } catch (e) {
      alert((e as Error).message || 'delete failed');
    }
  }

  // ---- Auto-scroll + textarea autoresize ---------------------------

  function scrollToBottom() {
    if (messageListEl) {
      messageListEl.scrollTop = messageListEl.scrollHeight;
    }
  }

  $effect(() => {
    messages.length;
    untrack(() => {
      tick().then(scrollToBottom);
    });
  });

  $effect(() => {
    input;
    untrack(() => {
      const el = textareaEl;
      if (!el) return;
      el.style.height = 'auto';
      el.style.height = Math.min(el.scrollHeight, 200) + 'px';
    });
  });

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  }

  // ---- Render helpers ----------------------------------------------

  function isToolGroup(m: ChatHistoryMessage): boolean {
    return !!(m as any).tool_calls && Array.isArray((m as any).tool_calls) && (m as any).tool_calls.length > 0;
  }
  function toolCalls(m: ChatHistoryMessage): Array<{ id?: string; name?: string; arguments?: any }> {
    return ((m as any).tool_calls as any[]) || [];
  }

  /**
   * Split an assistant bubble's raw content into:
   *   • thinking: every `<think>…</think>` block (or an unclosed
   *     `<think>…` if the stream was cut off mid-thought)
   *   • body:    the rest of the content, with the think blocks
   *     stripped, trimmed of leading/trailing whitespace
   *
   * Most providers (DeepSeek / MiniMax-M3 / Qwen) emit
   * `<think>` inline in the regular content_delta. Anthropic /
   * OpenAI o-series emit them on a separate `thinking_delta`
   * channel, which the SSE handler above folds back in by
   * wrapping the deltas in `<think>` markers. Either path ends
   * up here with the same shape, so the render code can stay
   * branch-free.
   */
  function parseAssistantContent(content: string): { thinking: string[]; body: string } {
    if (!content) return { thinking: [], body: '' };
    const thinking: string[] = [];
    const re = /<think>([\s\S]*?)(?:<\/think>|$)/g;
    let body = content;
    let m: RegExpExecArray | null;
    while ((m = re.exec(content)) !== null) {
      const t = m[1].trim();
      if (t.length > 0) thinking.push(t);
    }
    // Strip ALL `<think>…</think>` and any unterminated
    // `<think>…` tail from the body before markdown rendering,
    // so the user never sees the raw tags.
    body = body.replace(/<think>[\s\S]*?(?:<\/think>|$)/g, '');
    body = body.replace(/\n{3,}/g, '\n\n').trim();
    return { thinking, body };
  }
</script>

<div class="flex flex-col h-full">
  <!-- Header: agent picker + session title + actions -->
  <div class="border-b border-zinc-800 px-4 py-2 flex items-center gap-3">
    <select
      class="h-8 bg-zinc-900 border border-zinc-700 rounded px-2 text-sm"
      value={activeAgent?.id || ''}
      onchange={async (e) => {
        const id = (e.currentTarget as HTMLSelectElement).value;
        const a = agents.find((x) => x.id === id);
        if (a) {
          activeAgent = a;
          await refetchSessions();
          await loadHistory();
          void goto(`/agents/${a.id}/chat/`);
        }
      }}
    >
      {#each agents as a (a.id)}
        <option value={a.id}>{a.name || a.id}</option>
      {/each}
    </select>
    <div class="flex-1 min-w-0">
      {#if editingTitle}
        <div class="flex items-center gap-2">
          <input
            type="text"
            bind:value={titleDraft}
            class="h-7 bg-zinc-900 border border-zinc-700 rounded px-2 text-sm"
            placeholder="Session title"
          />
          <button type="button" class="text-xs text-violet-300" onclick={saveTitle}>Save</button>
          <button type="button" class="text-xs text-zinc-500" onclick={() => (editingTitle = false)}>Cancel</button>
        </div>
      {:else}
        <div class="flex items-center gap-2">
          <div class="text-sm font-medium truncate">
            {activeSessionId
              ? (sessions.find((s) => s.key === activeSessionId)?.title || activeSessionId)
              : 'New chat'}
          </div>
          {#if activeSessionId}
            <button type="button" class="text-zinc-500 hover:text-zinc-300 text-xs" onclick={startRename} title="Rename session">✎</button>
            <button type="button" class="text-zinc-500 hover:text-red-400 text-xs" onclick={removeSession} title="Delete session">🗑</button>
          {/if}
        </div>
      {/if}
    </div>
    <button
      type="button"
      class="text-xs px-2 py-1 rounded border border-zinc-700 hover:bg-zinc-800/60"
      onclick={newChat}
    >+ New chat</button>
  </div>

  <!-- Message list -->
  <div
    bind:this={messageListEl}
    class="flex-1 overflow-y-auto px-4 py-4 space-y-3"
  >
    {#if loading}
      <div class="text-center text-sm text-zinc-500 mt-12">Loading…</div>
    {:else if !activeAgent}
      <div class="text-center text-sm text-zinc-500 mt-12">
        No agents yet. <a href="/agents/" class="text-violet-300 hover:underline">Create one</a>.
      </div>
    {:else if messages.length === 0}
      <div class="text-center text-sm text-zinc-500 mt-12">
        {activeSessionId ? 'This session is empty.' : 'Send a message to start a new chat.'}
      </div>
    {/if}

    {#each messages as m, i (i)}
      {#if m.role === 'user'}
        <div class="flex justify-end">
          <div class="max-w-[80%] rounded-lg px-3 py-2 text-sm bg-violet-600/20 border border-violet-600/30 whitespace-pre-wrap break-words">
            {m.content}
          </div>
        </div>
      {:else if isToolGroup(m)}
        <div class="flex justify-start">
          <div class="max-w-[80%] rounded-lg border border-zinc-700 bg-zinc-900/40 text-xs">
            <details>
              <summary class="px-3 py-1.5 cursor-pointer text-zinc-300">Tool calls ({toolCalls(m).length})</summary>
              <div class="px-3 pb-2 space-y-1">
                {#each toolCalls(m) as tc, j (j)}
                  <div class="font-mono text-[11px] text-zinc-400">{tc.name}{tc.arguments ? ` · ${typeof tc.arguments === 'string' ? tc.arguments.slice(0, 80) : JSON.stringify(tc.arguments).slice(0, 80)}` : ''}</div>
                {/each}
              </div>
            </details>
          </div>
        </div>
      {:else}
        <div class="flex justify-start">
          <div class="max-w-[80%] rounded-lg bg-zinc-900 border border-zinc-800 overflow-hidden">
            {@render AssistantBubble({ content: m.content })}
          </div>
        </div>
      {/if}
    {/each}

    {#if liveAssistant.toolCalls.length > 0}
      <div class="flex justify-start">
        <div class="max-w-[80%] rounded-lg border border-zinc-700 bg-zinc-900/40 text-xs">
          <details open>
            <summary class="px-3 py-1.5 cursor-pointer text-zinc-300">Tool calls ({liveAssistant.toolCalls.length})</summary>
            <div class="px-3 pb-2 space-y-2">
              {#each liveAssistant.toolCalls as tc, j (j)}
                <div class="space-y-1">
                  <div class="font-mono text-[11px] text-zinc-300">{tc.name}</div>
                  {#if tc.arguments}
                    <pre class="text-[10px] text-zinc-500 whitespace-pre-wrap break-words">{tc.arguments}</pre>
                  {/if}
                  {#if tc.result !== undefined}
                    <pre class="text-[10px] text-zinc-500 whitespace-pre-wrap break-words border-t border-zinc-800 pt-1 mt-1">{tc.result}</pre>
                  {/if}
                </div>
              {/each}
            </div>
          </details>
        </div>
      </div>
    {/if}

    {#if liveAssistant.content && liveMarker.current && !messages.find((x) => x === liveMarker.current && (x as any).__settled)}
      <div class="flex justify-start">
        <div class="max-w-[80%] rounded-lg bg-zinc-900 border border-zinc-800 overflow-hidden">
          {@render AssistantBubble({ content: liveAssistant.content, streaming: true })}
        </div>
      </div>
    {/if}
  </div>

  <!-- Composer -->
  <form onsubmit={send} class="border-t border-zinc-800 p-3 flex gap-2 items-end">
    <textarea
      bind:this={textareaEl}
      bind:value={input}
      onkeydown={onKey}
      rows={1}
      placeholder="Type a message…"
      disabled={sending || !activeAgent}
      class="flex-1 resize-none bg-zinc-900 border border-zinc-700 rounded px-3 py-2 text-sm focus:border-violet-500 focus:ring-1 focus:ring-violet-500 outline-none disabled:opacity-50"
    ></textarea>
    <button
      type="submit"
      disabled={sending || !input.trim() || !activeAgent}
      class="h-9 px-3 rounded bg-violet-600 hover:bg-violet-500 disabled:opacity-40 text-sm font-medium"
    >{sending ? 'Sending…' : 'Send'}</button>
  </form>

  {#if error}
    <p class="px-3 pb-2 text-xs text-red-400">{error}</p>
  {/if}
</div>

{#snippet AssistantBubble(args: { content: string; streaming?: boolean })}
  {@const content = args.content}
  {@const streaming = args.streaming ?? false}
  {@const parsed = parseAssistantContent(content)}
  {#if parsed.thinking.length > 0}
    <details class="border-b border-zinc-800 bg-zinc-950/60 group" open={streaming}>
      <summary class="px-3 py-1.5 text-[11px] uppercase tracking-wider text-zinc-500 cursor-pointer flex items-center gap-2 hover:text-zinc-300 select-none">
        <span class="text-zinc-400 group-open:rotate-90 transition-transform inline-block">▸</span>
        <span>💭</span>
        <span>{parsed.thinking.length === 1 ? 'Thought' : `Thoughts (${parsed.thinking.length})`}</span>
        {#if streaming}<span class="text-violet-400 normal-case tracking-normal">streaming…</span>{/if}
      </summary>
      <div class="px-3 py-2 space-y-2 border-t border-zinc-800/60">
        {#each parsed.thinking as t, i (i)}
          <pre class="text-[11px] text-zinc-400 italic whitespace-pre-wrap break-words font-sans">{t}</pre>
        {/each}
      </div>
    </details>
  {/if}
  {#if parsed.body}
    <div class="px-3 py-2 text-sm prose prose-invert prose-sm max-w-none">
      <MarkdownLink content={parsed.body} />
    </div>
  {:else if streaming}
    <div class="px-3 py-2 text-xs text-zinc-500 italic">…</div>
  {/if}
{/snippet}
