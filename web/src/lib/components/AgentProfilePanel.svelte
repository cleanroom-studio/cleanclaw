<script lang="ts">
  // AgentProfilePanel — read-only summary card. Mirrors
  // .
  // Used inside the customize dialog / chat header to show
  // avatar + name + role + quota.

  import { getAgent, type AgentDetail } from '$lib/api';

  let { agentId }: { agentId: string } = $props();

  let agent = $state<AgentDetail | null>(null);
  let loading = $state(true);

  $effect(() => {
    if (!agentId) return;
    let cancelled = false;
    (async () => {
      loading = true;
      try {
        const r = await getAgent(agentId);
        if (!cancelled) agent = r.agent;
      } catch {
      } finally {
        if (!cancelled) loading = false;
      }
    })();
    return () => { cancelled = true; };
  });
</script>

<div class="flex items-center gap-3">
  <div class="h-12 w-12 rounded-xl bg-violet-600/20 flex items-center justify-center text-violet-300 font-semibold">
    {#if agent?.avatar_url}
      <img src={agent.avatar_url} alt={agent.name || ''} class="h-12 w-12 rounded-xl object-cover" />
    {:else}
      {(agent?.name || agentId).slice(0, 1).toUpperCase()}
    {/if}
  </div>
  <div class="min-w-0">
    {#if loading}
      <div class="text-sm text-zinc-500">Loading…</div>
    {:else}
      <div class="text-base font-semibold truncate">{agent?.name || agentId}</div>
      <div class="text-xs text-zinc-500 truncate">{agent?.model}</div>
      {#if agent?.description}
        <div class="text-xs text-zinc-400 truncate mt-0.5">{agent.description}</div>
      {/if}
    {/if}
  </div>
</div>
