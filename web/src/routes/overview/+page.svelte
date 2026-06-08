<script lang="ts">
  import { onMount } from 'svelte';
  import { getMe, listAgents, type UserInfo, type AgentInfo } from '$lib/api';

  let me = $state<UserInfo | null>(null);
  let agents = $state<AgentInfo[]>([]);
  let error = $state('');

  onMount(async () => {
    try {
      const r = await getMe();
      me = r.user ?? null;
    } catch (e) {
      error = (e as Error).message || 'unauthorized';
    }
    try {
      const r = await listAgents();
      agents = r.agents ?? [];
    } catch {}
  });
</script>

<div class="p-6 max-w-5xl mx-auto space-y-6">
  <div>
    <h2 class="text-2xl font-semibold tracking-tight">Overview</h2>
    <p class="text-sm text-zinc-400 mt-1">A snapshot of the workspace.</p>
  </div>

  {#if error}
    <p class="text-sm text-red-400">{error}</p>
  {/if}

  <div class="grid grid-cols-1 md:grid-cols-3 gap-4">
    <div class="rounded-lg border border-zinc-800 bg-zinc-900/50 p-4">
      <div class="text-xs text-zinc-500 uppercase tracking-wider">User</div>
      <div class="mt-2 text-lg font-semibold">{me?.display_name || me?.username || '—'}</div>
      <div class="text-xs text-zinc-500">{me?.email || ''}</div>
    </div>
    <div class="rounded-lg border border-zinc-800 bg-zinc-900/50 p-4">
      <div class="text-xs text-zinc-500 uppercase tracking-wider">Agents</div>
      <div class="mt-2 text-lg font-semibold">{agents.length}</div>
    </div>
    <div class="rounded-lg border border-zinc-800 bg-zinc-900/50 p-4">
      <div class="text-xs text-zinc-500 uppercase tracking-wider">Role</div>
      <div class="mt-2 text-lg font-semibold">{me?.role || '—'}</div>
    </div>
  </div>

  <div class="rounded-lg border border-zinc-800 bg-zinc-900/50 p-4">
    <h3 class="text-sm font-semibold mb-3">Recent agents</h3>
    {#if agents.length === 0}
      <p class="text-sm text-zinc-500">
        No agents yet. <a href="/agents/" class="text-violet-300 hover:underline">Create one</a>.
      </p>
    {:else}
      <ul class="divide-y divide-zinc-800">
        {#each agents.slice(0, 6) as a (a.id)}
          <li class="py-2 flex items-center justify-between">
            <div>
              <a href="/agents/{a.id}/" class="font-medium hover:underline">{a.name || a.id}</a>
              <div class="text-xs text-zinc-500 font-mono">{a.id}</div>
            </div>
            <span class="text-xs text-zinc-500 font-mono">{a.model}</span>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
</div>
