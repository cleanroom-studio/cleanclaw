<script lang="ts">
  import AgentAccessGate from "$lib/components/AgentAccessGate.svelte";

  let { children } = $props();
</script>

<AgentAccessGate>
  {@render children()}
</AgentAccessGate>
