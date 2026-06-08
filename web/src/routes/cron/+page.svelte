<script lang="ts">
  import { onMount } from 'svelte';
  import { listAgents, listCron, createCron, deleteCron, type AgentInfo, type CronJobInfo } from '$lib/api';
  import Card from '$lib/components/ui/Card.svelte';
  import Button from '$lib/components/ui/Button.svelte';

  let agents = $state<AgentInfo[]>([]);
  let selected = $state('');
  let jobs = $state<CronJobInfo[]>([]);
  let name = $state('');
  let type = $state('interval');
  let schedule = $state('5m');
  let message = $state('');
  let channel = $state('web');
  let chatId = $state('admin');
  let creating = $state(false);
  let error = $state('');

  async function refresh() {
    if (!selected) return;
    try {
      const r = await listCron(selected);
      jobs = r.jobs ?? [];
    } catch (e) { error = (e as Error).message; }
  }

  async function create(e: Event) {
    e.preventDefault();
    if (!selected) return;
    creating = true; error = '';
    try {
      await createCron(selected, { name, type, schedule, message, channel, chat_id: chatId } as any);
      name = ''; message = '';
      await refresh();
    } catch (e) {
      error = (e as Error).message || 'create failed';
    } finally { creating = false; }
  }

  async function rm(id: string) {
    if (!confirm('Delete this scheduled job?')) return;
    try {
      await deleteCron(selected, id);
      await refresh();
    } catch {}
  }

  onMount(async () => {
    try {
      const r = await listAgents();
      agents = r.agents ?? [];
      if (agents.length > 0) selected = agents[0].id;
      await refresh();
    } catch {}
  });

  $effect(() => { if (selected) void refresh(); });
</script>

<div class="p-6 max-w-4xl mx-auto space-y-4">
  <div>
    <h2 class="text-2xl font-semibold tracking-tight">Cron scheduler</h2>
    <p class="text-sm text-zinc-400 mt-1">Trigger agents on a schedule.</p>
  </div>

  <Card>
    <div class="flex items-center gap-3 mb-3">
      <label for="cr-a" class="text-xs text-zinc-400">Agent</label>
      <select id="cr-a" bind:value={selected}
        class="flex-1 h-9 bg-zinc-900 border border-zinc-700 rounded px-2 text-sm">
        {#each agents as a (a.id)}
          <option value={a.id}>{a.name || a.id}</option>
        {/each}
      </select>
    </div>
    <form onsubmit={create} class="grid grid-cols-2 gap-3">
      <div>
        <label for="cr-n" class="text-xs text-zinc-400">Name</label>
        <input id="cr-n" bind:value={name} required class="h-9 w-full bg-zinc-900 border border-zinc-700 rounded px-3 text-sm" />
      </div>
      <div>
        <label for="cr-t" class="text-xs text-zinc-400">Type</label>
        <select id="cr-t" bind:value={type} class="h-9 w-full bg-zinc-900 border border-zinc-700 rounded px-2 text-sm">
          <option value="interval">interval</option>
          <option value="cron">cron</option>
          <option value="once">once</option>
        </select>
      </div>
      <div>
        <label for="cr-s" class="text-xs text-zinc-400">Schedule</label>
        <input id="cr-s" bind:value={schedule} placeholder="5m / 0 9 * * * / ISO" required class="h-9 w-full bg-zinc-900 border border-zinc-700 rounded px-3 text-sm" />
      </div>
      <div>
        <label for="cr-c" class="text-xs text-zinc-400">Channel</label>
        <input id="cr-c" bind:value={channel} class="h-9 w-full bg-zinc-900 border border-zinc-700 rounded px-3 text-sm" />
      </div>
      <div class="col-span-2">
        <label for="cr-m" class="text-xs text-zinc-400">Message (prompt)</label>
        <input id="cr-m" bind:value={message} required class="h-9 w-full bg-zinc-900 border border-zinc-700 rounded px-3 text-sm" />
      </div>
      {#if error}<p class="col-span-2 text-sm text-red-400">{error}</p>{/if}
      <div class="col-span-2">
        <Button type="submit" disabled={creating || !selected}>{creating ? 'Creating…' : 'Schedule'}</Button>
      </div>
    </form>
  </Card>

  <Card>
    <h3 class="text-sm font-semibold mb-3">Scheduled jobs</h3>
    {#if jobs.length === 0}
      <p class="text-sm text-zinc-500">No scheduled jobs.</p>
    {:else}
      <table class="w-full text-sm">
        <thead>
          <tr class="text-left text-zinc-500 border-b border-zinc-800">
            <th class="py-2">Name</th>
            <th class="py-2">Type</th>
            <th class="py-2">Schedule</th>
            <th class="py-2">Message</th>
            <th class="py-2"></th>
          </tr>
        </thead>
        <tbody>
          {#each jobs as j (j.id)}
            <tr class="border-b border-zinc-800/50">
              <td class="py-2">{j.name}</td>
              <td class="py-2">{j.type}</td>
              <td class="py-2 font-mono text-xs">{j.schedule}</td>
              <td class="py-2 text-xs text-zinc-400 truncate max-w-xs">{j.message}</td>
              <td class="py-2 text-right">
                <button class="text-xs text-red-400 hover:underline" onclick={() => rm(j.id)}>Delete</button>
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </Card>
</div>
