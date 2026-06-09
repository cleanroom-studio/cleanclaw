<script lang="ts">
  import { onMount } from "svelte";
  import AppShell from "$lib/components/AppShell.svelte";
  import { Card } from "$lib/components/ui/card/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Textarea } from "$lib/components/ui/textarea/index.js";
  import { Label } from "$lib/components/ui/label/index.js";
  import { getAgent, updateAgent, type AgentDetail } from "$lib/api";

  let { params }: { params: { id: string } } = $props();
  let agent = $state<AgentDetail | null>(null);
  let soul = $state("");
  let identity = $state("");
  let systemPrompt = $state("");
  let saving = $state(false);
  let message = $state("");

  async function load() {
    try {
      const r = await getAgent(params.id);
      agent = r.agent;
      soul = r.agent.soul ?? "";
      identity = r.agent.identity ?? "";
      systemPrompt = r.agent.system_prompt ?? "";
    } catch (e: any) {
      message = e?.message || "failed to load";
    }
  }

  async function save(e: Event) {
    e.preventDefault();
    saving = true;
    message = "";
    try {
      await updateAgent(params.id, {
        soul,
        identity,
        system_prompt: systemPrompt,
      } as any);
      message = "saved";
    } catch (e: any) {
      message = e?.message || "save failed";
    } finally {
      saving = false;
    }
  }

  onMount(load);
</script>

<AppShell>
  <div class="space-y-4">
    <h1 class="text-2xl font-bold">Context</h1>
    <Card>
      <h2 class="text-base font-semibold mb-1">Agent identity & context</h2>
      <p class="text-sm text-muted-foreground mb-4">
        The system prompt, identity, and "soul" are the three parts of the
        agent's standing context. Edits take effect on the next turn.
      </p>
      <form onsubmit={save} class="space-y-4">
        <div class="space-y-1.5">
          <Label for="soul">Soul</Label>
          <Textarea
            id="soul"
            bind:value={soul}
            rows={6}
            placeholder="Personality, values, tone…"
          />
        </div>
        <div class="space-y-1.5">
          <Label for="identity">Identity</Label>
          <Textarea
            id="identity"
            bind:value={identity}
            rows={4}
            placeholder="Name, role, expertise…"
          />
        </div>
        <div class="space-y-1.5">
          <Label for="sp">System prompt</Label>
          <Textarea
            id="sp"
            bind:value={systemPrompt}
            rows={8}
            placeholder="You are…"
          />
        </div>
        <div class="flex items-center gap-3">
          <Button type="submit" disabled={saving}
            >{saving ? "Saving…" : "Save"}</Button
          >
          {#if message}
            <span class="text-sm text-muted-foreground">{message}</span>
          {/if}
        </div>
      </form>
    </Card>
  </div>
</AppShell>
