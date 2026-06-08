<script lang="ts">
  import { onMount } from 'svelte';
  import { adminListUsers, adminUpdateUserRole, adminDeleteUser, type UserInfo } from '$lib/api';
  import Card from '$lib/components/ui/Card.svelte';
  import Badge from '$lib/components/ui/Badge.svelte';

  let users = $state<UserInfo[]>([]);
  let loading = $state(true);
  let error = $state('');

  async function refresh() {
    loading = true;
    try {
      const r = await adminListUsers();
      users = (r.users ?? []).sort((a, b) => (a.username || '').localeCompare(b.username || ''));
    } catch (e) { error = (e as Error).message; } finally { loading = false; }
  }

  async function setRole(id: string, role: string) {
    try {
      await adminUpdateUserRole(id, role as any);
      await refresh();
    } catch (e) { alert((e as Error).message); }
  }

  async function remove(id: string, username: string) {
    if (!confirm(`Delete user ${username}?`)) return;
    try {
      await adminDeleteUser(id);
      await refresh();
    } catch (e) { alert((e as Error).message); }
  }

  onMount(refresh);
</script>

<div class="p-6 max-w-4xl mx-auto space-y-4">
  <div class="flex items-center gap-3">
    <h2 class="text-2xl font-semibold tracking-tight">Admin · Users</h2>
  </div>
  {#if error}<p class="text-sm text-red-400">{error}</p>{/if}
  <Card>
    {#if loading}
      <p class="text-sm text-zinc-500">Loading…</p>
    {:else}
      <table class="w-full text-sm">
        <thead>
          <tr class="text-left text-zinc-500 border-b border-zinc-800">
            <th class="py-2">ID</th>
            <th class="py-2">Username</th>
            <th class="py-2">Email</th>
            <th class="py-2">Role</th>
            <th class="py-2"></th>
          </tr>
        </thead>
        <tbody>
          {#each users as u (u.id)}
            <tr class="border-b border-zinc-800/50">
              <td class="py-2 font-mono text-xs">{u.id}</td>
              <td class="py-2">{u.username}</td>
              <td class="py-2 text-zinc-500 text-xs">{u.email || '—'}</td>
              <td class="py-2">
                <select
                  class="h-7 bg-zinc-900 border border-zinc-700 rounded px-2 text-xs"
                  value={u.role}
                  onchange={(e) => setRole(u.id, (e.currentTarget as HTMLSelectElement).value)}
                >
                  <option value="user">user</option>
                  <option value="admin">admin</option>
                  <option value="super_admin">super_admin</option>
                </select>
              </td>
              <td class="py-2 text-right">
                <button class="text-xs text-red-400 hover:underline" onclick={() => remove(u.id, u.username)}>Delete</button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </Card>
</div>
