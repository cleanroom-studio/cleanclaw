<script lang="ts">
  // AgentSwitcher — `Sidebar.Header` dropdown that lists every
  // agent the caller can see and routes to the chosen agent's
  // chat. Uses shadcn-svelte DropdownMenu wrapped in a
  // `Sidebar.MenuButton` so it collapses to a single icon in
  // icon-only mode.

  import { goto } from "$app/navigation";
  import * as Sidebar from "$lib/components/ui/sidebar/index.js";
  import * as DropdownMenu from "$lib/components/ui/dropdown-menu/index.js";
  import ChevronsUpDown from "@lucide/svelte/icons/chevrons-up-down";
  import Check from "@lucide/svelte/icons/check";
  import Plus from "@lucide/svelte/icons/plus";
  import type { AgentInfo } from "$lib/api";

  let {
    agents,
    activeAgentId,
    locked = false,
  }: {
    agents: AgentInfo[];
    activeAgentId: string | null;
    locked?: boolean;
  } = $props();

  const active = $derived(agents.find((a) => a.id === activeAgentId) || null);
  const label = $derived(active?.name || activeAgentId || "Select agent");
  const subtitle = $derived(active?.model || "—");

  function selectAgent(id: string) {
    void goto(`/agents/${id}/chat/`);
  }

  function manage() {
    void goto("/agents/");
  }
</script>

<Sidebar.Menu>
  <Sidebar.MenuItem>
    <DropdownMenu.Root>
      <DropdownMenu.Trigger>
        {#snippet child({ props })}
          <Sidebar.MenuButton
            {...props}
            size="lg"
            class="data-[state=open]:bg-sidebar-accent data-[state=open]:text-sidebar-accent-foreground"
          >
            <div
              class="flex size-7 items-center justify-center rounded-md bg-sidebar-primary text-sidebar-primary-foreground text-xs font-semibold"
            >
              {(label || "?").slice(0, 1).toUpperCase()}
            </div>
            <div class="grid flex-1 text-left text-sm leading-tight">
              <span class="truncate font-medium">{label}</span>
              <span class="truncate text-[10px] text-muted-foreground">
                {subtitle}
              </span>
            </div>
            <ChevronsUpDown class="ms-auto size-4" />
          </Sidebar.MenuButton>
        {/snippet}
      </DropdownMenu.Trigger>
      <DropdownMenu.Content
        class="min-w-56 rounded-lg"
        side="right"
        align="start"
        sideOffset={4}
      >
        <DropdownMenu.Label class="text-xs text-muted-foreground">
          Agents
        </DropdownMenu.Label>
        {#each agents as a (a.id)}
          <DropdownMenu.Item
            onSelect={() => selectAgent(a.id)}
            class="gap-2 p-2"
          >
            <div
              class="flex size-6 items-center justify-center rounded-sm border bg-background"
            >
              {(a.name || a.id).slice(0, 1).toUpperCase()}
            </div>
            <div class="flex-1 min-w-0">
              <div class="truncate font-medium">{a.name || a.id}</div>
              <div class="truncate text-[10px] text-muted-foreground font-mono">
                {a.model}
              </div>
            </div>
            {#if a.id === activeAgentId}
              <Check class="size-4" />
            {/if}
          </DropdownMenu.Item>
        {/each}
        {#if agents.length === 0}
          <DropdownMenu.Separator />
          <DropdownMenu.Item disabled>No agents yet.</DropdownMenu.Item>
        {/if}
        {#if !locked}
          <DropdownMenu.Separator />
          <DropdownMenu.Item onSelect={manage} class="gap-2 p-2">
            <div
              class="flex size-6 items-center justify-center rounded-md border bg-background"
            >
              <Plus class="size-4" />
            </div>
            <div class="font-medium text-muted-foreground">Manage agents</div>
          </DropdownMenu.Item>
        {/if}
      </DropdownMenu.Content>
    </DropdownMenu.Root>
  </Sidebar.MenuItem>
</Sidebar.Menu>
