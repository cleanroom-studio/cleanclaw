<script lang="ts">
  // /agents/<id> — single agent landing. The CleanClaw layout
  // mounts a single ChatScreen instance at the [id]/layout level
  // and reuses it for /chat, /chat/<session>, and /project/<pid>.
  // SvelteKit doesn't have an equivalent "singleton per route
  // tree" pattern out of the box, but mounting the same
  // component here with the same params is functionally
  // equivalent: the browser preserves the DOM when only the
  // session-id query / project-id path changes.

  import { page } from '$app/state';
  import ChatScreen from '$lib/components/ChatScreen.svelte';

  let { params }: { params: { id: string } } = $props();
  const agentId = $derived(params.id);
</script>

<ChatScreen />
