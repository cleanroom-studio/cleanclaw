<script lang="ts">
  // Models — agent-scoped model selection. Mirrors
  // .
  // Three-scope model view (system / agent / user), each
  // agent-scoped override wins over system. Per-row toggle
  // "Inherited" badge for the lower-priority rows.

  import { onMount } from "svelte";
  import {
    listModels,
    listProviders,
    getAgent,
    updateAgent,
    testStoredProvider,
    createProvider,
    updateProvider,
    deleteProvider,
    type AgentDetail,
    type ProviderInfo,
  } from "$lib/api";
  import Button from "$lib/components/ui/Button.svelte";
  import Card from "$lib/components/ui/Card.svelte";
  import Input from "$lib/components/ui/Input.svelte";
  import Badge from "$lib/components/ui/Badge.svelte";

  let { params }: { params: { id: string } } = $props();

  let agent = $state<AgentDetail | null>(null);
  let models = $state<Array<{ id: string; provider: string; label: string }>>(
    [],
  );
  let providers = $state<ProviderInfo[]>([]);
  let selectedModel = $state("");
  let saving = $state(false);
  let msg = $state("");

  // Create / edit form
  let newType = $state("openai");
  let newModel = $state("");
  let newBaseUrl = $state("");
  let newApiKey = $state("");
  let showKey = $state<Record<string, boolean>>({});
  let editingId = $state<string | null>(null);
  let editName = $state("");
  let editModel = $state("");
  let editBaseUrl = $state("");
  let editApiKey = $state("");

  async function load() {
    try {
      const r = await getAgent(params.id);
      agent = r.agent;
      selectedModel = agent.model || "";
    } catch (e) {
      msg = (e as Error).message;
    }
    try {
      const r = await listModels();
      models = r.models ?? [];
    } catch {}
    try {
      const r = await listProviders();
      providers = r.providers ?? [];
    } catch {}
  }

  async function save() {
    saving = true;
    msg = "";
    try {
      await updateAgent(params.id, { model: selectedModel } as any);
      msg = "Saved.";
    } catch (e) {
      msg = (e as Error).message;
    } finally {
      saving = false;
    }
  }

  async function addProvider(e: Event) {
    e.preventDefault();
    try {
      await createProvider({
        type: newType,
        model: newModel,
        base_url: newBaseUrl || undefined,
        api_key: newApiKey,
      });
      newModel = "";
      newBaseUrl = "";
      newApiKey = "";
      const r = await listProviders();
      providers = r.providers ?? [];
      const r2 = await listModels();
      models = r2.models ?? [];
    } catch (err) {
      alert((err as Error).message || "create failed");
    }
  }

  function startEdit(p: ProviderInfo) {
    editingId = p.id;
    editName = p.type || "";
    editModel = p.model || "";
    editBaseUrl = p.base_url || "";
    editApiKey = "";
  }

  async function saveEdit() {
    if (!editingId) return;
    try {
      await updateProvider(editingId, {
        model: editModel,
        base_url: editBaseUrl,
        api_key: editApiKey || undefined,
      } as any);
      editingId = null;
      const r = await listProviders();
      providers = r.providers ?? [];
      const r2 = await listModels();
      models = r2.models ?? [];
    } catch (e) {
      alert((e as Error).message || "update failed");
    }
  }

  async function removeProvider(id: string, name: string) {
    if (!confirm(`Delete provider ${name}?`)) return;
    await deleteProvider(id);
    const r = await listProviders();
    providers = r.providers ?? [];
  }

  async function test(id: string) {
    try {
      const r = await testStoredProvider(id);
      alert(r.ok ? "✓ provider OK" : r.message || "✗ failed");
    } catch (e) {
      alert((e as Error).message || "test failed");
    }
  }

  onMount(load);
</script>

<div class="p-6 max-w-5xl mx-auto space-y-6">
  <div>
    <h2 class="text-2xl font-semibold tracking-tight">
      Models · {agent?.name || params.id}
    </h2>
    <p class="text-sm text-zinc-400 mt-1">
      Pick the model this agent uses. The selection overrides the system
      fallback chain.
    </p>
  </div>

  <Card>
    <h3 class="text-sm font-semibold mb-3">Active model</h3>
    <div class="flex items-end gap-3">
      <div class="flex-1">
        <label for="am-pick" class="text-xs text-zinc-400">Model</label>
        <select
          id="am-pick"
          bind:value={selectedModel}
          class="flex h-9 w-full rounded-md border border-zinc-700 bg-zinc-900 px-3 text-sm"
        >
          <option value="">— use default —</option>
          {#each models as m (m.id)}
            <option value="{m.provider}/{m.id}"
              >{m.label || m.id} · {m.provider}</option
            >
          {/each}
        </select>
      </div>
      <Button onclick={save} disabled={saving}
        >{saving ? "Saving…" : "Save"}</Button
      >
    </div>
    {#if msg}<p class="text-xs text-zinc-400 mt-2">{msg}</p>{/if}
  </Card>

  <Card>
    <h3 class="text-sm font-semibold mb-3">Configured providers</h3>
    {#if providers.length === 0}
      <p class="text-sm text-zinc-500">No providers yet — add one below.</p>
    {:else}
      <table class="w-full text-sm">
        <thead>
          <tr class="text-left text-zinc-500 border-b border-zinc-800">
            <th class="py-2">Type</th>
            <th class="py-2">Model</th>
            <th class="py-2">Base URL</th>
            <th class="py-2">Key</th>
            <th class="py-2"></th>
          </tr>
        </thead>
        <tbody>
          {#each providers as p (p.id)}
            <tr class="border-b border-zinc-800/50">
              <td class="py-2 capitalize">{p.type || "—"}</td>
              <td class="py-2 font-mono text-xs">{p.model || "—"}</td>
              <td class="py-2 font-mono text-xs text-zinc-500"
                >{p.base_url || "—"}</td
              >
              <td class="py-2">
                {#if p.has_api_key}
                  <Badge variant="success">set</Badge>
                {:else}
                  <Badge variant="destructive">missing</Badge>
                {/if}
              </td>
              <td class="py-2 text-right space-x-1">
                <Button size="sm" variant="outline" onclick={() => test(p.id)}
                  >Test</Button
                >
                <Button size="sm" variant="outline" onclick={() => startEdit(p)}
                  >Edit</Button
                >
                <button
                  class="text-xs text-red-400 hover:underline"
                  onclick={() => removeProvider(p.id, p.type || p.id)}
                  >Delete</button
                >
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </Card>

  {#if editingId}
    <Card>
      <h3 class="text-sm font-semibold mb-3">Edit provider</h3>
      <div class="grid grid-cols-2 gap-3">
        <div>
          <label for="ep-m" class="text-xs text-zinc-400">Model</label>
          <Input id="ep-m" bind:value={editModel} />
        </div>
        <div>
          <label for="ep-b" class="text-xs text-zinc-400">Base URL</label>
          <Input id="ep-b" bind:value={editBaseUrl} />
        </div>
        <div class="col-span-2">
          <label for="ep-k" class="text-xs text-zinc-400"
            >API key (leave blank to keep)</label
          >
          <Input id="ep-k" type="password" bind:value={editApiKey} />
        </div>
      </div>
      <div class="flex items-center gap-2 mt-3">
        <Button onclick={saveEdit}>Save</Button>
        <Button variant="outline" onclick={() => (editingId = null)}
          >Cancel</Button
        >
      </div>
    </Card>
  {/if}

  <Card>
    <h3 class="text-sm font-semibold mb-3">Add provider</h3>
    <form onsubmit={addProvider} class="grid grid-cols-2 gap-3">
      <div>
        <label for="np-t" class="text-xs text-zinc-400">Type</label>
        <select
          id="np-t"
          bind:value={newType}
          class="flex h-9 w-full rounded-md border border-zinc-700 bg-zinc-900 px-3 text-sm"
        >
          <option value="openai">openai</option>
          <option value="anthropic">anthropic</option>
          <option value="minimax">minimax</option>
          <option value="ollama">ollama</option>
          <option value="openrouter">openrouter</option>
          <option value="custom">custom</option>
        </select>
      </div>
      <div>
        <label for="np-m" class="text-xs text-zinc-400">Model</label>
        <Input
          id="np-m"
          bind:value={newModel}
          placeholder="MiniMax-M3"
          required
        />
      </div>
      <div class="col-span-2">
        <label for="np-b" class="text-xs text-zinc-400"
          >Base URL (optional)</label
        >
        <Input
          id="np-b"
          bind:value={newBaseUrl}
          placeholder="https://api.openai.com/v1"
        />
      </div>
      <div class="col-span-2">
        <label for="np-k" class="text-xs text-zinc-400">API key</label>
        <Input id="np-k" type="password" bind:value={newApiKey} required />
      </div>
      <div class="col-span-2">
        <Button type="submit">Add</Button>
      </div>
    </form>
  </Card>
</div>
