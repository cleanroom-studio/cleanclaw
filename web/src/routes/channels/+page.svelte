<script lang="ts">
  import { onMount } from "svelte";
  import { listGlobalChannels, listChannels } from "$lib/api";
  import { Card } from "$lib/components/ui/card/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";

  let globalTypes = $state<
    Array<{ type: string; label: string; configured: boolean; logo: string }>
  >([]);
  let myChannels = $state<
    Array<{ type: string; account_id: string; enabled: boolean }>
  >([]);
  let error = $state("");

  onMount(async () => {
    try {
      const g = await listGlobalChannels();
      globalTypes = g.channels ?? [];
    } catch (e) {
      error = (e as Error).message;
    }
    try {
      const m = await listChannels();
      myChannels = m.channels ?? [];
    } catch {}
  });
</script>

<div class="p-6 max-w-4xl mx-auto space-y-4">
  <div>
    <h2 class="text-2xl font-semibold tracking-tight">Channels</h2>
    <p class="text-sm text-zinc-400 mt-1">
      Each channel is a real-time IM (Telegram, Discord, Slack, …) that an agent
      can use to receive and reply to messages. Per-agent bindings live on each
      agent's Channels tab.
    </p>
  </div>

  {#if error}<p class="text-sm text-red-400">{error}</p>{/if}

  <Card>
    <h3 class="text-sm font-semibold mb-3">Available channels</h3>
    <div class="grid grid-cols-2 md:grid-cols-3 gap-3">
      {#each globalTypes as t (t.type)}
        <div
          class="rounded-lg border border-zinc-800 bg-zinc-900/50 p-3 flex items-center gap-3"
        >
          <img src="/channels/{t.logo}" alt={t.label} class="h-8 w-8" />
          <div class="flex-1 min-w-0">
            <div class="text-sm font-semibold truncate">{t.label}</div>
            <Badge variant={t.configured ? "success" : "secondary"}>
              {t.configured ? "configured" : "not configured"}
            </Badge>
          </div>
        </div>
      {/each}
    </div>
  </Card>

  <Card>
    <h3 class="text-sm font-semibold mb-3">Your connected channels</h3>
    {#if myChannels.length === 0}
      <p class="text-sm text-zinc-500">
        No channels connected. Open an agent's Channels tab to connect one.
      </p>
    {:else}
      <ul class="divide-y divide-zinc-800">
        {#each myChannels as c (c.type + ":" + c.account_id)}
          <li class="py-2 flex items-center gap-3">
            <Badge variant="outline">{c.type}</Badge>
            <span class="text-sm font-mono text-xs">{c.account_id}</span>
            <span class="text-xs text-zinc-500 ml-auto"
              >{c.enabled ? "enabled" : "disabled"}</span
            >
          </li>
        {/each}
      </ul>
    {/if}
  </Card>
</div>
