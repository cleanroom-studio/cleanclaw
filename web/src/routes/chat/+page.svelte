<script lang="ts">
  // Chat — top-level /chat/ surface. When no agent is pinned
  // via ?agent=<id>, render the picker and the chat once the
  // caller picks one. When an agent is pinned, render the
  // shared ChatScreen component (same component mounted
  // under /agents/<id>/chat/).

  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { page } from '$app/state';
  import { listAgents, type AgentInfo } from '$lib/api';
  import ChatScreen from '$lib/components/ChatScreen.svelte';

  const queryAgent = $derived(page.url?.searchParams.get('agent') || '');

  let agents = $state<AgentInfo[]>([]);
  let loading = $state(true);

  onMount(async () => {
    try {
      const r = await listAgents();
      agents = r.agents ?? [];
      // If no agent picked, redirect to the first available
      // agent's chat — the bare `/chat/` is only useful when
      // zero agents exist (in which case we render the empty
      // hint below).
      if (!queryAgent && agents.length > 0) {
        await goto(`/chat/?agent=${encodeURIComponent(agents[0].id)}`, { replaceState: true });
      }
    } finally {
      loading = false;
    }
  });
</script>

{#if loading}
  <div class="p-6 text-sm text-zinc-400">Loading…</div>
{:else if agents.length === 0}
  <div class="p-6 max-w-2xl mx-auto space-y-4">
    <h1 class="text-2xl font-bold">Chat</h1>
    <p class="text-sm text-zinc-400">
      No agents yet. <a href="/agents/" class="text-violet-300 hover:underline">Create one</a> to start chatting.
    </p>
  </div>
{:else}
  <ChatScreen />
{/if}
