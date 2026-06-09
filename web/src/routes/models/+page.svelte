<script lang="ts">
  import { onMount } from "svelte";
  import { listModels } from "$lib/api";
  import { Card } from "$lib/components/ui/card/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";

  let models = $state<Array<{ id: string; provider: string; label: string }>>(
    [],
  );
  let loading = $state(true);
  let error = $state("");

  onMount(async () => {
    try {
      const r = await listModels();
      models = r.models ?? [];
    } catch (e) {
      error = (e as Error).message;
    } finally {
      loading = false;
    }
  });
</script>

<div class="p-6 max-w-4xl mx-auto space-y-4">
  <div>
    <h2 class="text-2xl font-semibold tracking-tight">Models</h2>
    <p class="text-sm text-zinc-400 mt-1">
      Available LLM models from every configured provider.
    </p>
  </div>

  <Card>
    {#if loading}
      <p class="text-sm text-zinc-500">Loading…</p>
    {:else if error}
      <p class="text-sm text-red-400">{error}</p>
    {:else if models.length === 0}
      <p class="text-sm text-zinc-500">
        No models. <a href="/providers/" class="text-violet-300 hover:underline"
          >Add a provider</a
        > first.
      </p>
    {:else}
      <ul class="divide-y divide-zinc-800">
        {#each models as m (m.id)}
          <li class="py-2 flex items-center gap-3">
            <Badge variant="outline">{m.provider}</Badge>
            <span class="text-sm font-medium">{m.label || m.id}</span>
            <span class="text-xs text-zinc-500 font-mono ml-auto">{m.id}</span>
          </li>
        {/each}
      </ul>
    {/if}
  </Card>
</div>
