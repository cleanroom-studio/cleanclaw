<script lang="ts">
  // NavSessions — flat list of recent chat sessions for the
  // active agent.
  // (`NavSessions`). The list re-sorts by `updated_at` so the
  // most recently active session floats to the top. Clicking a
  // row navigates into the chat with that session pre-selected;
  // a small delete button removes the session.

  import { goto } from '$app/navigation';
  import { deleteChatSession } from '$lib/api';
  import type { SessionInfo } from '$lib/api';

  let {
    agentId,
    sessions,
  }: { agentId: string; sessions: SessionInfo[] } = $props();

  const sorted = $derived(
    [...(sessions || [])].sort(
      (a, b) => new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime(),
    ),
  );

  function open(key: string) {
    void goto(`/agents/${agentId}/chat/?session=${encodeURIComponent(key)}`);
  }

  async function remove(key: string, e: Event) {
    e.stopPropagation();
    if (!confirm(`Delete session ${key}?`)) return;
    try {
      await deleteChatSession(agentId, key);
      // Broadcast so the sidebar re-fetches and the chat surface
      // resets if it was viewing this session.
      window.dispatchEvent(
        new CustomEvent('cleanclaw:sessions-changed', { detail: { agentId } }),
      );
    } catch (e) {
      alert((e as Error).message || 'delete failed');
    }
  }

  function preview(s: SessionInfo): string {
    if (s.title) return s.title;
    if (s.preview) return s.preview;
    return s.key;
  }
</script>

<div>
  <div class="px-2 py-1 text-[10px] uppercase tracking-wider text-zinc-500 flex items-center justify-between">
    <span>Sessions</span>
    <span class="text-zinc-600 normal-case tracking-normal">{sorted.length}</span>
  </div>
  <ul class="space-y-0.5">
    {#each sorted as s (s.key)}
      <li class="group flex items-center gap-1">
        <button
          type="button"
          class="flex-1 text-left px-2 py-1 rounded text-sm hover:bg-zinc-800/60 truncate"
          onclick={() => open(s.key)}
        >
          <div class="truncate text-zinc-200">{preview(s)}</div>
          <div class="truncate text-[10px] text-zinc-500 font-mono">{s.key}</div>
        </button>
        <button
          type="button"
          class="opacity-0 group-hover:opacity-100 text-zinc-500 hover:text-red-400 px-1"
          title="Delete session"
          onclick={(e) => remove(s.key, e)}
        >×</button>
      </li>
    {/each}
    {#if sorted.length === 0}
      <li class="px-2 py-1 text-xs text-zinc-600">No sessions yet.</li>
    {/if}
  </ul>
</div>
