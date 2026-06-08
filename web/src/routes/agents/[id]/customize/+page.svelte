<script lang="ts">
  // Customize — agent system prompt / soul / identity editor.
  //
  import { onMount } from "svelte";
  import { page } from "$app/state";
  import { getAgent, updateAgent, type AgentDetail } from "$lib/api";
  import Button from "$lib/components/ui/Button.svelte";
  import Card from "$lib/components/ui/Card.svelte";

  let { params }: { params: { id: string } } = $props();

  let agent = $state<AgentDetail | null>(null);
  let soul = $state("");
  let identity = $state("");
  let systemPrompt = $state("");
  let temperature = $state(0.7);
  let maxTokens = $state(2048);
  let maxIter = $state(8);
  let thinking = $state("");
  let saving = $state(false);
  let msg = $state("");

  async function load() {
    try {
      const r = await getAgent(params.id);
      agent = r.agent;
      soul = agent.soul || "";
      identity = agent.identity || "";
      systemPrompt = agent.system_prompt || "";
      temperature = agent.temperature ?? 0.7;
      maxTokens = agent.max_tokens ?? 2048;
      maxIter = agent.max_tool_iterations ?? 8;
      thinking = agent.thinking || "";
    } catch (e) {
      msg = (e as Error).message || "failed to load";
    }
  }

  async function save() {
    saving = true;
    msg = "";
    try {
      await updateAgent(params.id, {
        soul,
        identity,
        system_prompt: systemPrompt,
        temperature,
        max_tokens: maxTokens,
        max_tool_iterations: maxIter,
        thinking,
      } as any);
      msg = "Saved.";
    } catch (e) {
      msg = (e as Error).message || "save failed";
    } finally {
      saving = false;
    }
  }

  onMount(load);
</script>

<div class="p-6 max-w-3xl mx-auto space-y-4">
  <div>
    <h2 class="text-2xl font-semibold tracking-tight">
      Customize · {agent?.name || params.id}
    </h2>
    <p class="text-sm text-zinc-400 mt-1">
      System prompt, persona, and runtime parameters.
    </p>
  </div>

  <Card>
    <h3 class="text-sm font-semibold mb-3">Soul / system prompt</h3>
    <textarea
      bind:value={soul}
      rows={10}
      placeholder="You are a helpful assistant…"
      class="flex w-full rounded-md border border-zinc-700 bg-zinc-900 px-3 py-2 text-sm font-mono"
    ></textarea>
    <p class="mt-2 text-xs text-zinc-500">
      Markdown supported. This is the primary persona; it takes precedence when
      set.
    </p>
  </Card>

  <Card>
    <h3 class="text-sm font-semibold mb-3">Identity</h3>
    <textarea
      bind:value={identity}
      rows={6}
      placeholder="Name, voice, style…"
      class="flex w-full rounded-md border border-zinc-700 bg-zinc-900 px-3 py-2 text-sm"
    ></textarea>
  </Card>

  <Card>
    <h3 class="text-sm font-semibold mb-3">Runtime</h3>
    <div class="grid grid-cols-2 gap-3">
      <div>
        <label for="cu-t" class="text-xs text-zinc-400">Temperature</label>
        <input
          id="cu-t"
          type="number"
          min="0"
          max="2"
          step="0.05"
          bind:value={temperature}
          class="flex h-9 w-full rounded-md border border-zinc-700 bg-zinc-900 px-3 text-sm"
        />
      </div>
      <div>
        <label for="cu-mt" class="text-xs text-zinc-400">Max tokens</label>
        <input
          id="cu-mt"
          type="number"
          min="64"
          max="200000"
          bind:value={maxTokens}
          class="flex h-9 w-full rounded-md border border-zinc-700 bg-zinc-900 px-3 text-sm"
        />
      </div>
      <div>
        <label for="cu-mi" class="text-xs text-zinc-400"
          >Max tool iterations</label
        >
        <input
          id="cu-mi"
          type="number"
          min="1"
          max="32"
          bind:value={maxIter}
          class="flex h-9 w-full rounded-md border border-zinc-700 bg-zinc-900 px-3 text-sm"
        />
      </div>
      <div>
        <label for="cu-th" class="text-xs text-zinc-400">Thinking mode</label>
        <input
          id="cu-th"
          bind:value={thinking}
          placeholder="auto / enabled / disabled"
          class="flex h-9 w-full rounded-md border border-zinc-700 bg-zinc-900 px-3 text-sm"
        />
      </div>
    </div>
  </Card>

  <div class="flex items-center gap-3">
    <Button onclick={save} disabled={saving}
      >{saving ? "Saving…" : "Save"}</Button
    >
    {#if msg}<span class="text-xs text-zinc-400">{msg}</span>{/if}
  </div>
</div>
