<script lang="ts">
  import { onMount } from "svelte";
  import { adminListUsage, type UsageInfo } from "$lib/api";
  import Card from "$lib/components/ui/Card.svelte";

  let usage = $state<UsageInfo[]>([]);
  let loading = $state(true);
  let error = $state("");

  onMount(async () => {
    try {
      const r = await adminListUsage();
      usage = r.usage ?? [];
    } catch (e) {
      error = (e as Error).message;
    } finally {
      loading = false;
    }
  });
</script>

<div class="p-6 max-w-4xl mx-auto space-y-4">
  <h2 class="text-2xl font-semibold tracking-tight">Admin · Token Usage</h2>
  <p class="text-sm text-zinc-400">Aggregate token consumption.</p>
  {#if error}<p class="text-sm text-red-400">{error}</p>{/if}
  <Card>
    {#if loading}
      <p class="text-sm text-zinc-500">Loading…</p>
    {:else if usage.length === 0}
      <p class="text-sm text-zinc-500">No usage recorded yet.</p>
    {:else}
      <table class="w-full text-sm">
        <thead>
          <tr class="text-left text-zinc-500 border-b border-zinc-800">
            <th class="py-2">Agent</th>
            <th class="py-2">Period</th>
            <th class="py-2">Input</th>
            <th class="py-2">Output</th>
            <th class="py-2">Total</th>
          </tr>
        </thead>
        <tbody>
          {#each usage as u (u.agent_id + u.period)}
            <tr class="border-b border-zinc-800/50">
              <td class="py-2 font-mono text-xs">{u.agent_id}</td>
              <td class="py-2 text-zinc-500">{u.period}</td>
              <td class="py-2">{u.input_tokens.toLocaleString()}</td>
              <td class="py-2">{u.output_tokens.toLocaleString()}</td>
              <td class="py-2 font-medium">{u.total_tokens.toLocaleString()}</td
              >
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </Card>
</div>
