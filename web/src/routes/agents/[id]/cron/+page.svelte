<script lang="ts">
  // Cron scheduler for the active agent. Mirrors
  //
  // (the W1 path is `cron/` in cleanclaw).
  import { onMount } from "svelte";
  import { listCron, createCron, deleteCron, type CronJobInfo } from "$lib/api";
  import { Card } from "$lib/components/ui/card/index.js";
  import { Button } from "$lib/components/ui/button/index.js";

  let { params }: { params: { id: string } } = $props();

  let jobs = $state<CronJobInfo[]>([]);
  let loading = $state(true);
  let name = $state("");
  let type = $state("interval");
  let schedule = $state("5m");
  let message = $state("");
  let channel = $state("");
  let chatId = $state("admin");
  let creating = $state(false);
  let error = $state("");

  async function refresh() {
    try {
      const r = await listCron(params.id);
      jobs = r.jobs ?? [];
    } catch (e) {
      error = (e as Error).message;
    } finally {
      loading = false;
    }
  }

  async function create(e: Event) {
    e.preventDefault();
    creating = true;
    error = "";
    try {
      await createCron(params.id, {
        name,
        type,
        schedule,
        message: message,
        channel,
        chat_id: chatId,
      } as any);
      name = "";
      message = "";
      await refresh();
    } catch (e) {
      error = (e as Error).message || "create failed";
    } finally {
      creating = false;
    }
  }

  async function remove(id: string) {
    if (!confirm("Delete this scheduled job?")) return;
    await deleteCron(params.id, id);
    await refresh();
  }

  onMount(refresh);
</script>

<div class="p-6 max-w-4xl mx-auto space-y-4">
  <div>
    <h2 class="text-2xl font-semibold tracking-tight">
      Scheduler · {params.id}
    </h2>
    <p class="text-sm text-zinc-400 mt-1">Trigger the agent on a schedule.</p>
  </div>

  <Card>
    <h3 class="text-sm font-semibold mb-3">New job</h3>
    <form onsubmit={create} class="grid grid-cols-2 gap-3">
      <div>
        <label for="cj-n" class="text-xs text-zinc-400">Name</label>
        <input
          id="cj-n"
          bind:value={name}
          required
          class="h-9 w-full bg-zinc-900 border border-zinc-700 rounded px-3 text-sm"
        />
      </div>
      <div>
        <label for="cj-t" class="text-xs text-zinc-400">Type</label>
        <select
          id="cj-t"
          bind:value={type}
          class="h-9 w-full bg-zinc-900 border border-zinc-700 rounded px-2 text-sm"
        >
          <option value="interval">interval</option>
          <option value="cron">cron</option>
          <option value="once">once</option>
        </select>
      </div>
      <div>
        <label for="cj-s" class="text-xs text-zinc-400">Schedule</label>
        <input
          id="cj-s"
          bind:value={schedule}
          placeholder="5m / 0 9 * * * / ISO"
          required
          class="h-9 w-full bg-zinc-900 border border-zinc-700 rounded px-3 text-sm"
        />
      </div>
      <div>
        <label for="cj-c" class="text-xs text-zinc-400">Channel</label>
        <input
          id="cj-c"
          bind:value={channel}
          placeholder="web / telegram"
          class="h-9 w-full bg-zinc-900 border border-zinc-700 rounded px-3 text-sm"
        />
      </div>
      <div class="col-span-2">
        <label for="cj-m" class="text-xs text-zinc-400">Message (prompt)</label>
        <input
          id="cj-m"
          bind:value={message}
          required
          class="h-9 w-full bg-zinc-900 border border-zinc-700 rounded px-3 text-sm"
        />
      </div>
      {#if error}<p class="col-span-2 text-sm text-red-400">{error}</p>{/if}
      <div class="col-span-2">
        <Button type="submit" disabled={creating}
          >{creating ? "Creating…" : "Schedule"}</Button
        >
      </div>
    </form>
  </Card>

  <Card>
    <h3 class="text-sm font-semibold mb-3">Scheduled jobs</h3>
    {#if loading}
      <p class="text-sm text-zinc-500">Loading…</p>
    {:else if jobs.length === 0}
      <p class="text-sm text-zinc-500">No scheduled jobs.</p>
    {:else}
      <table class="w-full text-sm">
        <thead>
          <tr class="text-left text-zinc-500 border-b border-zinc-800">
            <th class="py-2">Name</th>
            <th class="py-2">Type</th>
            <th class="py-2">Schedule</th>
            <th class="py-2">Message</th>
            <th class="py-2">Channel</th>
            <th class="py-2"></th>
          </tr>
        </thead>
        <tbody>
          {#each jobs as j (j.id)}
            <tr class="border-b border-zinc-800/50">
              <td class="py-2">{j.name}</td>
              <td class="py-2">{j.type}</td>
              <td class="py-2 font-mono text-xs">{j.schedule}</td>
              <td class="py-2 text-xs text-zinc-400 truncate max-w-xs"
                >{j.message}</td
              >
              <td class="py-2 text-xs"
                >{j.channel || "—"} / {j.chat_id || "—"}</td
              >
              <td class="py-2 text-right">
                <button
                  class="text-xs text-red-400 hover:underline"
                  onclick={() => remove(j.id)}>Delete</button
                >
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </Card>
</div>
