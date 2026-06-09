<script lang="ts">
  // Runtime settings — placeholder for the deployment-wide
  // knobs (sandbox backend, port, log level). Only super_admin
  // can edit; the page bounces non-admins back to /overview/.

  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { getMe } from "$lib/api";
  import { Card } from "$lib/components/ui/card/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";

  let role = $state("");
  let sandboxBackend = $state("docker");
  let sandboxImage = $state("cleanclaw/sandbox:latest");
  let logLevel = $state("info");

  onMount(async () => {
    try {
      const r = await getMe();
      role = r.user?.role || "";
      if (role !== "super_admin") {
        await goto("/overview/");
      }
    } catch {}
  });
</script>

<div class="p-6 max-w-2xl mx-auto space-y-4">
  <h2 class="text-2xl font-semibold tracking-tight">Runtime</h2>
  <p class="text-sm text-zinc-400">
    Deployment-wide knobs. Changes take effect on the next gateway restart.
  </p>
  <Card>
    <div class="space-y-3">
      <div>
        <Label for="rt-sb">Sandbox backend</Label>
        <Input
          id="rt-sb"
          bind:value={sandboxBackend}
          placeholder="docker / e2b / boxlite"
        />
      </div>
      <div>
        <Label for="rt-si">Sandbox image</Label>
        <Input id="rt-si" bind:value={sandboxImage} />
      </div>
      <div>
        <Label for="rt-ll">Log level</Label>
        <Input
          id="rt-ll"
          bind:value={logLevel}
          placeholder="info / debug / warn"
        />
      </div>
      <p class="text-xs text-zinc-500">
        Saving is a no-op in this build — the values are read by the gateway on
        boot.
      </p>
    </div>
  </Card>
</div>
