<script lang="ts">
  // Files — agent workspace file browser. Mirrors
  //  (the
  // workspace file viewer). Lists the files in the agent's
  // workspace and lets the user view + edit + delete each one.

  import { onMount } from 'svelte';
  import { listAgentFiles, getAgentFile, putAgentFile, deleteAgentFile, type AgentFileEntry } from '$lib/api';
  import Card from '$lib/components/ui/Card.svelte';
  import Button from '$lib/components/ui/Button.svelte';

  let { params }: { params: { id: string } } = $props();

  let files = $state<AgentFileEntry[]>([]);
  let content = $state('');
  let original = $state('');
  let selected = $state('');
  let loading = $state(false);
  let saving = $state(false);
  let error = $state('');
  let message = $state('');

  async function refresh() {
    try {
      const r = await listAgentFiles(params.id);
      files = r.files ?? [];
      if (files.length > 0 && !selected) selected = files[0].filename;
    } catch (e) {
      error = (e as Error).message || 'failed to load';
    }
  }

  async function loadFile(name: string) {
    if (!name) return;
    try {
      const r = await getAgentFile(params.id, name);
      content = r.content || '';
      original = content;
    } catch (e) {
      error = (e as Error).message || 'failed to load file';
    }
  }

  $effect(() => {
    if (selected) void loadFile(selected);
  });

  async function save() {
    if (!selected) return;
    saving = true;
    try {
      await putAgentFile(params.id, selected, content);
      original = content;
      message = 'Saved.';
      setTimeout(() => (message = ''), 1500);
    } catch (e) {
      error = (e as Error).message || 'save failed';
    } finally {
      saving = false;
    }
  }

  async function remove(name: string) {
    if (!confirm(`Delete file ${name}?`)) return;
    try {
      await deleteAgentFile(params.id, name);
      if (selected === name) {
        selected = '';
        content = '';
      }
      await refresh();
    } catch (e) {
      error = (e as Error).message || 'delete failed';
    }
  }

  onMount(refresh);
</script>

<div class="p-6 max-w-6xl mx-auto space-y-4">
  <div>
    <h2 class="text-2xl font-semibold tracking-tight">Files · {params.id}</h2>
    <p class="text-sm text-zinc-400 mt-1">Workspace files (SOUL.md, IDENTITY.md, etc.) that the agent reads on every turn.</p>
  </div>

  {#if error}
    <p class="text-sm text-red-400">{error}</p>
  {/if}

  <Card class="p-0 overflow-hidden">
    <div class="grid grid-cols-[14rem_1fr] h-[60vh]">
      <aside class="border-r border-zinc-800 p-2 space-y-1 overflow-y-auto">
        {#if files.length === 0}
          <p class="text-xs text-zinc-500 px-2">No files yet.</p>
        {/if}
        {#each files as f (f.filename)}
          <div class="group flex items-center gap-1">
            <button
              class="flex-1 text-left text-sm px-2 py-1 rounded {selected === f.filename ? 'bg-zinc-800' : 'hover:bg-zinc-800/60'}"
              onclick={() => (selected = f.filename)}
            >
              {f.filename}
            </button>
            <button
              class="opacity-0 group-hover:opacity-100 text-zinc-500 hover:text-red-400 text-xs px-1"
              onclick={() => remove(f.filename)}
              title="Delete file"
            >×</button>
          </div>
        {/each}
      </aside>
      <div class="flex flex-col">
        <textarea
          bind:value={content}
          class="flex-1 bg-zinc-900 p-4 text-xs font-mono whitespace-pre resize-none outline-none"
        ></textarea>
        <div class="flex items-center gap-3 border-t border-zinc-800 px-3 py-2">
          <Button size="sm" onclick={save} disabled={saving || content === original}>
            {saving ? 'Saving…' : 'Save'}
          </Button>
          {#if content !== original}<span class="text-xs text-amber-400">unsaved changes</span>{/if}
          {#if message}<span class="text-xs text-zinc-400">{message}</span>{/if}
        </div>
      </div>
    </div>
  </Card>
</div>
