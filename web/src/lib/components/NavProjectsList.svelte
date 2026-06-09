<script lang="ts">
  // NavProjectsList — collapsible per-agent projects with their
  // sessions nested underneath. Each project is rendered as a
  // shadcn `Collapsible` so the chevron rotates with state, and
  // sessions use the `Sidebar.MenuSub` style.

  import { goto } from "$app/navigation";
  import * as Sidebar from "$lib/components/ui/sidebar/index.js";
  import * as Collapsible from "$lib/components/ui/collapsible/index.js";
  import * as DropdownMenu from "$lib/components/ui/dropdown-menu/index.js";
  import { createProject, deleteProject } from "$lib/api";
  import type { SessionInfo, ProjectInfo } from "$lib/api";
  import Folder from "@lucide/svelte/icons/folder";
  import FolderOpen from "@lucide/svelte/icons/folder-open";
  import ChevronRight from "@lucide/svelte/icons/chevron-right";
  import MoreHorizontal from "@lucide/svelte/icons/more-horizontal";
  import Trash2 from "@lucide/svelte/icons/trash-2";
  import Plus from "@lucide/svelte/icons/plus";
  import Check from "@lucide/svelte/icons/check";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Button } from "$lib/components/ui/button/index.js";

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

  let open: Record<string, boolean> = $state({});
  let creating = $state(false);
  let newName = $state("");

  function toggle(id: string) {
    open[id] = !open[id];
  }

  function projectSessions(p: ProjectInfo): SessionInfo[] {
    return sessions.filter((s) => s.project_id === p.id);
  }

  function openProject(p: ProjectInfo) {
    void goto(`/agents/${agentId}/project/${encodeURIComponent(p.id)}/`);
  }

  function openSession(s: SessionInfo) {
    void goto(
      `/agents/${agentId}/chat/?session=${encodeURIComponent(s.key)}`,
    );
  }

  async function create(e: Event) {
    e.preventDefault();
    if (!newName.trim()) return;
    try {
      await createProject(agentId, { name: newName.trim() });
      newName = "";
      creating = false;
      onChanged?.();
    } catch (err) {
      alert((err as Error).message || "create failed");
    }
  }

  async function removeProject(p: ProjectInfo) {
    if (!confirm(`Delete project ${p.name}?`)) return;
    try {
      await deleteProject(agentId, p.id);
      onChanged?.();
    } catch (err) {
      alert((err as Error).message || "delete failed");
    }
  }
</script>

<Sidebar.Group>
  <Sidebar.GroupLabel>
    Projects
    <Sidebar.GroupAction title="New project" onclick={() => (creating = true)}>
      <Plus class="size-4" />
      <span class="sr-only">New project</span>
    </Sidebar.GroupAction>
  </Sidebar.GroupLabel>
  <Sidebar.GroupContent>
    {#if creating}
      <form
        onsubmit={create}
        class="flex items-center gap-1 px-2 pb-2 group/project"
      >
        <Input
          type="text"
          placeholder="project name"
          bind:value={newName}
          class="h-7 text-xs"
        />
        <Button
          type="submit"
          size="icon-sm"
          variant="ghost"
          aria-label="Create"
        >
          <Check class="size-4" />
        </Button>
        <Button
          type="button"
          size="icon-sm"
          variant="ghost"
          aria-label="Cancel"
          onclick={() => (creating = false)}
        >
          <span class="text-base leading-none">×</span>
        </Button>
      </form>
    {/if}
    <Sidebar.Menu>
      {#each projects as p (p.id)}
        {@const ps = projectSessions(p)}
        {@const isOpen = open[p.id]}
        <Collapsible.Root bind:open={() => open[p.id], (v) => (open[p.id] = v)} class="group/collapsible">
          <Sidebar.MenuItem>
            <Collapsible.Trigger>
              {#snippet child({ props })}
                <Sidebar.MenuButton {...props}>
                  {#if isOpen}
                    <FolderOpen />
                  {:else}
                    <Folder />
                  {/if}
                  <span>{p.name}</span>
                  {#if ps.length > 0}
                    <ChevronRight
                      class="ms-auto size-4 transition-transform group-data-[state=open]/collapsible:rotate-90"
                    />
                  {/if}
                </Sidebar.MenuButton>
              {/snippet}
            </Collapsible.Trigger>
            <DropdownMenu.Root>
              <DropdownMenu.Trigger>
                {#snippet child({ props })}
                  <Sidebar.MenuAction
                    {...props}
                    showOnHover
                    class="peer-data-[state=open]/menu-button:opacity-100"
                  >
                    <MoreHorizontal />
                    <span class="sr-only">Project actions</span>
                  </Sidebar.MenuAction>
                {/snippet}
              </DropdownMenu.Trigger>
              <DropdownMenu.Content side="right" align="start">
                <DropdownMenu.Item
                  variant="destructive"
                  onSelect={() => removeProject(p)}
                >
                  <Trash2 class="size-4" />
                  <span>Delete project</span>
                </DropdownMenu.Item>
              </DropdownMenu.Content>
            </DropdownMenu.Root>
            {#if ps.length > 0}
              <Collapsible.Content>
                <Sidebar.MenuSub>
                  {#each ps as s (s.key)}
                    <Sidebar.MenuSubItem>
                      <Sidebar.MenuSubButton>
                        {#snippet child({ props })}
                          <a
                            href={`/agents/${agentId}/chat/?session=${encodeURIComponent(s.key)}`}
                            {...props}
                          >
                            <span>{s.title || s.key}</span>
                          </a>
                        {/snippet}
                      </Sidebar.MenuSubButton>
                    </Sidebar.MenuSubItem>
                  {/each}
                </Sidebar.MenuSub>
              </Collapsible.Content>
            {/if}
          </Sidebar.MenuItem>
        </Collapsible.Root>
      {/each}
      {#if projects.length === 0 && !creating}
        <Sidebar.MenuItem>
          <span class="px-2 py-1 text-xs text-muted-foreground">
            No projects.
          </span>
        </Sidebar.MenuItem>
      {/if}
    </Sidebar.Menu>
  </Sidebar.GroupContent>
</Sidebar.Group>
