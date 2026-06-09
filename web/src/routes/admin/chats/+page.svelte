<script lang="ts">
  import { onMount } from "svelte";
  import { adminListChats, type SessionInfo } from "$lib/api";
  import { Card } from "$lib/components/ui/card/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";

  let sessions = $state<SessionInfo[]>([]);
  let loading = $state(true);
  let error = $state("");

  onMount(async () => {
    try {
      const r = await adminListChats();
      sessions = (r.sessions ?? []).sort(
        (a, b) =>
          new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime(),
      );
    } catch (e) {
      error = (e as Error).message;
    } finally {
      loading = false;
    }
  });
</script>

<div class="p-6 max-w-5xl mx-auto space-y-4">
  <h2 class="text-2xl font-semibold tracking-tight">Admin · Chats</h2>
  <p class="text-sm text-zinc-400">All chat sessions across every agent.</p>
  {#if error}<p class="text-sm text-red-400">{error}</p>{/if}
  <Card>
    {#if loading}
      <p class="text-sm text-zinc-500">Loading…</p>
    {:else if sessions.length === 0}
      <p class="text-sm text-zinc-500">No sessions.</p>
    {:else}
      <table class="w-full text-sm">
        <thead>
          <tr class="text-left text-zinc-500 border-b border-zinc-800">
            <th class="py-2">Key</th>
            <th class="py-2">Agent</th>
            <th class="py-2">Channel</th>
            <th class="py-2">Title</th>
            <th class="py-2">Messages</th>
            <th class="py-2">Updated</th>
          </tr>
        </thead>
        <tbody>
          {#each sessions as s (s.key + (s.user_id || ""))}
            <tr class="border-b border-zinc-800/50">
              <td class="py-2 font-mono text-xs">{s.key}</td>
              <td class="py-2 font-mono text-xs">{s.chat_id || "—"}</td>
              <td class="py-2"
                ><Badge variant="outline">{s.channel || "web"}</Badge></td
              >
              <td class="py-2 truncate max-w-xs">{s.title || "—"}</td>
              <td class="py-2">{s.message_count}</td>
              <td class="py-2 text-zinc-500 text-xs"
                >{new Date(s.updated_at).toLocaleString()}</td
              >
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </Card>
</div>
