<script lang="ts">
  // NavUser — `Sidebar.Footer` with avatar + name + a DropdownMenu
  // (Account / Settings / Sign out). Uses the official shadcn
  // pattern so it composes with both the expanded and icon-only
  // sidebar states.

  import { goto } from "$app/navigation";
  import { logout } from "$lib/api";
  import * as Sidebar from "$lib/components/ui/sidebar/index.js";
  import * as DropdownMenu from "$lib/components/ui/dropdown-menu/index.js";
  import ChevronsUpDown from "@lucide/svelte/icons/chevrons-up-down";
  import LogOut from "@lucide/svelte/icons/log-out";
  import Settings from "@lucide/svelte/icons/settings";
  import UserCog from "@lucide/svelte/icons/user-cog";

  let {
    name,
    subtitle,
    onSettings,
  }: {
    name: string;
    subtitle: string;
    onSettings?: () => void;
  } = $props();

  async function doLogout() {
    try {
      await logout();
    } catch {}
    void goto("/");
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
              class="flex size-7 items-center justify-center rounded-full bg-sidebar-accent text-sidebar-accent-foreground text-xs font-semibold"
            >
              {(name || "?").slice(0, 1).toUpperCase()}
            </div>
            <div class="grid flex-1 text-left text-sm leading-tight">
              <span class="truncate font-medium">{name}</span>
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
        align="end"
        sideOffset={4}
      >
        <DropdownMenu.Label class="text-xs text-muted-foreground">
          Signed in as
        </DropdownMenu.Label>
        <DropdownMenu.Separator />
        <DropdownMenu.Item onSelect={onSettings} class="gap-2 p-2">
          <Settings class="size-4" />
          <span>Settings</span>
        </DropdownMenu.Item>
        <DropdownMenu.Item disabled class="gap-2 p-2">
          <UserCog class="size-4" />
          <span>Profile</span>
        </DropdownMenu.Item>
        <DropdownMenu.Separator />
        <DropdownMenu.Item onSelect={doLogout} class="gap-2 p-2">
          <LogOut class="size-4" />
          <span>Sign out</span>
        </DropdownMenu.Item>
      </DropdownMenu.Content>
    </DropdownMenu.Root>
  </Sidebar.MenuItem>
</Sidebar.Menu>
