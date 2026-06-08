<script lang="ts">
  import { onMount } from "svelte";
  import {
    listSkills,
    installSkill,
    deleteSkill,
    searchSkills,
    type SkillInfo,
    type SkillSearchResult,
  } from "$lib/api";
  import Card from "$lib/components/ui/Card.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import Badge from "$lib/components/ui/Badge.svelte";

  let skills = $state<SkillInfo[]>([]);
  let loading = $state(true);
  let search = $state("");
  let source = $state("skillssh");
  let results = $state<SkillSearchResult[]>([]);
  let message = $state("");
  let error = $state("");

  async function refresh() {
    loading = true;
    try {
      const r = await listSkills();
      skills = r.skills ?? [];
    } catch (e) {
      error = (e as Error).message;
    } finally {
      loading = false;
    }
  }

  async function doSearch() {
    if (!search.trim()) return;
    try {
      const r = await searchSkills(search.trim(), source);
      results = r.results ?? [];
    } catch {
      results = [];
    }
  }

  async function install(name: string) {
    try {
      const r = await installSkill({ source, spec: name });
      if (!r.ok) {
        message = r.error || "install failed";
        return;
      }
      message = `installed ${r.name}`;
      await refresh();
    } catch (e) {
      message = (e as Error).message || "install failed";
    }
  }

  async function remove(name: string) {
    if (!confirm(`Remove skill ${name}?`)) return;
    try {
      await deleteSkill(name);
      await refresh();
    } catch (e) {
      message = (e as Error).message || "uninstall failed";
    }
  }

  onMount(refresh);
</script>

<div class="p-6 max-w-5xl mx-auto space-y-4">
  <div>
    <h2 class="text-2xl font-semibold tracking-tight">Skills</h2>
    <p class="text-sm text-zinc-400 mt-1">
      Global skills available to every agent. Per-agent overrides go on each
      agent's Skills tab.
    </p>
  </div>

  {#if error}<p class="text-sm text-red-400">{error}</p>{/if}

  <Card>
    <div class="flex items-center justify-between mb-3">
      <h3 class="text-sm font-semibold">Installed (global)</h3>
      <Button size="sm" variant="outline" onclick={refresh}>Refresh</Button>
    </div>
    {#if loading}
      <p class="text-sm text-zinc-500">Loading…</p>
    {:else if skills.length === 0}
      <p class="text-sm text-zinc-500">No skills installed yet.</p>
    {:else}
      <table class="w-full text-sm">
        <thead>
          <tr class="text-left text-zinc-500 border-b border-zinc-800">
            <th class="py-2">Name</th>
            <th class="py-2">Description</th>
            <th class="py-2">Layer</th>
            <th class="py-2"></th>
          </tr>
        </thead>
        <tbody>
          {#each skills as s (s.name)}
            <tr class="border-b border-zinc-800/50">
              <td class="py-2 font-medium">{s.name}</td>
              <td class="py-2 text-zinc-500 truncate max-w-md"
                >{s.description}</td
              >
              <td class="py-2"
                ><Badge variant="outline">{s.layer || "global"}</Badge></td
              >
              <td class="py-2 text-right">
                <button
                  class="text-xs text-red-400 hover:underline"
                  onclick={() => remove(s.name)}>Remove</button
                >
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </Card>

  <Card>
    <h3 class="text-sm font-semibold mb-3">Install from registry</h3>
    <div class="flex items-center gap-2">
      <select
        bind:value={source}
        class="h-9 bg-zinc-900 border border-zinc-700 rounded px-2 text-sm"
      >
        <option value="skillssh">skills.sh</option>
        <option value="clawhub">ClawHub</option>
      </select>
      <input
        type="text"
        bind:value={search}
        onkeydown={(e: KeyboardEvent) => {
          if (e.key === "Enter") doSearch();
        }}
        placeholder="search…"
        class="flex-1 h-9 bg-zinc-900 border border-zinc-700 rounded px-3 text-sm"
      />
      <Button onclick={doSearch}>Search</Button>
    </div>
    {#if results.length > 0}
      <ul class="mt-3 divide-y divide-zinc-800">
        {#each results as r (r.name)}
          <li class="py-2 flex items-center gap-3">
            <div class="flex-1">
              <div class="text-sm font-medium">{r.name}</div>
              {#if r.description}<div class="text-xs text-zinc-500">
                  {r.description}
                </div>{/if}
            </div>
            <Button size="sm" variant="outline" onclick={() => install(r.name)}
              >Install</Button
            >
          </li>
        {/each}
      </ul>
    {/if}
    {#if message}<p class="text-xs text-zinc-400 mt-2">{message}</p>{/if}
  </Card>
</div>
