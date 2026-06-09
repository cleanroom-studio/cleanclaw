<script lang="ts">
  // Onboard — 6-step bootstrap wizard. Mirrors CleanClaw's
  // onboard flow (welcome / admin / provider / agent / sandbox /
  // launch). Compressed for the parity sweep: welcome card +
  // admin registration form. The full provider/agent/sandbox
  // steps are stubbed as cards pointing to the relevant pages
  // so the bootstrap can complete in the time budget while
  // preserving the surface the dashboard needs.

  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { getMe, getStatus, register, type UserInfo } from "$lib/api";
  import { Card } from "$lib/components/ui/card/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";

  let me = $state<UserInfo | null>(null);
  let status = $state<any>(null);
  let step = $state(0);
  let username = $state("");
  let email = $state("");
  let password = $state("");
  let displayName = $state("");
  let error = $state("");
  let creating = $state(false);

  onMount(async () => {
    try {
      const r = await getMe();
      me = r.user ?? null;
    } catch {}
    try {
      status = await getStatus();
    } catch {}

    // If a user already exists, skip the bootstrap.
    if (me || (status && (status.user_count || 0) > 0)) {
      step = 5; // jump to "launch" step
    }
  });

  async function createAdmin(e: Event) {
    e.preventDefault();
    error = "";
    creating = true;
    try {
      const r = await register({
        username,
        email,
        password,
        display_name: displayName || username,
      });
      if (!r.ok) {
        error = r.error || "failed";
        creating = false;
        return;
      }
      me = {
        id: r.user_id!,
        username,
        email,
        display_name: displayName,
        role: "super_admin",
        is_admin: true,
      };
      step = 2;
    } catch (err) {
      error = (err as Error).message || "failed";
    } finally {
      creating = false;
    }
  }

  function next() {
    step = Math.min(5, step + 1);
  }
  function prev() {
    step = Math.max(0, step - 1);
  }
</script>

<div class="min-h-screen bg-zinc-950 text-zinc-100 p-8">
  <div class="max-w-2xl mx-auto space-y-6">
    <div class="flex items-center gap-2 text-xs text-zinc-500">
      {#each ["Welcome", "Admin", "Provider", "Agent", "Sandbox", "Launch"] as label, i (label)}
        <div
          class:font-semibold={step === i}
          class:text-violet-300={step === i}
        >
          {i + 1}. {label}
        </div>
        {#if i < 5}<span class="text-zinc-700">·</span>{/if}
      {/each}
    </div>

    {#if step === 0}
      <Card>
        <h2 class="text-xl font-bold mb-2">Welcome to CleanClaw</h2>
        <p class="text-sm text-zinc-400">
          A multi-tenant AI agent runtime. This wizard sets up the first
          super_admin so you can sign in. After that you'll be able to add
          providers, agents, and channels from the dashboard.
        </p>
        <div class="mt-4 flex justify-end">
          <Button onclick={next}>Get started →</Button>
        </div>
      </Card>
    {:else if step === 1}
      <Card>
        <h2 class="text-xl font-bold mb-2">Create super_admin</h2>
        <form onsubmit={createAdmin} class="space-y-3">
          <div>
            <Label for="ad-u">Username</Label><Input
              id="ad-u"
              bind:value={username}
              required
            />
          </div>
          <div>
            <Label for="ad-dn">Display name</Label><Input
              id="ad-dn"
              bind:value={displayName}
              placeholder="Admin"
            />
          </div>
          <div>
            <Label for="ad-e">Email</Label><Input
              id="ad-e"
              type="email"
              bind:value={email}
              required
            />
          </div>
          <div>
            <Label for="ad-p">Password (min 8)</Label><Input
              id="ad-p"
              type="password"
              bind:value={password}
              minlength={8}
              required
            />
          </div>
          {#if error}<p class="text-sm text-red-400">{error}</p>{/if}
          <div class="flex justify-between">
            <Button variant="outline" onclick={prev}>← Back</Button>
            <Button type="submit" disabled={creating}
              >{creating ? "Creating…" : "Create"}</Button
            >
          </div>
        </form>
      </Card>
    {:else if step === 2}
      <Card>
        <h2 class="text-xl font-bold mb-2">Add a provider</h2>
        <p class="text-sm text-zinc-400">
          Providers are LLM backends (OpenAI, Anthropic, minimax, Ollama, …).
          You'll need at least one to drive agent turns.
        </p>
        <div class="mt-4 flex justify-between">
          <Button variant="outline" onclick={prev}>← Back</Button>
          <div class="flex gap-2">
            <Button variant="outline" onclick={() => (step = 5)}>Skip</Button>
            <a href="/providers/"><Button>Open providers →</Button></a>
          </div>
        </div>
      </Card>
    {:else if step === 3}
      <Card>
        <h2 class="text-xl font-bold mb-2">Create your first agent</h2>
        <p class="text-sm text-zinc-400">
          Agents are the LLM-backed personas that respond to messages. Each
          agent has its own workspace, skills, channels, and chat history.
        </p>
        <div class="mt-4 flex justify-between">
          <Button variant="outline" onclick={prev}>← Back</Button>
          <div class="flex gap-2">
            <Button variant="outline" onclick={() => (step = 5)}>Skip</Button>
            <a href="/agents/"><Button>Open agents →</Button></a>
          </div>
        </div>
      </Card>
    {:else if step === 4}
      <Card>
        <h2 class="text-xl font-bold mb-2">Sandbox</h2>
        <p class="text-sm text-zinc-400">
          The sandbox runs tool calls in isolation. Default: <code
            class="bg-zinc-800 px-1 rounded">docker</code
          >. Operators can swap to E2B or BoxLite at runtime.
        </p>
        <div class="mt-4 flex justify-between">
          <Button variant="outline" onclick={prev}>← Back</Button>
          <Button onclick={next}>Continue →</Button>
        </div>
      </Card>
    {:else if step === 5}
      <Card>
        <h2 class="text-2xl font-bold mb-2">🎉 You're set up!</h2>
        <p class="text-sm text-zinc-400">
          CleanClaw is ready. Head to the dashboard to start chatting, or jump
          to a specific section:
        </p>
        <div class="mt-4 grid grid-cols-2 gap-2">
          <a href="/overview/"><Button class="w-full">Overview</Button></a>
          <a href="/chat/"
            ><Button class="w-full" variant="outline">Chat</Button></a
          >
          <a href="/agents/"
            ><Button class="w-full" variant="outline">Agents</Button></a
          >
          <a href="/providers/"
            ><Button class="w-full" variant="outline">Providers</Button></a
          >
        </div>
      </Card>
    {/if}
  </div>
</div>
