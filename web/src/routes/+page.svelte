<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { getStatus } from "$lib/api";

  // Index route — if no users exist, send to /onboard;
  // otherwise show a tiny sign-in prompt (the actual login
  // form is rendered by AuthGuard, but the index path
  // bypasses the AppShell so we land on the bare login).

  onMount(async () => {
    try {
      const s = await getStatus();
      // If the platform is unconfigured, push to /onboard.
      if (!s.user_count && !s.configured) {
        await goto("/onboard/");
      }
    } catch {}
  });
</script>

<div
  class="min-h-screen flex items-center justify-center text-zinc-400 text-sm"
>
  Redirecting…
</div>
