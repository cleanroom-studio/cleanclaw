<script lang="ts">
  // Sessions — agent-scoped session list. Mirrors
  // .
  // In CleanClaw the page just hands the URL to ChatScreen; we
  // surface a full editor too so the operator can rename /
  // delete / drill into history directly.

  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import {
    listChatSessions,
    renameChatSession,
    deleteChatSession,
    type SessionInfo,
  } from "$lib/api";
  import Card from "$lib/components/ui/Card.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import Badge from "$lib/components/ui/Badge.svelte";

  let { params }: { params: { id: string } } = $props();

  let sessions = $state<SessionInfo[]>([]);
  let loading = $state(true);
  let editing = $state<string | null>(null);
  let titleDraft = $state("");

  async function refresh() {
    loading = true;
    try {
      const r = await listChatSessions(params.id);
      sessions = (r.sessions ?? [])
        .slice()
        .sort(
          (a, b) =>
            new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime(),
        );
    } catch {
    } finally {
      loading = false;
    }
  }

  function open(key: string) {
    void goto(`/agents/${params.id}/chat/?session=${encodeURIComponent(key)}`);
  }

  function startEdit(s: SessionInfo) {
    editing = s.key;
    titleDraft = s.title || "";
  }

  async function saveTitle() {
    if (!editing) return;
    try {
      await renameChatSession(params.id, editing, titleDraft.trim());
      editing = null;
      await refresh();
    } catch (e) {
      alert((e as Error).message);
    }
  }

  async function remove(key: string) {
    if (!confirm(`Delete session ${key}?`)) return;
    await deleteChatSession(params.id, key);
    await refresh();
  }

  onMount(refresh);
</script>

<div class="p-6 max-w-4xl mx-auto space-y-4">
  <div class="flex items-center gap-3">
    <h2 class="text-2xl font-semibold tracking-tight">Sessions</h2>
    <Badge>{params.id}</Badge>
    <Button size="sm" variant="outline" class="ml-auto" onclick={refresh}
      >Refresh</Button
    >
  </div>

  <Card>
    {#if loading}
      <p class="text-sm text-zinc-500">Loading…</p>
    {:else if sessions.length === 0}
      <p class="text-sm text-zinc-500">
        No sessions yet. Start a chat to create one.
      </p>
    {:else}
      <table class="w-full text-sm">
        <thead>
          <tr class="text-left text-zinc-500 border-b border-zinc-800">
            <th class="py-2">Key</th>
            <th class="py-2">Title</th>
            <th class="py-2">Channel</th>
            <th class="py-2">Messages</th>
            <th class="py-2">Updated</th>
            <th class="py-2"></th>
          </tr>
        </thead>
        <tbody>
          {#each sessions as s (s.key)}
            <tr class="border-b border-zinc-800/50">
              <td class="py-2 font-mono text-xs">{s.key}</td>
              <td class="py-2">
                {#if editing === s.key}
                  <input
                    bind:value={titleDraft}
                    class="h-7 bg-zinc-900 border border-zinc-700 rounded px-2 text-sm"
                  />
                {:else}
                  {s.title || "—"}
                {/if}
              </td>
              <td class="py-2">{s.channel || "web"}</td>
              <td class="py-2">{s.message_count}</td>
              <td class="py-2 text-zinc-500 text-xs"
                >{new Date(s.updated_at).toLocaleString()}</td
              >
              <td class="py-2 text-right space-x-2">
                {#if editing === s.key}
                  <button class="text-xs text-violet-300" onclick={saveTitle}
                    >Save</button
                  >
                  <button
                    class="text-xs text-zinc-500"
                    onclick={() => (editing = null)}>×</button
                  >
                {:else}
                  <button
                    class="text-xs text-violet-300 hover:underline"
                    onclick={() => open(s.key)}>Open</button
                  >
                  <button
                    class="text-xs text-zinc-500 hover:underline"
                    onclick={() => startEdit(s)}>Rename</button
                  >
                  <button
                    class="text-xs text-red-400 hover:underline"
                    onclick={() => remove(s.key)}>Delete</button
                  >
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </Card>
</div>
