<script lang="ts">
  // AgentSwitcher — dropdown in the sidebar header. Mirrors
  // . Lists
  // every agent the caller can see, allows selecting a different
  // one, and surfaces "Manage agents" when admin / quota permits.

  import { goto } from '$app/navigation';
  import type { AgentInfo } from '$lib/api';

  let {
    agents,
    activeAgentId,
    locked = false,
  }: {
    agents: AgentInfo[];
    activeAgentId: string | null;
    locked?: boolean;
  } = $props();

  let open = $state(false);

  const active = $derived(agents.find((a) => a.id === activeAgentId) || null);
  const label = $derived(active?.name || activeAgentId || 'Select agent');

  function selectAgent(id: string) {
    open = false;
    void goto(`/agents/${id}/chat/`);
  }

  function manage() {
    open = false;
    void goto('/agents/');
  }
</script>

<div class="relative">
  <button
    type="button"
    onclick={() => (open = !open)}
    class="w-full flex items-center gap-2 px-2 py-1.5 rounded hover:bg-zinc-800/60 text-left"
  >
    <div class="h-7 w-7 rounded-md bg-violet-600/30 flex items-center justify-center text-xs font-semibold">
      {(label || '?').slice(0, 1).toUpperCase()}
    </div>
    <div class="flex-1 min-w-0">
      <div class="text-sm font-medium truncate">{label}</div>
      {#if active}
        <div class="text-[10px] text-zinc-500 truncate">{active.model}</div>
      {/if}
    </div>
    <span class="text-zinc-500 text-xs">{open ? '▴' : '▾'}</span>
  </button>

  {#if open}
    <div
      class="absolute z-30 left-0 right-0 mt-1 bg-zinc-900 border border-zinc-700 rounded-md shadow-lg max-h-80 overflow-y-auto"
    >
      <ul class="py-1">
        {#each agents as a (a.id)}
          <li>
            <button
              type="button"
              class="w-full text-left px-3 py-1.5 text-sm hover:bg-zinc-800/60 flex items-center gap-2"
              class:bg-violet-600={a.id === activeAgentId}
              class:text-white={a.id === activeAgentId}
              onclick={() => selectAgent(a.id)}
            >
              <div class="h-5 w-5 rounded bg-zinc-700 flex items-center justify-center text-[10px]">
                {(a.name || a.id).slice(0, 1).toUpperCase()}
              </div>
              <div class="flex-1 min-w-0">
                <div class="truncate">{a.name || a.id}</div>
                <div class="text-[10px] text-zinc-400 truncate font-mono">{a.model}</div>
              </div>
              {#if a.role === 'viewer'}
                <span class="text-[10px] text-amber-400">viewer</span>
              {/if}
            </button>
          </li>
        {/each}
        {#if agents.length === 0}
          <li class="px-3 py-2 text-sm text-zinc-500">No agents yet.</li>
        {/if}
        {#if !locked}
          <li class="border-t border-zinc-800 mt-1">
            <button
              type="button"
              class="w-full text-left px-3 py-1.5 text-sm text-violet-300 hover:bg-zinc-800/60"
              onclick={manage}
            >
              Manage agents →
            </button>
          </li>
        {/if}
      </ul>
    </div>
  {/if}
</div>
