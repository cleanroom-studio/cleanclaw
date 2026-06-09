<script lang="ts">
  import { onMount } from "svelte";
  import {
    listProviders,
    createProvider,
    deleteProvider,
    testStoredProvider,
    testProvider,
    type ProviderInfo,
  } from "$lib/api";
  import { Card } from "$lib/components/ui/card/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Input } from "$lib/components/ui/input/index.js";

  let providers = $state<ProviderInfo[]>([]);
  let loading = $state(true);
  let error = $state("");

  // Create
  let newType = $state("openai");
  let newModel = $state("MiniMax-M3");
  let newBaseUrl = $state("");
  let newApiKey = $state("");
  let saving = $state(false);
  let msg = $state("");

  // Test inline
  let testApiKey = $state("");
  let testBase = $state("https://api.openai.com/v1");
  let testType = $state("openai");
  let testModel = $state("MiniMax-M3");
  let testing = $state(false);
  let testResult = $state("");

  async function refresh() {
    loading = true;
    try {
      const r = await listProviders();
      providers = r.providers ?? [];
    } catch (e) {
      error = (e as Error).message;
    } finally {
      loading = false;
    }
  }

  async function add(e: Event) {
    e.preventDefault();
    saving = true;
    msg = "";
    try {
      await createProvider({
        type: newType,
        model: newModel,
        base_url: newBaseUrl || undefined,
        api_key: newApiKey,
      });
      newApiKey = "";
      newBaseUrl = "";
      await refresh();
    } catch (err) {
      msg = (err as Error).message || "create failed";
    } finally {
      saving = false;
    }
  }

  async function remove(id: string) {
    if (!confirm("Delete this provider?")) return;
    await deleteProvider(id);
    await refresh();
  }

  async function runTest() {
    testing = true;
    testResult = "";
    try {
      const r = await testProvider({
        type: testType,
        api_base: testBase,
        api_key: testApiKey,
        model: testModel,
      });
      testResult = r.ok ? "✓ OK" : r.message || "✗ failed";
    } catch (e) {
      testResult = (e as Error).message || "test failed";
    } finally {
      testing = false;
    }
  }

  async function test(id: string) {
    try {
      const r = await testStoredProvider(id);
      alert(r.ok ? "✓ provider OK" : r.message || "✗ failed");
    } catch (e) {
      alert((e as Error).message || "test failed");
    }
  }

  onMount(refresh);
</script>

<div class="p-6 max-w-5xl mx-auto space-y-4">
  <div>
    <h2 class="text-2xl font-semibold tracking-tight">Providers</h2>
    <p class="text-sm text-zinc-400 mt-1">
      LLM backends. Add a provider once, then point agents at its model.
    </p>
  </div>

  {#if error}<p class="text-sm text-red-400">{error}</p>{/if}

  <Card>
    <h3 class="text-sm font-semibold mb-3">Configured</h3>
    {#if loading}
      <p class="text-sm text-zinc-500">Loading…</p>
    {:else if providers.length === 0}
      <p class="text-sm text-zinc-500">No providers yet.</p>
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
              <td class="py-2 capitalize">{p.type}</td>
              <td class="py-2 font-mono text-xs">{p.model}</td>
              <td class="py-2 text-xs text-zinc-500 font-mono"
                >{p.base_url || "—"}</td
              >
              <td class="py-2">
                <Badge variant={p.has_api_key ? "success" : "destructive"}>
                  {p.has_api_key ? "set" : "missing"}
                </Badge>
              </td>
              <td class="py-2 text-right space-x-2">
                <Button size="sm" variant="outline" onclick={() => test(p.id)}
                  >Test</Button
                >
                <button
                  class="text-xs text-red-400 hover:underline"
                  onclick={() => remove(p.id)}>Delete</button
                >
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </Card>

  <Card>
    <h3 class="text-sm font-semibold mb-3">Add provider</h3>
    <form onsubmit={add} class="grid grid-cols-2 gap-3">
      <div>
        <label for="p-t" class="text-xs text-zinc-400">Type</label>
        <select
          id="p-t"
          bind:value={newType}
          class="h-9 w-full bg-zinc-900 border border-zinc-700 rounded px-2 text-sm"
        >
          <option value="openai">OpenAI</option>
          <option value="anthropic">Anthropic</option>
          <option value="minimax">minimax</option>
          <option value="ollama">Ollama (local)</option>
          <option value="openrouter">OpenRouter</option>
          <option value="custom">Custom</option>
        </select>
      </div>
      <div>
        <label for="p-m" class="text-xs text-zinc-400">Model</label>
        <Input
          id="p-m"
          bind:value={newModel}
          placeholder="MiniMax-M3"
          required
        />
      </div>
      <div class="col-span-2">
        <label for="p-b" class="text-xs text-zinc-400"
          >Base URL (optional)</label
        >
        <Input
          id="p-b"
          bind:value={newBaseUrl}
          placeholder="https://api.minimaxi.com/v1"
        />
      </div>
      <div class="col-span-2">
        <label for="p-k" class="text-xs text-zinc-400">API key</label>
        <Input id="p-k" type="password" bind:value={newApiKey} required />
      </div>
      {#if msg}<p class="col-span-2 text-sm text-red-400">{msg}</p>{/if}
      <div class="col-span-2">
        <Button type="submit" disabled={saving}
          >{saving ? "Adding…" : "Add"}</Button
        >
      </div>
    </form>
  </Card>

  <Card>
    <h3 class="text-sm font-semibold mb-3">Test a key without storing</h3>
    <p class="text-xs text-zinc-500 mb-3">
      Verify an API key + base URL + model works before saving it as a provider.
    </p>
    <div class="grid grid-cols-2 gap-3">
      <div>
        <label for="tt-t" class="text-xs text-zinc-400">Type</label>
        <select
          id="tt-t"
          bind:value={testType}
          class="h-9 w-full bg-zinc-900 border border-zinc-700 rounded px-2 text-sm"
        >
          <option value="openai">openai</option>
          <option value="anthropic">anthropic</option>
        </select>
      </div>
      <div>
        <label for="tt-m" class="text-xs text-zinc-400">Model</label>
        <Input id="tt-m" bind:value={testModel} />
      </div>
      <div class="col-span-2">
        <label for="tt-b" class="text-xs text-zinc-400">Base URL</label>
        <Input id="tt-b" bind:value={testBase} />
      </div>
      <div class="col-span-2">
        <label for="tt-k" class="text-xs text-zinc-400">API key</label>
        <Input id="tt-k" type="password" bind:value={testApiKey} />
      </div>
      <div class="col-span-2 flex items-center gap-3">
        <Button onclick={runTest} disabled={testing}
          >{testing ? "Testing…" : "Test"}</Button
        >
        {#if testResult}<span class="text-xs text-zinc-300">{testResult}</span
          >{/if}
      </div>
    </div>
  </Card>
</div>
