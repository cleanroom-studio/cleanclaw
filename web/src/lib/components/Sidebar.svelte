<script lang="ts">
  // Sidebar — full replacement for
  // + `sidebar.tsx` combined into a single Svelte component.
  //
  // Composition:
  //   • SidebarProvider     — left rail
  //   • AppSidebar
  //       • SidebarHeader   — AgentSwitcher (dropdown to swap active agent)
  //       • SidebarContent  — nav groups (Overview / Agent / User) +
  //                           (when an agent is active) NavProjectsList +
  //                           NavSessions
  //       • SidebarFooter   — Settings button + NavUser
  //   • SidebarInset        — the main content slot (passed `children`)
  //
  // Polling:
  //   • /api/status every 15s → admin flag, online dot
  //   • /api/me once on mount → role + display name in the footer
  //   • /api/agents once on mount → agent switcher list
  //   • /api/chat/sessions when active agent changes → sessions list
  //
  // Events:
  //   • `cleanclaw:sessions-changed` is broadcast by the chat
  //     surface after a new chat / rename / delete so the sidebar
  //     re-fetches without a page reload.

  import { onMount, onDestroy } from "svelte";
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
  import AgentSwitcher from "./AgentSwitcher.svelte";
  import NavMain from "./NavMain.svelte";
  import NavUser from "./NavUser.svelte";
  import NavProjectsList from "./NavProjectsList.svelte";
  import NavSessions from "./NavSessions.svelte";
  import AgentSettingsDialog from "./AgentSettingsDialog.svelte";

  let { children }: { children?: any } = $props();

  // Active agent id parsed from `/agents/<id>/...` paths. The
  // explicit allow-list of sub-routes keeps `/agents/` (the
  // index list) on the platform nav.
  function extractAgentId(pathname: string): string | null {
    const m = pathname.match(
      /^\/agents\/([^/]+)\/(chat|customize|skills|models|sessions|channels|chats|cron|files|plugins|context|usage|project)/,
    );
    return m ? m[1] : null;
  }
  function extractSessionKey(pathname: string, search: string): string | null {
    // Both `?session=<id>` query on `/chat/` and the
    // `/chat/<session>/` path segment are valid.
    const m = pathname.match(/^\/(chat|agents\/[^/]+\/chat)\/([^/?#]+)/);
    if (m && m[2] !== "undefined" && m[2] !== "new")
      return decodeURIComponent(m[2]);
    try {
      const params = new URLSearchParams(search || "");
      const s = params.get("session");
      if (s) return s;
    } catch {}
    return null;
  }
  const activeAgentId = $derived(extractAgentId(page.url?.pathname || ""));
  const activeSessionKey = $derived(
    extractSessionKey(page.url?.pathname || "", page.url?.search || ""),
  );
  const hasOpenSession = $derived(!!activeSessionKey);

  let status = $state<StatusResponse | null>(null);
  let user = $state<UserInfo | null>(null);
  let agents = $state<AgentInfo[]>([]);
  let sessions = $state<SessionInfo[]>([]);
  let projects = $state<ProjectInfo[]>([]);
  let settingsOpen = $state(false);
  let settingsUserOnly = $state(false);

  let pollTimer: ReturnType<typeof setInterval> | undefined;

  // Status polling — keep the online dot and admin flag fresh.
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

  // Re-fetch sessions + projects whenever the active agent changes
  // OR the chat surface broadcasts a sessions-changed event.
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

  // Nav item helpers.
  const AGENT_NAV = $derived(
    activeAgentId
      ? [
          {
            title: "New chat",
            url: `/agents/${activeAgentId}/chat/`,
            icon: "Plus",
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
    icon: "LayoutDashboard",
  };
  const PLATFORM_AGENT_USER = [
    { title: "Agents", url: "/agents/", icon: "Bot" },
    { title: "Models", url: "/models/", icon: "Brain" },
  ];
  const PLATFORM_AGENT_ADMIN = [
    { title: "Agents", url: "/agents/", icon: "Bot" },
    { title: "Models", url: "/models/", icon: "Brain" },
    { title: "Skills", url: "/skills/", icon: "Sparkles" },
    { title: "Tools", url: "/tools/", icon: "Wrench" },
  ];
  const PLATFORM_USER_USER = [
    { title: "API Keys", url: "/apikeys/", icon: "KeyRound" },
  ];
  const PLATFORM_USER_ADMIN = [
    { title: "Users", url: "/admin/users/", icon: "Users" },
    { title: "Chats", url: "/admin/chats/", icon: "MessagesSquare" },
    { title: "Token Usage", url: "/admin/usage/", icon: "Coins" },
    { title: "API Keys", url: "/apikeys/", icon: "KeyRound" },
  ];
</script>

<div class="min-h-screen flex bg-zinc-950 text-zinc-100">
  <aside class="w-64 shrink-0 border-r border-zinc-800 flex flex-col">
    <div class="p-3 border-b border-zinc-800">
      <AgentSwitcher
        {agents}
        {activeAgentId}
        locked={!isAdmin && (user?.agent_quota ?? 1) === 0}
      />
    </div>

    <nav class="flex-1 overflow-y-auto p-2 space-y-4 text-sm">
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
    </nav>

    <div class="p-3 border-t border-zinc-800 space-y-2">
      <button
        type="button"
        class="w-full flex items-center gap-2 px-2 py-1.5 text-sm rounded hover:bg-zinc-800/60"
        onclick={() => {
          settingsUserOnly = !activeAgentId;
          settingsOpen = true;
        }}
      >
        <span>⚙</span>
        <span>Settings</span>
      </button>
      <NavUser
        name={user?.display_name ||
          user?.username ||
          (isAdmin ? "Admin" : "User")}
        subtitle={user?.role || (isAdmin ? "super_admin" : "user")}
      />
    </div>
  </aside>

  <main class="flex-1 overflow-y-auto">
    {@render children?.()}
  </main>
</div>

<AgentSettingsDialog
  bind:open={settingsOpen}
  userOnly={settingsUserOnly}
  agentId={activeAgentId || ""}
  role={activeAgentId ? "owner" : "owner"}
  {isAdmin}
  onChanged={broadcastSessionsChanged}
/>
