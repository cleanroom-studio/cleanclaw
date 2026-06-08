<script lang="ts">
  import { onMount } from 'svelte';
  import { listApikeys, createApikey, deleteApikey, rotateApikey, type ApiKeyInfo } from '$lib/api';
  import Card from '$lib/components/ui/Card.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import Badge from '$lib/components/ui/Badge.svelte';
  import Input from '$lib/components/ui/Input.svelte';

  let keys = $state<ApiKeyInfo[]>([]);
  let loading = $state(true);
  let creating = $state(false);
  let name = $state('');
  let type = $state('user');
  let newKey = $state<{ name: string; token: string; type: string } | null>(null);
  let rotatedToken = $state<string | null>(null);
  let error = $state('');

  async function refresh() {
    try {
      const r = await listApikeys();
      keys = r.keys ?? [];
    } catch (e) { error = (e as Error).message; } finally { loading = false; }
  }

  async function create(e: Event) {
    e.preventDefault();
    creating = true; error = '';
    try {
      const r = await createApikey({ name, type: type as any });
      const k = r.key || (r as any).apikey;
      const token = r.secret || (r as any).token || '';
      if (k) {
        newKey = { name: k.name, token, type: k.type };
      } else {
        newKey = { name, token, type };
      }
      name = '';
      await refresh();
    } catch (e) {
      error = (e as Error).message || 'failed';
    } finally { creating = false; }
  }

  async function rotate(id: string) {
    if (!confirm('Rotate this key? The old token stops working immediately.')) return;
    const r = await rotateApikey(id);
    rotatedToken = r.secret || null;
  }

  async function remove(id: string) {
    if (!confirm('Delete this key?')) return;
    await deleteApikey(id);
    await refresh();
  }

  onMount(refresh);
</script>

<div class="p-6 max-w-4xl mx-auto space-y-4">
  <div>
    <h2 class="text-2xl font-semibold tracking-tight">API keys</h2>
    <p class="text-sm text-zinc-400 mt-1">Issue, rotate, and revoke API keys for programmatic access.</p>
  </div>

  <Card>
    <h3 class="text-sm font-semibold mb-3">Mint a new key</h3>
    <form onsubmit={create} class="flex items-end gap-3">
      <div class="flex-1">
        <label for="k-name" class="text-xs text-zinc-400">Name</label>
        <Input id="k-name" bind:value={name} placeholder="prod-bot-1" required />
      </div>
      <div class="w-40">
        <label for="k-type" class="text-xs text-zinc-400">Type</label>
        <select id="k-type" bind:value={type} class="h-9 w-full bg-zinc-900 border border-zinc-700 rounded px-2 text-sm">
          <option value="user">user</option>
          <option value="agent">agent</option>
          {#if false}<option value="admin">admin</option>{/if}
        </select>
      </div>
      <Button type="submit" disabled={creating}>{creating ? 'Minting…' : 'Mint'}</Button>
    </form>
    {#if error}<p class="text-xs text-red-400 mt-2">{error}</p>{/if}
  </Card>

  {#if newKey}
    <Card>
      <h3 class="text-sm font-semibold mb-2">New API key — save it now</h3>
      <p class="text-sm text-zinc-400 mb-2">This is the only time you'll see this token.</p>
      <pre class="bg-zinc-900 p-3 rounded text-xs overflow-x-auto font-mono">{newKey.token}</pre>
      <button class="text-xs text-violet-300 mt-2" onclick={() => (newKey = null)}>Dismiss</button>
    </Card>
  {/if}

  {#if rotatedToken}
    <Card>
      <h3 class="text-sm font-semibold mb-2">Rotated key — save it now</h3>
      <pre class="bg-zinc-900 p-3 rounded text-xs overflow-x-auto font-mono">{rotatedToken}</pre>
      <button class="text-xs text-violet-300 mt-2" onclick={() => (rotatedToken = null)}>Dismiss</button>
    </Card>
  {/if}

  <Card>
    <h3 class="text-sm font-semibold mb-3">Existing keys</h3>
    {#if loading}
      <p class="text-sm text-zinc-500">Loading…</p>
    {:else if keys.length === 0}
      <p class="text-sm text-zinc-500">No keys yet.</p>
    {:else}
      <table class="w-full text-sm">
        <thead>
          <tr class="text-left text-zinc-500 border-b border-zinc-800">
            <th class="py-2">ID</th>
            <th class="py-2">Name</th>
            <th class="py-2">Type</th>
            <th class="py-2">Prefix</th>
            <th class="py-2"></th>
          </tr>
        </thead>
        <tbody>
          {#each keys as k (k.id)}
            <tr class="border-b border-zinc-800/50">
              <td class="py-2 font-mono text-xs">{k.id}</td>
              <td class="py-2">{k.name || '—'}</td>
              <td class="py-2"><Badge variant="outline">{k.type}</Badge></td>
              <td class="py-2 font-mono text-xs">{k.key_prefix}</td>
              <td class="py-2 text-right space-x-2">
                <button class="text-xs text-violet-300 hover:underline" onclick={() => rotate(k.id)}>Rotate</button>
                <button class="text-xs text-red-400 hover:underline" onclick={() => remove(k.id)}>Delete</button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </Card>
</div>
