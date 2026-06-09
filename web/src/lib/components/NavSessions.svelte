<script lang="ts">
  // NavSessions — flat list of recent chat sessions for the
  // active agent. Sorted by `updated_at` so the most recently
  // active session floats to the top. Each row is a shadcn
  // `Sidebar.MenuButton` with a `MenuAction` hover-revealed
  // dropdown for delete.

  import { goto } from "$app/navigation";
  import * as Sidebar from "$lib/components/ui/sidebar/index.js";
  import * as DropdownMenu from "$lib/components/ui/dropdown-menu/index.js";
  import { deleteChatSession } from "$lib/api";
  import type { SessionInfo } from "$lib/api";
  import MessageSquare from "@lucide/svelte/icons/message-square";
  import MoreHorizontal from "@lucide/svelte/icons/more-horizontal";
  import Trash2 from "@lucide/svelte/icons/trash-2";

  let { agentId, sessions }: { agentId: string; sessions: SessionInfo[] } =
    $props();

  const sorted = $derived(
    [...(sessions || [])].sort(
      (a, b) =>
        new Date(b.updated_at).getTime() - new Date(a.updated_at).getTime(),
    ),
  );

  function open(key: string) {
    void goto(`/agents/${agentId}/chat/?session=${encodeURIComponent(key)}`);
  }

  async function remove(key: string) {
    if (!confirm(`Delete session ${key}?`)) return;
    try {
      await deleteChatSession(agentId, key);
      window.dispatchEvent(
        new CustomEvent("cleanclaw:sessions-changed", { detail: { agentId } }),
      );
    } catch (e) {
      alert((e as Error).message || "delete failed");
    }
  }

  function preview(s: SessionInfo): string {
    if (s.title) return s.title;
    if (s.preview) return s.preview;
    return s.key;
  }
</script>

<Sidebar.Group>
  <Sidebar.GroupLabel>
    Sessions
    <span class="ms-auto text-[10px] font-normal text-muted-foreground">
      {sorted.length}
    </span>
  </Sidebar.GroupLabel>
  <Sidebar.GroupContent>
    <Sidebar.Menu>
      {#each sorted as s (s.key)}
        <Sidebar.MenuItem>
          <Sidebar.MenuButton tooltip={preview(s)}>
            {#snippet child({ props })}
              <a
                href={`/agents/${agentId}/chat/?session=${encodeURIComponent(s.key)}`}
                {...props}
              >
                <MessageSquare />
                <div class="grid flex-1 text-left text-sm leading-tight">
                  <span class="truncate font-medium">{preview(s)}</span>
                  <span class="truncate text-[10px] text-muted-foreground font-mono">
                    {s.key}
                  </span>
                </div>
              </a>
            {/snippet}
          </Sidebar.MenuButton>
          <DropdownMenu.Root>
            <DropdownMenu.Trigger>
              {#snippet child({ props })}
                <Sidebar.MenuAction
                  {...props}
                  showOnHover
                  class="peer-data-[state=open]/menu-button:opacity-100"
                >
                  <MoreHorizontal />
                  <span class="sr-only">Session actions</span>
                </Sidebar.MenuAction>
              {/snippet}
            </DropdownMenu.Trigger>
            <DropdownMenu.Content side="right" align="start">
              <DropdownMenu.Item
                variant="destructive"
                onSelect={() => remove(s.key)}
              >
                <Trash2 class="size-4" />
                <span>Delete session</span>
              </DropdownMenu.Item>
            </DropdownMenu.Content>
          </DropdownMenu.Root>
        </Sidebar.MenuItem>
      {/each}
      {#if sorted.length === 0}
        <Sidebar.MenuItem>
          <span class="px-2 py-1 text-xs text-muted-foreground">
            No sessions yet.
          </span>
        </Sidebar.MenuItem>
      {/if}
    </Sidebar.Menu>
  </Sidebar.GroupContent>
</Sidebar.Group>
