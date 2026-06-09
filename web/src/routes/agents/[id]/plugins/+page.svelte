<script lang="ts">
  import { onMount } from "svelte";
  import AppShell from "$lib/components/AppShell.svelte";
  import { Card } from "$lib/components/ui/card/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Switch } from "$lib/components/ui/switch/index.js";
  import { listPlugins, togglePlugin, type PluginInfo } from "$lib/api";

  let { params }: { params: { id: string } } = $props();
  let plugins = $state<PluginInfo[]>([]);
  let loading = $state(true);

  async function refresh() {
    loading = true;
    try {
      const r = await listPlugins();
      plugins = r.plugins ?? [];
    } catch {
      plugins = [];
    } finally {
      loading = false;
    }
  }

  async function toggle(id: string, enabled: boolean) {
    try {
      await togglePlugin(id, enabled);
      await refresh();
    } catch {}
  }

  onMount(refresh);
</script>

<AppShell>
  <div class="space-y-4">
    <h1 class="text-2xl font-bold">Plugins</h1>
    <Card>
      <h2 class="text-base font-semibold mb-3">
        Enabled plugins for this agent
      </h2>
      {#if loading}
        <p class="text-sm text-muted-foreground">Loading…</p>
      {:else if plugins.length === 0}
        <p class="text-sm text-muted-foreground">No plugins installed.</p>
      {:else}
        <ul class="divide-y divide-border">
          {#each plugins as p}
            <li class="py-3 flex items-center gap-3">
              <div class="flex-1">
                <div class="text-sm font-medium">{p.name}</div>
                {#if p.description}
                  <div class="text-xs text-muted-foreground">
                    {p.description}
                  </div>
                {/if}
              </div>
              <Switch
                checked={p.enabled}
                onchange={() => toggle(p.id, !p.enabled)}
              />
            </li>
          {/each}
        </ul>
      {/if}
      <div class="mt-4">
        <Button size="sm" variant="outline" onclick={refresh}>Refresh</Button>
      </div>
    </Card>
  </div>
</AppShell>
