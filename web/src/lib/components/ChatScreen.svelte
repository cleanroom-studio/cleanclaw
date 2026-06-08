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

  import { onMount, tick, untrack } from "svelte";
  import { page } from "$app/state";
  import { goto } from "$app/navigation";
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
  } from "$lib/api";
  import ChatRowActions from "./ChatRowActions.svelte";
  import MarkdownLink from "./MarkdownLink.svelte";

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
  const routeAgentId = $derived(agentIdFromPath(page.url?.pathname || ""));
  const routeProjectId = $derived(projectIdFromPath(page.url?.pathname || ""));
  const queryAgentId = $derived(page.url?.searchParams.get("agent"));
  const querySessionId = $derived(page.url?.searchParams.get("session"));
  const pathnameOnly = $derived(page.url?.pathname || "");
  // /chat/<sessionId> path-segment form
  const pathSession = $derived(
    (() => {
      const m = (page.url?.pathname || "").match(
        /^\/(chat|agents\/[^/]+\/chat)\/([^/?#]+)/,
      );
      return m ? decodeURIComponent(m[2]) : null;
    })(),
  );
  const activeSessionId = $derived(querySessionId || pathSession || null);
  /// When the sidebar "New chat" is clicked it navigates with
  /// `?fresh=1` — we detect that here and treat it as a new
  /// chat request (same as `freshNonce > 0`).
  const isFreshQuery = $derived(
    page.url?.searchParams.get("fresh") === "1",
  );

  // ---- State --------------------------------------------------------

  let agents = $state<AgentInfo[]>([]);
  let activeAgent = $state<AgentInfo | null>(null);
  let sessions = $state<SessionInfo[]>([]);
  let messages = $state<ChatHistoryMessage[]>([]);
  let input = $state("");
  let sending = $state(false);
  let error = $state("");
  let loading = $state(true);
  let editingTitle = $state(false);
  let titleDraft = $state("");
  let messageListEl: HTMLElement | undefined = $state();
  let textareaEl: HTMLTextAreaElement | undefined = $state();
  let liveAssistant = $state<{
    content: string;
    toolCalls: {
      id: string;
      name: string;
      arguments: string;
      result?: string;
    }[];
  }>({ content: "", toolCalls: [] });
  /// Bumped on every "New chat" click so the route effect can
  /// distinguish a fresh empty state from a route that just
  /// happens to have no `?session=` query. Without this, the
  /// effect's `activeSessionId || sorted[0]?.key` fallback
  /// would silently re-load the most recent session every time
  /// the user clicks "+ New chat".
  let freshNonce = $state(0);

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
      error = (e as Error).message || "failed to load agents";
    } finally {
      loading = false;
    }
  });

  // Re-run when the active agent / session changes via the URL.
  // `freshNonce` is the "+ New chat" toggle — when the user
  // clicks it we want a blank slate even though the URL is
  // still `/agents/<id>/chat/`. Including it in the key makes
  // the effect fire, and `onRouteChange` reads it to clear
  // `messages` instead of loading the most recent session.
  // The full `page.url.search` is also part of the key so the
  // sidebar's "New chat" link (which appends a cache-busting
  // `?_=<ts>` query to force a real nav) re-fires the effect.
  let lastRouteKey = '';
  $effect(() => {
    const key = `${routeAgentId || queryAgentId || ''}|${activeSessionId || ''}|${freshNonce}|${page.url?.search || ''}`;
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
    // `freshNonce` got bumped since the last run OR the
    // sidebar navigated with `?fresh=1` → the user explicitly
    // asked for a clean slate. Wipe the bubble stack and skip
    // the history fetch entirely (so we don't silently
    // re-populate from the most recent session).
    if (freshNonce > 0 || isFreshQuery) {
      if (!activeSessionId) {
        messages = [];
        liveAssistant = { content: "", toolCalls: [] };
        // Clean up the `?fresh=1` query param after consuming it.
        if (isFreshQuery && activeAgent) {
          history.replaceState(null, "", `/agents/${activeAgent.id}/chat/`);
        }
        return;
      }
    }
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
    //   2. nothing — fall through to the empty hint
    // (Previously we silently re-loaded the most recent session
    // whenever the URL had no `?session=`. That broke the
    // "+ New chat" flow: every click on New chat would pull up
    // the previous conversation. The fresh-empty path is now
    // handled by the route effect checking `freshNonce` and
    // bailing before `loadHistory` runs.)
    const key = activeSessionId || '';
    if (!key) {
      messages = [];
      liveAssistant = { content: '', toolCalls: [] };
      return;
    }
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
    if (typeof window !== "undefined" && activeAgent) {
      window.dispatchEvent(
        new CustomEvent("cleanclaw:sessions-changed", {
          detail: { agentId: activeAgent.id },
        }),
      );
    }
  }

  // ---- Sending a turn ----------------------------------------------

  async function send(e?: Event) {
    e?.preventDefault();
    if (!activeAgent) {
      error = "Pick an agent first";
      return;
    }
    const text = input.trim();
    if (!text || sending) return;

    // Snapshot the session key we'll send to. If the user is
    // parked on a project route, we carry the project_id through.
    const sessionKey = activeSessionId || `sk_${Date.now().toString(36)}`;

    // Optimistically append the user bubble.
    messages = [
      ...messages,
      { role: "user", content: text, created_at: new Date().toISOString() },
    ];
    input = "";
    liveAssistant = { content: "", toolCalls: [] };
    sending = true;
    error = "";

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
      error = (e as Error).message || "chat failed";
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
      case "content_delta": {
        const delta = evt.data.delta || "";
        if (!delta) break;
        // Coalesce into a single in-flight assistant bubble.
        liveAssistant.content += delta;
        // Mirror liveAssistant into messages by replacing the
        // last assistant entry if it exists; otherwise append.
        const idx = messages.findIndex(
          (m) => m.role === "assistant" && m === liveMarker.current,
        );
        if (idx >= 0) {
          messages = messages.map((m, i) =>
            i === idx ? { ...m, content: liveAssistant.content } : m,
          );
        } else {
          const marker: any = {
            role: "assistant",
            content: liveAssistant.content,
            created_at: new Date().toISOString(),
          };
          liveMarker.current = marker;
          messages = [...messages, marker];
        }
        break;
      }
      case "thinking_delta": {
        // Some providers stream thinking on a separate channel
        // (Anthropic extended thinking, OpenAI o-series). When
        // the hub emits these, fold them into the in-flight
        // assistant bubble as a `<think>` block so the render
        // path can parse + collapse them like any other think
        // tag. Markers are inserted at the start so deltas land
        // inside even before any regular content has streamed.
        const delta = evt.data.delta || "";
        if (delta && !liveAssistant.content.includes("<think>")) {
          liveAssistant.content =
            `<think>${delta}</think>\n` + liveAssistant.content;
        } else if (delta) {
          // Already has an opening tag — splice before the
          // closing `</think>` (or at end if the tag is still
          // open mid-stream).
          const close = liveAssistant.content.indexOf("</think>");
          if (close >= 0) {
            liveAssistant.content =
              liveAssistant.content.slice(0, close) +
              delta +
              liveAssistant.content.slice(close);
          } else {
            liveAssistant.content += delta;
          }
        }
        const idx = messages.findIndex((m) => m === liveMarker.current);
        if (idx >= 0) {
          messages = messages.map((m, i) =>
            i === idx ? { ...m, content: liveAssistant.content } : m,
          );
        }
        break;
      }
      case "tool_call": {
        // Attach to the current in-flight assistant.
        liveAssistant.toolCalls.push({
          id: evt.data.id || "",
          name: evt.data.name || "",
          arguments:
            typeof evt.data.arguments === "string"
              ? evt.data.arguments
              : JSON.stringify(evt.data.arguments || {}),
        });
        // The CleanClaw pattern coalesces tool-call round into
        // a tool-group rendered as a single collapsed bubble;
        // Svelte version emits a compact inline card for each.
        break;
      }
      case "tool_result": {
        const tc = liveAssistant.toolCalls.find((t) => t.id === evt.data.id);
        if (tc) tc.result = evt.data.result || "";
        break;
      }
      case "done": {
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
                (a, b) =>
                  new Date(b.updated_at).getTime() -
                  new Date(a.updated_at).getTime(),
              );
              const key = activeSessionId || sorted[0]?.key || "";
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
      case "error": {
        messages = [
          ...messages,
          {
            role: "assistant",
            content: `⚠️ ${evt.data.message || "error"}`,
            created_at: new Date().toISOString(),
          },
        ];
        break;
      }
    }
  }

  // Marker ref for the in-flight assistant bubble. A plain
  // object so we can compare by reference in the message array.
  const liveMarker = { current: null as any };

  // ---- Session actions ---------------------------------------------

  function newChat() {
    if (!activeAgent) return;
    // Bump `freshNonce` so the route effect's key changes and
    // `onRouteChange` clears `messages` without falling back
    // to the most-recent session. The URL we navigate to is
    // the same shape the user is already on in most cases
    // (`/agents/<id>/chat/`), so we rely on the nonce to
    // re-trigger the effect rather than the URL.
    freshNonce = freshNonce + 1;
    void goto(`/agents/${activeAgent.id}/chat/`, { replaceState: true, noScroll: true });
  }

  async function startRename() {
    if (!activeSessionId) return;
    const sess = sessions.find((s) => s.key === activeSessionId);
    titleDraft = sess?.title || "";
    editingTitle = true;
  }

  async function saveTitle() {
    if (!activeAgent || !activeSessionId) return;
    try {
      await renameChatSession(
        activeAgent.id,
        activeSessionId,
        titleDraft.trim(),
      );
      editingTitle = false;
      await refetchSessions();
      broadcastSessionsChanged();
    } catch (e) {
      alert((e as Error).message || "rename failed");
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
      alert((e as Error).message || "delete failed");
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
      el.style.height = "auto";
      el.style.height = Math.min(el.scrollHeight, 200) + "px";
    });
  });

  function onKey(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  }

  // ---- Render helpers ----------------------------------------------

  function isToolGroup(m: ChatHistoryMessage): boolean {
    return (
      !!(m as any).tool_calls &&
      Array.isArray((m as any).tool_calls) &&
      (m as any).tool_calls.length > 0
    );
  }
  function toolCalls(
    m: ChatHistoryMessage,
  ): Array<{ id?: string; name?: string; arguments?: any }> {
    return ((m as any).tool_calls as any[]) || [];
  }

  /** Pretty-print a tool-call `arguments` payload. May already
   * be a JSON string (the SSE shape) or a plain object. */
  function formatToolArgs(args: any): string {
    if (args == null || args === "") return "";
    if (typeof args === "string") {
      // The agent layer often hands us a serialized JSON string
      // with stray wrapping — try to pretty-print it if it
      // parses, otherwise just show the raw text.
      try {
        return JSON.stringify(JSON.parse(args), null, 2);
      } catch {
        return args;
      }
    }
    try {
      return JSON.stringify(args, null, 2);
    } catch {
      return String(args);
    }
  }

  /**
   * Parse the web_search provider's plain-text result format
   * into a list of {title, url, snippet} cards. The agent's
   * web_search tool (Brave / DuckDuckGo / Bing / Google / Baidu)
   * all emit a `Search results for: <q>\n\n1. Title\n   url\n   snippet\n\n…`
   * shape; we recognize that here.
   *
   * Returns `null` when the content doesn't look like a
   * search-result blob (so the caller falls through to the
   * generic "render as text" path).
   */
  function parseWebSearchResults(content: string): {
    query: string;
    results: Array<{ index: number; title: string; url: string; snippet: string }>;
  } | null {
    if (!content) return null;
    const m = content.match(/^Search results for:\s*(.+?)\s*$/m);
    if (!m) return null;
    const query = m[1].trim();
    // The rest of the body alternates `N. <title>` lines with
    // `   <url>` and `   <snippet>` lines, separated by blank
    // lines. We split on the blank lines, then parse each block.
    const after = content.slice(m.index ?? 0 + m[0].length).trim();
    const blocks = after
      .split(/\n\s*\n/)
      .map((b) => b.trim())
      .filter((b) => b.length > 0);
    const results: Array<{ index: number; title: string; url: string; snippet: string }> = [];
    for (const block of blocks) {
      const lines = block.split("\n");
      const head = lines[0].match(/^(\d+)\.\s+(.+)$/);
      if (!head) continue;
      const idx = parseInt(head[1], 10);
      const title = head[2].trim();
      let url = "";
      let snippet = "";
      for (const ln of lines.slice(1)) {
        const trimmed = ln.trim();
        if (!url && /^https?:\/\//.test(trimmed)) {
          url = trimmed;
        } else if (url && trimmed) {
          // First non-URL line after the URL is the snippet.
          snippet = trimmed;
          break;
        }
      }
      if (title || url) results.push({ index: idx, title, url, snippet });
    }
    if (results.length === 0) return null;
    return { query, results };
  }

  /** True when the content looks like a tool error envelope
   * (`[tool error: ...]`) so we can render a red alert card. */
  function isToolError(content: string): boolean {
    return /^\s*\[tool\s*error\s*[:：]/i.test(content || "");
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
  function parseAssistantContent(content: string): {
    thinking: string[];
    body: string;
  } {
    if (!content) return { thinking: [], body: "" };
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
    body = body.replace(/<think>[\s\S]*?(?:<\/think>|$)/g, "");
    body = body.replace(/\n{3,}/g, "\n\n").trim();
    return { thinking, body };
  }
</script>

<div class="flex flex-col h-full">
  <!-- Header: agent picker + session title + actions -->
  <div class="border-b border-zinc-800 px-4 py-2 flex items-center gap-3">
    <select
      class="h-8 bg-zinc-900 border border-zinc-700 rounded px-2 text-sm"
      value={activeAgent?.id || ""}
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
          <button
            type="button"
            class="text-xs text-violet-300"
            onclick={saveTitle}>Save</button
          >
          <button
            type="button"
            class="text-xs text-zinc-500"
            onclick={() => (editingTitle = false)}>Cancel</button
          >
        </div>
      {:else}
        <div class="flex items-center gap-2">
          <div class="text-sm font-medium truncate">
            {activeSessionId
              ? sessions.find((s) => s.key === activeSessionId)?.title ||
                activeSessionId
              : "New chat"}
          </div>
          {#if activeSessionId}
            <button
              type="button"
              class="text-zinc-500 hover:text-zinc-300 text-xs"
              onclick={startRename}
              title="Rename session">✎</button
            >
            <button
              type="button"
              class="text-zinc-500 hover:text-red-400 text-xs"
              onclick={removeSession}
              title="Delete session">🗑</button
            >
          {/if}
        </div>
      {/if}
    </div>
    <button
      type="button"
      class="text-xs px-2 py-1 rounded border border-zinc-700 hover:bg-zinc-800/60"
      onclick={newChat}>+ New chat</button
    >
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
        No agents yet. <a
          href="/agents/"
          class="text-violet-300 hover:underline">Create one</a
        >.
      </div>
    {:else if messages.length === 0}
      <div class="text-center text-sm text-zinc-500 mt-12">
        {activeSessionId
          ? "This session is empty."
          : "Send a message to start a new chat."}
      </div>
    {/if}

    {#each messages as m, i (i)}
      {#if m.role === "user"}
        <div class="flex justify-end">
          <div
            class="max-w-[80%] rounded-lg px-3 py-2 text-sm bg-violet-600/20 border border-violet-600/30 whitespace-pre-wrap break-words"
          >
            {m.content}
          </div>
        </div>
      {:else if isToolGroup(m)}
        <!-- Assistant's tool-call message: one card per call with
             name + pretty-printed arguments. Open by default so
             users see what the model asked for without an extra
             click; users can collapse to reduce noise. -->
        <div class="flex justify-start">
          <div
            class="max-w-[80%] rounded-lg border border-zinc-700 bg-zinc-900/40 text-xs"
          >
            <div class="px-3 py-1.5 flex items-center gap-2 text-zinc-300">
              <span>🔧</span>
              <span class="font-medium">Tool calls ({toolCalls(m).length})</span>
            </div>
            <div class="px-3 pb-2 space-y-2">
              {#each toolCalls(m) as tc, j (j)}
                <div class="rounded border border-zinc-800 bg-zinc-950/40 p-2">
                  <div class="flex items-center gap-2 mb-1">
                    <span class="font-mono text-[11px] text-violet-300">{tc.name}</span>
                    {#if (m as any).tool_calls && (m as any).tool_calls[j]?.id}
                      <span class="text-[10px] text-zinc-600 font-mono">{(m as any).tool_calls[j].id}</span>
                    {/if}
                  </div>
                  {#if tc.arguments !== undefined && tc.arguments !== null && tc.arguments !== ""}
                    <pre
                      class="text-[10px] text-zinc-400 whitespace-pre-wrap break-words font-mono bg-zinc-950/50 rounded p-1.5 mt-1 max-h-60 overflow-auto">{formatToolArgs(tc.arguments)}</pre>
                  {/if}
                </div>
              {/each}
            </div>
          </div>
        </div>
      {:else if m.role === "tool"}
        <!-- Tool result message: the agent runtime sends one of
             these per tool call. We render three flavors:
             • web-search results → nice cards
             • error envelope     → red alert
             • everything else    → monospace pre + header -->
        {@const searchResults = parseWebSearchResults(m.content || "")}
        {@const isError = isToolError(m.content || "")}
        <div class="flex justify-start">
          <div
            class="max-w-[80%] rounded-lg border bg-zinc-900/40 overflow-hidden text-sm {isError ? 'border-red-700/60' : 'border-zinc-700'}"
          >
            <div class="px-3 py-1.5 flex items-center gap-2 text-xs {isError ? 'text-red-300 bg-red-950/30' : 'text-zinc-300 bg-zinc-900/60'}">
              <span>{isError ? '⚠️' : '📥'}</span>
              <span class="font-medium">{(m as any).name || 'tool'} result</span>
              {#if (m as any).tool_call_id}
                <span class="text-[10px] text-zinc-500 font-mono">{(m as any).tool_call_id}</span>
              {/if}
            </div>
            <div class="px-3 py-2">
              {#if searchResults}
                <div class="space-y-2">
                  <div class="text-xs text-zinc-500">
                    Search results for <span class="text-zinc-300 font-medium">{searchResults.query}</span>
                  </div>
                  {#each searchResults.results as r (r.index)}
                    <a
                      href={r.url}
                      target="_blank"
                      rel="noopener noreferrer"
                      class="block rounded border border-zinc-800 bg-zinc-950/40 p-2 hover:border-zinc-600 hover:bg-zinc-900/60 transition-colors"
                    >
                      <div class="flex items-start gap-2">
                        <span class="text-[10px] text-zinc-500 font-mono mt-0.5 shrink-0 w-4 text-right">{r.index}.</span>
                        <div class="min-w-0 flex-1">
                          <div class="text-violet-300 text-[13px] font-medium truncate">
                            {r.title || '(no title)'}
                          </div>
                          {#if r.url}
                            <div class="text-[10px] text-zinc-500 font-mono truncate">{r.url}</div>
                          {/if}
                          {#if r.snippet}
                            <div class="text-xs text-zinc-400 mt-0.5 line-clamp-3">{r.snippet}</div>
                          {/if}
                        </div>
                      </div>
                    </a>
                  {/each}
                </div>
              {:else if isError}
                <pre class="text-xs text-red-200 whitespace-pre-wrap break-words font-mono">{m.content}</pre>
              {:else}
                <pre class="text-xs text-zinc-300 whitespace-pre-wrap break-words font-mono max-h-96 overflow-auto">{m.content}</pre>
              {/if}
            </div>
          </div>
        </div>
      {:else}
        <div class="flex justify-start">
          <div
            class="max-w-[80%] rounded-lg bg-zinc-900 border border-zinc-800 overflow-hidden"
          >
            {@render AssistantBubble({ content: m.content })}
          </div>
        </div>
      {/if}
    {/each}

    {#if liveAssistant.toolCalls.length > 0}
      <div class="flex justify-start">
        <div
          class="max-w-[80%] rounded-lg border border-zinc-700 bg-zinc-900/40 text-xs"
        >
          <div class="px-3 py-1.5 flex items-center gap-2 text-zinc-300">
            <span>🔧</span>
            <span class="font-medium">Tool calls ({liveAssistant.toolCalls.length})</span>
            {#if liveAssistant.content}
              <span class="text-zinc-500">·</span>
              <span class="text-zinc-500">running…</span>
            {/if}
          </div>
          <div class="px-3 pb-2 space-y-2">
            {#each liveAssistant.toolCalls as tc, j (j)}
              <div class="rounded border border-zinc-800 bg-zinc-950/40 p-2">
                <div class="flex items-center gap-2 mb-1">
                  <span class="font-mono text-[11px] text-violet-300">{tc.name}</span>
                  {#if tc.id}
                    <span class="text-[10px] text-zinc-600 font-mono">{tc.id}</span>
                  {/if}
                  <span class="ml-auto text-[10px] {tc.result !== undefined ? 'text-emerald-400' : 'text-amber-400'}">
                    {tc.result !== undefined ? '✓ done' : '⏳ running'}
                  </span>
                </div>
                {#if tc.arguments}
                  <pre
                    class="text-[10px] text-zinc-400 whitespace-pre-wrap break-words font-mono bg-zinc-950/50 rounded p-1.5 mt-1 max-h-60 overflow-auto">{formatToolArgs(tc.arguments)}</pre>
                {/if}
                {#if tc.result !== undefined}
                  {@const searchResults = parseWebSearchResults(tc.result)}
                  {#if searchResults}
                    <div class="mt-1 space-y-1">
                      <div class="text-[10px] text-zinc-500">Results for <span class="text-zinc-300">{searchResults.query}</span></div>
                      {#each searchResults.results as r (r.index)}
                        <a
                          href={r.url}
                          target="_blank"
                          rel="noopener noreferrer"
                          class="block rounded border border-zinc-800 bg-zinc-950/50 p-1.5 hover:border-zinc-600 transition-colors"
                        >
                          <div class="text-[11px] text-violet-300 truncate">{r.title || '(no title)'}</div>
                          {#if r.snippet}<div class="text-[10px] text-zinc-400 line-clamp-2">{r.snippet}</div>{/if}
                          {#if r.url}<div class="text-[9px] text-zinc-500 font-mono truncate">{r.url}</div>{/if}
                        </a>
                      {/each}
                    </div>
                  {:else if isToolError(tc.result)}
                    <pre class="text-[10px] text-red-300 whitespace-pre-wrap break-words font-mono mt-1 bg-red-950/30 rounded p-1.5">{tc.result}</pre>
                  {:else}
                    <pre
                      class="text-[10px] text-zinc-300 whitespace-pre-wrap break-words font-mono mt-1 max-h-60 overflow-auto bg-zinc-950/50 rounded p-1.5">{tc.result}</pre>
                  {/if}
                {/if}
              </div>
            {/each}
          </div>
        </div>
      </div>
    {/if}

    {#if liveAssistant.content && liveMarker.current && !messages.find((x) => x === liveMarker.current && (x as any).__settled)}
      <div class="flex justify-start">
        <div
          class="max-w-[80%] rounded-lg bg-zinc-900 border border-zinc-800 overflow-hidden"
        >
          {@render AssistantBubble({
            content: liveAssistant.content,
            streaming: true,
          })}
        </div>
      </div>
    {/if}
  </div>

  <!-- Composer -->
  <form
    onsubmit={send}
    class="border-t border-zinc-800 p-3 flex gap-2 items-end"
  >
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
      >{sending ? "Sending…" : "Send"}</button
    >
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
    <details
      class="border-b border-zinc-800 bg-zinc-950/60 group"
      open={streaming}
    >
      <summary
        class="px-3 py-1.5 text-[11px] uppercase tracking-wider text-zinc-500 cursor-pointer flex items-center gap-2 hover:text-zinc-300 select-none"
      >
        <span
          class="text-zinc-400 group-open:rotate-90 transition-transform inline-block"
          >▸</span
        >
        <span>💭</span>
        <span
          >{parsed.thinking.length === 1
            ? "Thought"
            : `Thoughts (${parsed.thinking.length})`}</span
        >
        {#if streaming}<span class="text-violet-400 normal-case tracking-normal"
            >streaming…</span
          >{/if}
      </summary>
      <div class="px-3 py-2 space-y-2 border-t border-zinc-800/60">
        {#each parsed.thinking as t, i (i)}
          <pre
            class="text-[11px] text-zinc-400 italic whitespace-pre-wrap break-words font-sans">{t}</pre>
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
