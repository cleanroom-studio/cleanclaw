<script lang="ts">
  // Admin index — small landing that surfaces the three
  // admin-only sub-pages.

  import { onMount } from "svelte";
  import { getMe } from "$lib/api";
  import { Card } from "$lib/components/ui/card/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";

  let role = $state("");
  onMount(async () => {
    try {
      const r = await getMe();
      role = r.user?.role || "";
    } catch {}
  });
</script>

<div class="p-6 max-w-3xl mx-auto space-y-4">
  <div class="flex items-center gap-3">
    <h2 class="text-2xl font-semibold tracking-tight">Admin</h2>
    <Badge>{role}</Badge>
  </div>
  <div class="grid grid-cols-3 gap-3">
    <a href="/admin/users/" class="block">
      <Card>
        <div class="text-sm font-semibold">Users</div>
        <div class="text-xs text-zinc-500 mt-1">
          Create, role, delete accounts.
        </div>
      </Card>
    </a>
    <a href="/admin/chats/" class="block">
      <Card>
        <div class="text-sm font-semibold">Chats</div>
        <div class="text-xs text-zinc-500 mt-1">
          All chat sessions across every agent.
        </div>
      </Card>
    </a>
    <a href="/admin/usage/" class="block">
      <Card>
        <div class="text-sm font-semibold">Token Usage</div>
        <div class="text-xs text-zinc-500 mt-1">
          Aggregate per-agent / per-period tokens.
        </div>
      </Card>
    </a>
  </div>
</div>
