<script lang="ts">
  // NavProjectsList — collapsible list of projects (per-user, per-
  // agent) with the sessions that belong to each project nested
  // underneath.
  // Clicking a project title navigates to the project's chat
  // view; sessions inside expand/collapse on click.

  import { goto } from "$app/navigation";
  import { createProject, deleteProject } from "$lib/api";
  import type { SessionInfo, ProjectInfo } from "$lib/api";

  let {
    agentId,
    projects,
    sessions,
    onChanged,
  }: {
    agentId: string;
    projects: ProjectInfo[];
    sessions: SessionInfo[];
    onChanged?: () => void;
  } = $props();

  let expanded = $state<Record<string, boolean>>({});
  let creating = $state(false);
  let newName = $state("");

  function toggle(id: string) {
    expanded[id] = !expanded[id];
  }

  function projectSessions(p: ProjectInfo): SessionInfo[] {
    return sessions.filter((s) => s.project_id === p.id);
  }

  function openProject(p: ProjectInfo) {
    void goto(`/agents/${agentId}/project/${encodeURIComponent(p.id)}/`);
  }

  function openSession(s: SessionInfo) {
    void goto(`/agents/${agentId}/chat/?session=${encodeURIComponent(s.key)}`);
  }

  async function create(e: Event) {
    e.preventDefault();
    if (!newName.trim()) return;
    try {
      const r = await createProject(agentId, { name: newName.trim() });
      newName = "";
      creating = false;
      onChanged?.();
    } catch (err) {
      alert((err as Error).message || "create failed");
    }
  }

  async function removeProject(p: ProjectInfo, e: Event) {
    e.stopPropagation();
    if (!confirm(`Delete project ${p.name}?`)) return;
    try {
      await deleteProject(agentId, p.id);
      onChanged?.();
    } catch (err) {
      alert((err as Error).message || "delete failed");
    }
  }
</script>

<div>
  <div
    class="px-2 py-1 text-[10px] uppercase tracking-wider text-zinc-500 flex items-center justify-between"
  >
    <span>Projects</span>
    <button
      type="button"
      class="text-zinc-500 hover:text-zinc-300 normal-case tracking-normal"
      title="New project"
      onclick={() => (creating = true)}>+</button
    >
  </div>
  {#if creating}
    <form onsubmit={create} class="px-2 mb-1 flex gap-1">
      <input
        type="text"
        bind:value={newName}
        placeholder="project name"
        class="flex-1 h-7 bg-zinc-900 border border-zinc-700 rounded px-2 text-xs"
      />
      <button type="submit" class="text-xs text-violet-300">Save</button>
      <button
        type="button"
        class="text-xs text-zinc-500"
        onclick={() => (creating = false)}>×</button
      >
    </form>
  {/if}
  <ul class="space-y-0.5">
    {#each projects as p (p.id)}
      {@const ps = projectSessions(p)}
      {@const isOpen = expanded[p.id]}
      <li>
        <div class="group flex items-center gap-1">
          <button
            type="button"
            class="px-1 text-zinc-500 hover:text-zinc-300"
            onclick={() => toggle(p.id)}
            title={isOpen ? "Collapse" : "Expand"}>{isOpen ? "▾" : "▸"}</button
          >
          <button
            type="button"
            class="flex-1 text-left px-2 py-1 rounded text-sm hover:bg-zinc-800/60 truncate"
            onclick={() => openProject(p)}
          >
            <div class="truncate text-zinc-200">{p.name}</div>
            <div class="truncate text-[10px] text-zinc-500 font-mono">
              {ps.length} session{ps.length === 1 ? "" : "s"}
            </div>
          </button>
          <button
            type="button"
            class="opacity-0 group-hover:opacity-100 text-zinc-500 hover:text-red-400 px-1"
            title="Delete project"
            onclick={(e) => removeProject(p, e)}>×</button
          >
        </div>
        {#if isOpen && ps.length > 0}
          <ul class="ml-4 mt-0.5 space-y-0.5 border-l border-zinc-800 pl-2">
            {#each ps as s (s.key)}
              <li>
                <button
                  type="button"
                  class="w-full text-left px-2 py-0.5 rounded text-xs hover:bg-zinc-800/60 truncate"
                  onclick={() => openSession(s)}
                >
                  <div class="truncate text-zinc-300">{s.title || s.key}</div>
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      </li>
    {/each}
    {#if projects.length === 0 && !creating}
      <li class="px-2 py-1 text-xs text-zinc-600">No projects.</li>
    {/if}
  </ul>
</div>
