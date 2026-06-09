<script lang="ts">
  // NavMain — labeled nav group rendered with the shadcn-svelte
  // `Sidebar.*` primitives so it picks up the icon-only collapsed
  // state automatically (icons stay visible, labels hide).

  import { page } from "$app/state";
  import * as Sidebar from "$lib/components/ui/sidebar/index.js";
  import Plus from "@lucide/svelte/icons/plus";
  import Bot from "@lucide/svelte/icons/bot";
  import Brain from "@lucide/svelte/icons/brain";
  import Sparkles from "@lucide/svelte/icons/sparkles";
  import Wrench from "@lucide/svelte/icons/wrench";
  import KeyRound from "@lucide/svelte/icons/key-round";
  import Users from "@lucide/svelte/icons/users";
  import MessagesSquare from "@lucide/svelte/icons/messages-square";
  import Coins from "@lucide/svelte/icons/coins";
  import LayoutDashboard from "@lucide/svelte/icons/layout-dashboard";
  import type { Component } from "svelte";

  type IconName =
    | "Bot"
    | "Brain"
    | "Sparkles"
    | "Wrench"
    | "KeyRound"
    | "Users"
    | "MessagesSquare"
    | "Coins"
    | "LayoutDashboard"
    | "Plus";

  type NavItem = {
    title: string;
    url: string;
    icon?: IconName;
    active?: boolean;
    onclick?: () => void;
  };

  let { label, items }: { label?: string; items: NavItem[] } = $props();

  const pathname = $derived(page.url?.pathname || "");

  function isActive(item: NavItem, pathname: string): boolean {
    if (item.active !== undefined) return item.active;
    if (item.url === "/") return pathname === "/";
    return pathname === item.url || pathname.startsWith(item.url);
  }

  const ICONS: Record<IconName, Component> = {
    Bot,
    Brain,
    Sparkles,
    Wrench,
    KeyRound,
    Users,
    MessagesSquare,
    Coins,
    LayoutDashboard,
    Plus,
  };
</script>

{#if items.length > 0}
  <Sidebar.Group>
    {#if label}
      <Sidebar.GroupLabel>{label}</Sidebar.GroupLabel>
    {/if}
    <Sidebar.GroupContent>
      <Sidebar.Menu>
        {#each items as it (it.url)}
          <Sidebar.MenuItem>
            <Sidebar.MenuButton isActive={isActive(it, pathname)}>
              {#snippet child({ props })}
                {@const Icon = ICONS[it.icon ?? "Bot"]}
                {#if it.onclick}
                  <button
                    type="button"
                    onclick={it.onclick}
                    {...props}
                  >
                    <Icon />
                    <span>{it.title}</span>
                  </button>
                {:else}
                  <a href={it.url} {...props}>
                    <Icon />
                    <span>{it.title}</span>
                  </a>
                {/if}
              {/snippet}
            </Sidebar.MenuButton>
          </Sidebar.MenuItem>
        {/each}
      </Sidebar.Menu>
    </Sidebar.GroupContent>
  </Sidebar.Group>
{/if}
