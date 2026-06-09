<script lang="ts">
  // Sidebar — composed with shadcn-svelte's official Sidebar
  // primitives:
  //
  //   <Sidebar.Provider>          (collapsible state, keyboard ⌘B)
  //     <Sidebar.Root>            (the left rail)
  //       <Sidebar.Header>        → AgentSwitcher (DropdownMenu)
  //       <Sidebar.Content>       → NavMain / NavProjects / NavSessions
  //       <Sidebar.Footer>        → NavUser (DropdownMenu with logout)
  //       <Sidebar.Rail>          (icon-only toggle rail)
  //     <Sidebar.Inset>           → main content slot + Sidebar.Trigger
  //
  // Polling, sessions-refresh on agent change, and the
  // settings-dialog mount all live here. Bare constants and
  // small pure helpers stay near the top.

  import { onMount } from "svelte";
  import { page } from "$app/state";
  import {
    getStatus,
    getMe,
    listAgents,
    listChatSessions,
    listProjects,
    type UserInfo,
    type AgentInfo,
    type SessionInfo,
    type ProjectInfo,
    type StatusResponse,
  } from "$lib/api";
  import * as Sidebar from "$lib/components/ui/sidebar/index.js";
  import AgentSwitcher from "./AgentSwitcher.svelte";
  import NavMain from "./NavMain.svelte";
  import NavUser from "./NavUser.svelte";
  import NavProjectsList from "./NavProjectsList.svelte";
  import NavSessions from "./NavSessions.svelte";
  import AgentSettingsDialog from "./AgentSettingsDialog.svelte";

  let { children }: { children?: any } = $props();

  function extractAgentId(pathname: string): string | null {
    const m = pathname.match(
      /^\/agents\/([^/]+)\/(chat|customize|skills|models|sessions|channels|chats|cron|files|plugins|context|usage|project)/,
    );
    return m ? m[1] : null;
  }

  const activeAgentId = $derived(extractAgentId(page.url?.pathname || ""));

  let status = $state<StatusResponse | null>(null);
  let user = $state<UserInfo | null>(null);
  let agents = $state<AgentInfo[]>([]);
  let sessions = $state<SessionInfo[]>([]);
  let projects = $state<ProjectInfo[]>([]);
  let settingsOpen = $state(false);
  let settingsUserOnly = $state(false);

  let pollTimer: ReturnType<typeof setInterval> | undefined;

  onMount(() => {
    void getStatus()
      .then((s) => (status = s))
      .catch(() => {});
    pollTimer = setInterval(() => {
      void getStatus()
        .then((s) => (status = s))
        .catch(() => {});
    }, 15000);
    void getMe()
      .then((r) => (user = r.user ?? null))
      .catch(() => {});
    void listAgents()
      .then((r) => (agents = r.agents ?? []))
      .catch(() => {});

    return () => {
      if (pollTimer) clearInterval(pollTimer);
    };
  });

  let lastAgentId: string | null = null;
  $effect(() => {
    const a = activeAgentId;
    if (a === lastAgentId) return;
    lastAgentId = a;
    if (!a) {
      sessions = [];
      projects = [];
      return;
    }
    const refetch = () => {
      void listChatSessions(a)
        .then((r) => (sessions = r.sessions ?? []))
        .catch(() => {});
      void listProjects(a)
        .then((r) => (projects = r.projects ?? []))
        .catch(() => {});
    };
    refetch();
    const handler = (e: Event) => {
      const detail = (e as CustomEvent<{ agentId?: string }>).detail;
      if (!detail || !detail.agentId || detail.agentId === a) refetch();
    };
    window.addEventListener("cleanclaw:sessions-changed", handler);
    return () =>
      window.removeEventListener("cleanclaw:sessions-changed", handler);
  });

  function broadcastSessionsChanged() {
    if (typeof window !== "undefined" && activeAgentId) {
      window.dispatchEvent(
        new CustomEvent("cleanclaw:sessions-changed", {
          detail: { agentId: activeAgentId },
        }),
      );
    }
  }

  const isAdmin = $derived(status?.isAdmin ?? user?.role === "super_admin");

  const AGENT_NAV = $derived(
    activeAgentId
      ? [
          {
            title: "New chat",
            url: `/agents/${activeAgentId}/chat/?_=${Date.now()}`,
            icon: "Plus" as const,
            active:
              page.url?.pathname === `/agents/${activeAgentId}/chat` ||
              page.url?.pathname === `/agents/${activeAgentId}/chat/`,
          },
        ]
      : [],
  );

  const PLATFORM_OVERVIEW = {
    title: "Overview",
    url: "/overview/",
    icon: "LayoutDashboard" as const,
  };
  const PLATFORM_AGENT_USER = [
    { title: "Agents", url: "/agents/", icon: "Bot" as const },
    { title: "Models", url: "/models/", icon: "Brain" as const },
  ];
  const PLATFORM_AGENT_ADMIN = [
    { title: "Agents", url: "/agents/", icon: "Bot" as const },
    { title: "Models", url: "/models/", icon: "Brain" as const },
    { title: "Skills", url: "/skills/", icon: "Sparkles" as const },
    { title: "Tools", url: "/tools/", icon: "Wrench" as const },
  ];
  const PLATFORM_USER_USER = [
    { title: "API Keys", url: "/apikeys/", icon: "KeyRound" as const },
  ];
  const PLATFORM_USER_ADMIN = [
    { title: "Users", url: "/admin/users/", icon: "Users" as const },
    { title: "Chats", url: "/admin/chats/", icon: "MessagesSquare" as const },
    { title: "Token Usage", url: "/admin/usage/", icon: "Coins" as const },
    { title: "API Keys", url: "/apikeys/", icon: "KeyRound" as const },
  ];

  function openSettings() {
    settingsUserOnly = !activeAgentId;
    settingsOpen = true;
  }
</script>

<Sidebar.Provider
  class="min-h-screen bg-background text-foreground"
  style="--sidebar-width: 16rem; --sidebar-width-icon: 3.5rem;"
>
  <Sidebar.Root collapsible="icon">
    <Sidebar.Header>
      <AgentSwitcher
        {agents}
        {activeAgentId}
        locked={!isAdmin && (user?.agent_quota ?? 1) === 0}
      />
    </Sidebar.Header>

    <Sidebar.Content>
      {#if activeAgentId}
        <NavMain label="Agent" items={AGENT_NAV} />
        <NavProjectsList
          agentId={activeAgentId}
          {projects}
          {sessions}
          onChanged={broadcastSessionsChanged}
        />
        <NavSessions agentId={activeAgentId} {sessions} />
      {:else}
        <NavMain items={[PLATFORM_OVERVIEW]} />
        <NavMain
          label="Agent"
          items={isAdmin ? PLATFORM_AGENT_ADMIN : PLATFORM_AGENT_USER}
        />
        <NavMain
          label="User"
          items={isAdmin ? PLATFORM_USER_ADMIN : PLATFORM_USER_USER}
        />
      {/if}
    </Sidebar.Content>

    <Sidebar.Footer>
      <Sidebar.Menu>
        <Sidebar.MenuItem>
          <Sidebar.MenuButton
            tooltip="Settings"
            onclick={openSettings}
          >
            {#snippet child({ props })}
              <button type="button" {...props}>
                <span>⚙</span>
                <span>Settings</span>
              </button>
            {/snippet}
          </Sidebar.MenuButton>
        </Sidebar.MenuItem>
      </Sidebar.Menu>
      <NavUser
        name={user?.display_name ||
          user?.username ||
          (isAdmin ? "Admin" : "User")}
        subtitle={user?.role || (isAdmin ? "super_admin" : "user")}
        onSettings={openSettings}
      />
    </Sidebar.Footer>

    <Sidebar.Rail />
  </Sidebar.Root>

  <Sidebar.Inset>
    <header
      class="sticky top-0 z-10 flex h-12 shrink-0 items-center gap-2 border-b border-sidebar-border bg-background/80 px-4 backdrop-blur"
    >
      <Sidebar.Trigger class="md:flex hidden" />
      <span class="text-xs text-muted-foreground">
        {#if activeAgentId}
          Agent · {agents.find((a) => a.id === activeAgentId)?.name || activeAgentId}
        {:else}
          {isAdmin ? "Admin Console" : "Workspace"}
        {/if}
      </span>
    </header>
    <div class="flex-1">
      {@render children?.()}
    </div>
  </Sidebar.Inset>
</Sidebar.Provider>

<AgentSettingsDialog
  bind:open={settingsOpen}
  userOnly={settingsUserOnly}
  agentId={activeAgentId || ""}
  role={activeAgentId ? "owner" : "owner"}
  {isAdmin}
  onChanged={broadcastSessionsChanged}
/>
