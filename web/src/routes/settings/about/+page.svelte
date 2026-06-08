<script lang="ts">
  import { onMount } from "svelte";
  import { getStatus } from "$lib/api";
  import Card from "$lib/components/ui/Card.svelte";

  let version = $state("…");
  let uptime = $state("…");
  let running = $state(true);

  onMount(async () => {
    try {
      const r = await getStatus();
      version = r.version || "dev";
      uptime = r.uptime || "—";
      running = !!r.running;
    } catch {}
  });
</script>

<div class="p-6 max-w-2xl mx-auto space-y-4">
  <h2 class="text-2xl font-semibold tracking-tight">About</h2>
  <Card>
    <dl class="grid grid-cols-2 gap-y-2 text-sm">
      <dt class="text-zinc-500">Version</dt>
      <dd><code class="bg-zinc-800 px-1.5 py-0.5 rounded">{version}</code></dd>
      <dt class="text-zinc-500">Uptime</dt>
      <dd>{uptime}</dd>
      <dt class="text-zinc-500">Running</dt>
      <dd>{running ? "✓" : "✗"}</dd>
    </dl>
    <p class="text-xs text-zinc-500 mt-3">
      Build details appear on the gateway binary. Check <code
        >./target/release/cleanclaw --version</code
      > for the git commit.
    </p>
  </Card>
</div>
