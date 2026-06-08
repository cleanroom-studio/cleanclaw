<script lang="ts">
  // NavUser — bottom of the sidebar. Shows display name + role.
  // Includes a small Logout button (POST /api/logout).

  import { goto } from '$app/navigation';
  import { logout } from '$lib/api';

  let { name, subtitle }: { name: string; subtitle: string } = $props();

  async function doLogout() {
    try {
      await logout();
    } catch {}
    void goto('/');
  }
</script>

<div class="flex items-center gap-2 px-2 py-1.5 text-sm">
  <div class="h-7 w-7 rounded-full bg-zinc-800 flex items-center justify-center text-xs font-semibold">
    {(name || '?').slice(0, 1).toUpperCase()}
  </div>
  <div class="flex-1 min-w-0">
    <div class="truncate font-medium">{name}</div>
    <div class="truncate text-[10px] text-zinc-500">{subtitle}</div>
  </div>
  <button
    type="button"
    class="text-zinc-500 hover:text-zinc-300 text-xs"
    onclick={doLogout}
    title="Sign out"
  >⎋</button>
</div>
