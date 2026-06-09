<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { listAgents, createAgent, type AgentInfo } from "$lib/api";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Card } from "$lib/components/ui/card/index.js";
  import { Input } from "$lib/components/ui/input/index.js";

  let agents = $state<AgentInfo[]>([]);
  let name = $state("");
  let model = $state("openai/MiniMax-M3");
  let description = $state("");
  let isPublic = $state(false);
  let creating = $state(false);
  let error = $state("");

  async function refresh() {
    try {
      const r = await listAgents();
      agents = r.agents ?? [];
    } catch (e) {
      error = (e as Error).message || "failed to load";
    }
  }

  async function create(e: Event) {
    e.preventDefault();
    creating = true;
    error = "";
    try {
      const r = await createAgent({
        name,
        model,
        description,
        is_public: isPublic,
      });
      name = "";
      description = "";
      await refresh();
    } catch (e) {
      error = (e as Error).message || "failed to create";
    } finally {
      creating = false;
    }
  }

  onMount(refresh);
</script>

<div class="p-6 max-w-5xl mx-auto space-y-6">
  <div>
    <h2 class="text-2xl font-semibold tracking-tight">Agents</h2>
    <p class="text-sm text-zinc-400 mt-1">
      LLM-backed personas. Each agent gets its own workspace, skills, channels,
      and chat history.
    </p>
  </div>

  <Card>
    <h3 class="text-sm font-semibold mb-3">Create new agent</h3>
    <form onsubmit={create} class="space-y-3">
      <div class="grid grid-cols-2 gap-3">
        <div>
          <label for="an-name" class="text-xs text-zinc-400">Name</label>
          <Input id="an-name" bind:value={name} placeholder="alpha" required />
        </div>
        <div>
          <label for="an-model" class="text-xs text-zinc-400">Model</label>
          <Input
            id="an-model"
            bind:value={model}
            placeholder="openai/MiniMax-M3"
            required
          />
        </div>
      </div>
      <div>
        <label for="an-desc" class="text-xs text-zinc-400">Description</label>
        <Input
          id="an-desc"
          bind:value={description}
          placeholder="Helpful assistant"
        />
      </div>
      <div class="flex items-center gap-2">
        <input
          id="an-pub"
          type="checkbox"
          bind:checked={isPublic}
          class="h-4 w-4"
        />
        <label for="an-pub" class="text-xs text-zinc-300"
          >Public (shareable URL)</label
        >
      </div>
      {#if error}
        <p class="text-sm text-red-400">{error}</p>
      {/if}
      <Button type="submit" disabled={creating}
        >{creating ? "Creating…" : "Create"}</Button
      >
    </form>
  </Card>

  <Card>
    <h3 class="text-sm font-semibold mb-3">All agents</h3>
    {#if agents.length === 0}
      <p class="text-sm text-zinc-500">No agents yet.</p>
    {:else}
      <table class="w-full text-sm">
        <thead>
          <tr class="text-left text-zinc-500 border-b border-zinc-800">
            <th class="py-2">ID</th>
            <th class="py-2">Name</th>
            <th class="py-2">Model</th>
            <th class="py-2">Role</th>
            <th class="py-2"></th>
          </tr>
        </thead>
        <tbody>
          {#each agents as a (a.id)}
            <tr class="border-b border-zinc-800/50">
              <td class="py-2 font-mono text-xs">{a.id}</td>
              <td class="py-2">
                <a
                  href="/agents/{a.id}/"
                  class="text-violet-300 hover:underline">{a.name || a.id}</a
                >
              </td>
              <td class="py-2 text-zinc-400 font-mono text-xs">{a.model}</td>
              <td class="py-2 text-zinc-400 text-xs">{a.role || "owner"}</td>
              <td class="py-2 text-right">
                <a
                  href="/agents/{a.id}/chat/"
                  class="text-xs text-violet-300 hover:underline">Open</a
                >
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </Card>
</div>
