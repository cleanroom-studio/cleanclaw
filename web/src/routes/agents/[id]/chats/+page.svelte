<script lang="ts">
  // Per-agent chats list.
  import { onMount } from 'svelte';
  import Card from '$lib/components/ui/Card.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import Badge from '$lib/components/ui/Badge.svelte';
  import { listChatSessions, type SessionInfo } from '$lib/api';

  let { params }: { params: { id: string } } = $props();
  let sessions = $state<SessionInfo[]>([]);
  let loading = $state(true);

  async function refresh() {
    loading = true;
    try {
      const r = await listChatSessions(params.id);
      sessions = (r.sessions ?? []).sort(
        (a, b) => new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime(),
      );
    } catch {
      sessions = [];
    } finally {
      loading = false;
    }
  }

  onMount(refresh);
</script>

<div class="p-6 max-w-4xl mx-auto space-y-4">
  <div class="flex items-center justify-between">
    <h2 class="text-2xl font-semibold tracking-tight">Chats</h2>
    <Button size="sm" variant="outline" onclick={refresh}>Refresh</Button>
  </div>
  <Card>
    <h3 class="text-sm font-semibold mb-3">Recent chats</h3>
    {#if loading}
      <p class="text-sm text-zinc-500">Loading…</p>
    {:else if sessions.length === 0}
      <p class="text-sm text-zinc-500">No chats yet for this agent.</p>
    {:else}
      <ul class="divide-y divide-zinc-800">
        {#each sessions as s (s.key)}
          <li class="py-2 flex items-center gap-3">
            <Badge variant="outline">{s.channel || 'web'}</Badge>
            <span class="text-sm flex-1 truncate">{s.title || s.key}</span>
            <span class="text-xs text-zinc-500">{s.message_count} msgs</span>
            <a href="/agents/{params.id}/chat/?session={s.key}" class="text-violet-300 text-xs hover:underline">Open</a>
          </li>
        {/each}
      </ul>
    {/if}
  </Card>
</div>
