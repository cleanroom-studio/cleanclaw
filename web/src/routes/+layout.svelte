<script lang="ts">
  import "../app.css";
  import { onMount } from "svelte";
  import AuthGuard from "$lib/components/AuthGuard.svelte";
  import AppShell from "$lib/components/AppShell.svelte";

  let { children } = $props();

  onMount(() => {
    document.documentElement.classList.add("dark");
  });
</script>

<AuthGuard>
  <AppShell>
    {@render children()}
  </AppShell>
</AuthGuard>
