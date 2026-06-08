<script lang="ts">
  import { onMount } from 'svelte';
  import AppShell from '$lib/components/AppShell.svelte';
  import Card from '$lib/components/ui/Card.svelte';
  import { agentUsage, type UsageInfo } from '$lib/api';

  let { params }: { params: { id: string } } = $props();
  let usage = $state<UsageInfo[]>([]);
  let loading = $state(true);
  let totalIn = $state(0);
  let totalOut = $state(0);

  async function refresh() {
    loading = true;
    try {
      const r = await agentUsage(params.id);
      usage = r.usage ?? [];
      totalIn = usage.reduce((a, u) => a + (u.input_tokens || 0), 0);
      totalOut = usage.reduce((a, u) => a + (u.output_tokens || 0), 0);
    } catch {
      usage = [];
    } finally {
      loading = false;
    }
  }

  onMount(refresh);
</script>

<AppShell>
  <div class="space-y-4">
    <h1 class="text-2xl font-bold">Usage</h1>
    <div class="grid grid-cols-3 gap-4">
      <Card>
        <div class="text-xs text-muted-foreground">Input tokens</div>
        <div class="text-2xl font-bold">{totalIn.toLocaleString()}</div>
      </Card>
      <Card>
        <div class="text-xs text-muted-foreground">Output tokens</div>
        <div class="text-2xl font-bold">{totalOut.toLocaleString()}</div>
      </Card>
      <Card>
        <div class="text-xs text-muted-foreground">Total</div>
        <div class="text-2xl font-bold">{(totalIn + totalOut).toLocaleString()}</div>
      </Card>
    </div>
    <Card>
      <h2 class="text-base font-semibold mb-3">By period</h2>
      {#if loading}
        <p class="text-sm text-muted-foreground">Loading…</p>
      {:else if usage.length === 0}
        <p class="text-sm text-muted-foreground">No usage yet for this agent.</p>
      {:else}
        <table class="w-full text-sm">
          <thead>
            <tr class="text-left text-muted-foreground border-b border-border">
              <th class="py-2">Period</th>
              <th class="py-2 text-right">Input</th>
              <th class="py-2 text-right">Output</th>
              <th class="py-2 text-right">Total</th>
            </tr>
          </thead>
          <tbody>
            {#each usage as u}
              <tr class="border-b border-border/50">
                <td class="py-2">{u.period}</td>
                <td class="py-2 text-right font-mono text-xs">{u.input_tokens.toLocaleString()}</td>
                <td class="py-2 text-right font-mono text-xs">{u.output_tokens.toLocaleString()}</td>
                <td class="py-2 text-right font-mono text-xs">{u.total_tokens.toLocaleString()}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </Card>
  </div>
</AppShell>
