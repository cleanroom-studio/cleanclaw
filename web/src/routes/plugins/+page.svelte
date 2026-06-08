<script lang="ts">
  import { onMount } from "svelte";
  import { listPlugins, togglePlugin, type PluginInfo } from "$lib/api";
  import Card from "$lib/components/ui/Card.svelte";
  import Switch from "$lib/components/ui/Switch.svelte";

  let plugins = $state<PluginInfo[]>([]);
  let loading = $state(true);
  let error = $state("");

  async function refresh() {
    loading = true;
    try {
      const r = await listPlugins();
      plugins = r.plugins ?? [];
    } catch (e) {
      error = (e as Error).message;
    } finally {
      loading = false;
    }
  }

  async function toggle(id: string, enabled: boolean) {
    try {
      await togglePlugin(id, enabled);
      await refresh();
    } catch (e) {
      error = (e as Error).message;
    }
  }

  onMount(refresh);
</script>

<div class="p-6 max-w-4xl mx-auto space-y-4">
  <div>
    <h2 class="text-2xl font-semibold tracking-tight">Plugins</h2>
    <p class="text-sm text-zinc-400 mt-1">
      Runtime hooks and tool extensions loaded from $CLEANCLAW_HOME/plugins.
    </p>
  </div>

  {#if error}<p class="text-sm text-red-400">{error}</p>{/if}

  <Card>
    {#if loading}
      <p class="text-sm text-zinc-500">Loading…</p>
    {:else if plugins.length === 0}
      <p class="text-sm text-zinc-500">No plugins installed.</p>
    {:else}
      <ul class="divide-y divide-zinc-800">
        {#each plugins as p (p.id)}
          <li class="py-3 flex items-center gap-3">
            <div class="flex-1">
              <div class="text-sm font-medium">{p.name || p.id}</div>
              {#if p.description}<div class="text-xs text-zinc-500">
                  {p.description}
                </div>{/if}
              {#if p.version}<div class="text-xs text-zinc-500">
                  v{p.version}
                </div>{/if}
            </div>
            <Switch
              checked={p.enabled}
              onchange={() => toggle(p.id, !p.enabled)}
            />
          </li>
        {/each}
      </ul>
    {/if}
  </Card>
</div>
