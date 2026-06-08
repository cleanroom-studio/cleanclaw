<script lang="ts">
  // Skills (agent-scoped) — enable / disable skills for this
  // agent.

  import { onMount } from 'svelte';
  import { listSkills, installSkill, searchSkills, type SkillInfo, type SkillSearchResult } from '$lib/api';
  import Card from '$lib/components/ui/Card.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import Badge from '$lib/components/ui/Badge.svelte';

  let { params }: { params: { id: string } } = $props();

  let skills = $state<SkillInfo[]>([]);
  let loading = $state(true);
  let search = $state('');
  let source = $state('skillssh');
  let results = $state<SkillSearchResult[]>([]);
  let message = $state('');

  async function refresh() {
    loading = true;
    try {
      const r = await listSkills(params.id);
      skills = r.skills ?? [];
    } catch {} finally { loading = false; }
  }

  async function doSearch() {
    if (!search.trim()) return;
    try {
      const r = await searchSkills(search.trim(), source);
      results = r.results ?? [];
    } catch { results = []; }
  }

  async function install(name: string) {
    try {
      const r = await installSkill({ source, spec: name });
      if (!r.ok) {
        message = r.error || 'install failed';
        return;
      }
      message = `installed ${r.name}`;
      await refresh();
    } catch (e) {
      message = (e as Error).message || 'install failed';
    }
  }

  onMount(refresh);
</script>

<div class="p-6 max-w-5xl mx-auto space-y-4">
  <div>
    <h2 class="text-2xl font-semibold tracking-tight">Skills · {params.id}</h2>
    <p class="text-sm text-zinc-400 mt-1">Skills add capabilities (tools, prompts, knowledge packs) the agent can call.</p>
  </div>

  <Card>
    <h3 class="text-sm font-semibold mb-3">Installed</h3>
    {#if loading}
      <p class="text-sm text-zinc-500">Loading…</p>
    {:else if skills.length === 0}
      <p class="text-sm text-zinc-500">No skills installed for this agent.</p>
    {:else}
      <ul class="divide-y divide-zinc-800">
        {#each skills as s (s.name)}
          <li class="py-3 flex items-center gap-3">
            <div class="flex-1">
              <div class="text-sm font-medium">{s.name}</div>
              <div class="text-xs text-zinc-500 truncate max-w-lg">{s.description}</div>
            </div>
            <Badge variant="outline">{s.layer || 'agent'}</Badge>
          </li>
        {/each}
      </ul>
    {/if}
  </Card>

  <Card>
    <h3 class="text-sm font-semibold mb-3">Install from registry</h3>
    <div class="flex items-center gap-2">
      <select bind:value={source}
        class="h-9 bg-zinc-900 border border-zinc-700 rounded px-2 text-sm">
        <option value="skillssh">skills.sh</option>
        <option value="clawhub">ClawHub</option>
      </select>
      <input
        type="text"
        bind:value={search}
        onkeydown={(e: KeyboardEvent) => { if (e.key === 'Enter') doSearch(); }}
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
              {#if r.description}
                <div class="text-xs text-zinc-500">{r.description}</div>
              {/if}
            </div>
            <Button size="sm" variant="outline" onclick={() => install(r.name)}>Install</Button>
          </li>
        {/each}
      </ul>
    {/if}
    {#if message}<p class="mt-2 text-xs text-zinc-400">{message}</p>{/if}
  </Card>
</div>
