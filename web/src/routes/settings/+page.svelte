<script lang="ts">
  // Settings index — small landing that points to the four
  // sub-pages. The full settings dialog is mounted from the
  // sidebar (AgentSettingsDialog); this page is a quick
  // navigation entry for power users.

  import Card from '$lib/components/ui/Card.svelte';
</script>

<div class="p-6 max-w-3xl mx-auto space-y-4">
  <h2 class="text-2xl font-semibold tracking-tight">Settings</h2>
  <p class="text-sm text-zinc-400">Pick a sub-page or use the ⚙ Settings button in the sidebar for the dialog version.</p>
  <div class="grid grid-cols-2 gap-3">
    <a href="/settings/account/"><Card><div class="text-sm font-semibold">Account</div><div class="text-xs text-zinc-500 mt-1">Display name, avatar, password.</div></Card></a>
    <a href="/settings/general/"><Card><div class="text-sm font-semibold">General</div><div class="text-xs text-zinc-500 mt-1">Registration toggle + platform defaults.</div></Card></a>
    <a href="/settings/runtime/"><Card><div class="text-sm font-semibold">Runtime</div><div class="text-xs text-zinc-500 mt-1">Sandbox, port, log level (admin only).</div></Card></a>
    <a href="/settings/about/"><Card><div class="text-sm font-semibold">About</div><div class="text-xs text-zinc-500 mt-1">Version + uptime (admin only).</div></Card></a>
  </div>
</div>
