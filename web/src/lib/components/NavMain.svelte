<script lang="ts">
  // NavMain — labeled nav group with iconography. Used twice in
  // the sidebar: once for the agent-scoped nav (just "New chat")
  // and once for the platform nav (Overview / Agent / User).

  import { page } from "$app/state";

  type NavItem = {
    title: string;
    url: string;
    icon?: string;
    active?: boolean;
  };

  let { label, items }: { label?: string; items: NavItem[] } = $props();

  const pathname = $derived(page.url?.pathname || "");

  function isActive(item: NavItem, pathname: string): boolean {
    if (item.active !== undefined) return item.active;
    if (item.url === "/") return pathname === "/";
    return pathname === item.url || pathname.startsWith(item.url);
  }
</script>

{#if items.length > 0}
  <div>
    {#if label}
      <div class="px-2 py-1 text-[10px] uppercase tracking-wider text-zinc-500">
        {label}
      </div>
    {/if}
    <ul class="space-y-0.5">
      {#each items as it (it.url)}
        <li>
          <a
            href={it.url}
            class:font-semibold={isActive(it, pathname)}
            class:bg-zinc-800={isActive(it, pathname)}
            class="flex items-center gap-2 px-2 py-1.5 rounded text-zinc-200 hover:bg-zinc-800/60"
          >
            <span class="w-4 h-4 inline-block text-center text-xs">
              {it.icon === "Bot"
                ? "🤖"
                : it.icon === "Brain"
                  ? "🧠"
                  : it.icon === "Sparkles"
                    ? "✨"
                    : it.icon === "Wrench"
                      ? "🔧"
                      : it.icon === "KeyRound"
                        ? "🔑"
                        : it.icon === "Users"
                          ? "👥"
                          : it.icon === "MessagesSquare"
                            ? "💬"
                            : it.icon === "Coins"
                              ? "🪙"
                              : it.icon === "LayoutDashboard"
                                ? "📊"
                                : it.icon === "Plus"
                                  ? "＋"
                                  : "·"}
            </span>
            <span>{it.title}</span>
          </a>
        </li>
      {/each}
    </ul>
  </div>
{/if}
