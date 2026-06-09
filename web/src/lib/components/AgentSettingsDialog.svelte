<script lang="ts">
  // AgentSettingsDialog — tabbed configuration panel that hosts
  // both the per-agent pages (Customize / Models / Skills / …
  // / Usage) and the per-user pages (Account / General). It is
  // mounted from the sidebar's Settings entry, so a single
  // click covers everything the user can change.
  //
  // The tabs are now shadcn-svelte `Tabs.Root` / `Tabs.List` /
  // `Tabs.Trigger` / `Tabs.Content` (built on `bits-ui`'s
  // accessible primitives), with each panel as its own
  // `Tabs.Content` so the layout switches with the keyboard
  // and the URL is still the single source of truth.

  import {
    getAgent,
    updateAgent,
    getMe,
    updateMe,
    changeMyPassword,
  } from "$lib/api";
  import * as Dialog from "./ui/dialog/index.js";
  import { Button } from "./ui/button/index.js";
  import { Input } from "./ui/input/index.js";
  import { Label } from "./ui/label/index.js";
  import { Textarea } from "./ui/textarea/index.js";
  import { Switch } from "./ui/switch/index.js";
  import * as Tabs from "./ui/tabs/index.js";
  import { Separator } from "./ui/separator/index.js";
  import {
    User,
    Paintbrush,
    Brain,
    Layers,
    Sparkles,
    Plug,
    Radio,
    Clock,
    Coins,
    Settings as SettingsIcon,
    Sliders,
    Info,
  } from "@lucide/svelte/icons";

  let {
    open = $bindable(false),
    userOnly = false,
    agentId = "",
    role = "owner",
    isAdmin = false,
    onChanged = () => {},
  }: {
    open?: boolean;
    userOnly?: boolean;
    agentId?: string;
    role?: "owner" | "viewer";
    isAdmin?: boolean;
    onChanged?: () => void;
  } = $props();

  type TabId =
    | "profile"
    | "customize"
    | "models"
    | "context"
    | "skills"
    | "plugins"
    | "channels"
    | "cron"
    | "usage"
    | "account"
    | "general"
    | "about";

  const AGENT_TABS: Array<{ id: TabId; label: string; Icon: any }> = [
    { id: "profile", label: "Profile", Icon: User },
    { id: "customize", label: "Customize", Icon: Paintbrush },
    { id: "models", label: "Models", Icon: Brain },
    { id: "context", label: "Context", Icon: Layers },
    { id: "skills", label: "Skills", Icon: Sparkles },
    { id: "plugins", label: "Plugins", Icon: Plug },
    { id: "channels", label: "Channels", Icon: Radio },
    { id: "cron", label: "Scheduler", Icon: Clock },
    { id: "usage", label: "Token Usage", Icon: Coins },
  ];

  const USER_TABS: Array<{ id: TabId; label: string; Icon: any }> = [
    { id: "account", label: "Account", Icon: User },
    { id: "general", label: "General", Icon: Sliders },
  ];
  if (isAdmin) USER_TABS.push({ id: "about", label: "About", Icon: Info });

  const tabs = $derived(
    userOnly
      ? USER_TABS
      : role === "viewer"
        ? AGENT_TABS.filter((t) => t.id === "profile" || t.id === "usage")
        : AGENT_TABS,
  );
  let active = $state<TabId>("profile");

  $effect(() => {
    const t = tabs;
    if (open && t.length > 0) active = t[0].id;
  });

  // ---- Profile tab ------------------------------------------------

  let profileName = $state("");
  let profileDescription = $state("");
  let profileModel = $state("");
  let profileIsPublic = $state(false);
  let profileShare = $state(false);
  let profileAvatar = $state("");
  let profileSaving = $state(false);
  let profileMsg = $state("");

  $effect(() => {
    if (open && active === "profile" && agentId) {
      void (async () => {
        try {
          const r = await getAgent(agentId);
          const a = r.agent;
          profileName = a.name || "";
          profileDescription = a.description || "";
          profileModel = a.model || "";
          profileIsPublic = !!a.is_public;
          profileShare = !!a.share_model_config;
          profileAvatar = a.avatar_url || "";
        } catch {}
      })();
    }
  });

  async function saveProfile() {
    profileSaving = true;
    profileMsg = "";
    try {
      await updateAgent(agentId, {
        name: profileName,
        description: profileDescription,
        model: profileModel,
        is_public: profileIsPublic,
        share_model_config: profileShare,
        avatar_url: profileAvatar,
      });
      profileMsg = "Saved.";
      onChanged();
    } catch (e) {
      profileMsg = (e as Error).message || "Save failed";
    } finally {
      profileSaving = false;
    }
  }

  // ---- Customize tab ----------------------------------------------

  let customizeSoul = $state("");
  let customizeIdentity = $state("");
  let customizeSystemPrompt = $state("");
  let customizePromptMode = $state("agent");
  let customizeTemp = $state(0.7);
  let customizeMaxTokens = $state(2048);
  let customizeMaxIter = $state(8);
  let customizeThinking = $state("");
  let customizeSaving = $state(false);
  let customizeMsg = $state("");

  $effect(() => {
    if (open && active === "customize" && agentId) {
      void (async () => {
        try {
          const r = await getAgent(agentId);
          const a = r.agent;
          customizeSoul = a.soul || a.system_prompt || "";
          customizeIdentity = a.identity || "";
          customizeSystemPrompt = a.system_prompt || "";
          customizePromptMode = a.prompt_mode || "agent";
          customizeTemp = a.temperature ?? 0.7;
          customizeMaxTokens = a.max_tokens ?? 2048;
          customizeMaxIter = a.max_tool_iterations ?? 8;
          customizeThinking = a.thinking || "";
        } catch {}
      })();
    }
  });

  async function saveCustomize() {
    customizeSaving = true;
    customizeMsg = "";
    try {
      await updateAgent(agentId, {
        soul: customizeSoul,
        identity: customizeIdentity,
        system_prompt: customizeSystemPrompt,
        prompt_mode: customizePromptMode,
        temperature: customizeTemp,
        max_tokens: customizeMaxTokens,
        max_tool_iterations: customizeMaxIter,
        thinking: customizeThinking,
      });
      customizeMsg = "Saved.";
      onChanged();
    } catch (e) {
      customizeMsg = (e as Error).message || "Save failed";
    } finally {
      customizeSaving = false;
    }
  }

  // ---- Account tab (user) -----------------------------------------

  let accountName = $state("");
  let accountAvatar = $state("");
  let accountOldPw = $state("");
  let accountNewPw = $state("");
  let accountMsg = $state("");
  let accountSaving = $state(false);

  $effect(() => {
    if (open && active === "account") {
      void (async () => {
        try {
          const r = await getMe();
          if (r.user) {
            accountName = r.user.display_name || r.user.username || "";
            accountAvatar = r.user.avatar_url || "";
          }
        } catch {}
      })();
    }
  });

  async function saveAccount() {
    accountSaving = true;
    accountMsg = "";
    try {
      await updateMe({ display_name: accountName, avatar_url: accountAvatar });
      if (accountOldPw && accountNewPw) {
        const r = await changeMyPassword(accountOldPw, accountNewPw);
        if (!r.ok) {
          accountMsg = r.error || "Password change failed";
          accountSaving = false;
          return;
        }
      }
      accountMsg = "Saved.";
      accountOldPw = "";
      accountNewPw = "";
    } catch (e) {
      accountMsg = (e as Error).message || "Save failed";
    } finally {
      accountSaving = false;
    }
  }

  // ---- General tab ------------------------------------------------

  let generalRegistrationOpen = $state(false);
  let generalSaving = $state(false);
  let generalMsg = $state("");
  $effect(() => {
    if (open && active === "general" && isAdmin) {
      void (async () => {
        try {
          const r = await fetch("/api/admin/registration", {
            credentials: "same-origin",
          });
          if (r.ok) {
            const j = await r.json();
            generalRegistrationOpen = !!j.open;
          }
        } catch {}
      })();
    }
  });
  async function saveGeneral() {
    generalSaving = true;
    generalMsg = "";
    try {
      await fetch("/api/admin/registration", {
        method: "PUT",
        credentials: "same-origin",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ open: generalRegistrationOpen }),
      });
      generalMsg = "Saved.";
    } catch (e) {
      generalMsg = (e as Error).message || "Save failed";
    } finally {
      generalSaving = false;
    }
  }

  // ---- About tab (admin) -----------------------------------------

  let aboutVersion = $state("");
  $effect(() => {
    if (open && active === "about") {
      void (async () => {
        try {
          const r = await fetch("/api/status", { credentials: "same-origin" });
          if (r.ok) {
            const j = await r.json();
            aboutVersion = j.version || "dev";
          }
        } catch {}
      })();
    }
  });

  // Helper: link target for "advanced" agent tabs that defer to
  // the dedicated full-page editor.
  function advancedPath(id: TabId): string {
    const map: Partial<Record<TabId, string>> = {
      models: "models",
      context: "context",
      skills: "skills",
      plugins: "plugins",
      channels: "channels",
      cron: "cron",
      usage: "usage",
    };
    const sub = map[id] || id;
    return `/agents/${agentId}/${sub}/`;
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Content class="max-w-2xl">
    <Dialog.Header>
      <Dialog.Title>
        {userOnly
          ? "Settings"
          : active === "profile"
            ? `Profile · ${agentId}`
            : `${tabs.find((t) => t.id === active)?.label} · ${agentId}`}
      </Dialog.Title>
    </Dialog.Header>
    <div>
  <Tabs.Root bind:value={active} class="w-full">
    <Tabs.List
      class="flex flex-wrap items-center gap-1 bg-transparent p-0 border-b border-border mb-4"
    >
      {#each tabs as t (t.id)}
        <Tabs.Trigger
          value={t.id}
          class="gap-1.5 rounded-t-md rounded-b-none border-b-2 border-transparent data-[state=active]:border-primary data-[state=active]:bg-muted/40 px-3 py-1.5 text-xs"
        >
          <t.Icon class="size-3.5" />
          <span>{t.label}</span>
        </Tabs.Trigger>
      {/each}
    </Tabs.List>

    <div class="min-h-[20rem]">
      <Tabs.Content value="profile" class="space-y-3">
        <div class="space-y-1.5">
          <Label for="prof-name">Name</Label>
          <Input id="prof-name" bind:value={profileName} />
        </div>
        <div class="space-y-1.5">
          <Label for="prof-desc">Description</Label>
          <Input id="prof-desc" bind:value={profileDescription} />
        </div>
        <div class="space-y-1.5">
          <Label for="prof-model">Model</Label>
          <Input
            id="prof-model"
            bind:value={profileModel}
            placeholder="openai/MiniMax-M3"
          />
        </div>
        <div class="flex items-center justify-between rounded-md border border-border p-3">
          <div class="space-y-0.5">
            <Label for="prof-public" class="text-sm">Public (shareable URL)</Label>
            <p class="text-xs text-muted-foreground">
              Anyone with the link can start a chat.
            </p>
          </div>
          <Switch id="prof-public" bind:checked={profileIsPublic} />
        </div>
        <div class="flex items-center justify-between rounded-md border border-border p-3">
          <div class="space-y-0.5">
            <Label for="prof-share" class="text-sm">Share model config with viewers</Label>
            <p class="text-xs text-muted-foreground">
              Viewers see the model + customizations on the share page.
            </p>
          </div>
          <Switch id="prof-share" bind:checked={profileShare} />
        </div>
        <div class="space-y-1.5">
          <Label for="prof-avatar">Avatar URL</Label>
          <Input id="prof-avatar" bind:value={profileAvatar} />
        </div>
        <Separator />
        <div class="flex items-center gap-3 pt-1">
          <Button onclick={saveProfile} disabled={profileSaving}>
            {profileSaving ? "Saving…" : "Save"}
          </Button>
          {#if profileMsg}
            <span class="text-xs text-muted-foreground">{profileMsg}</span>
          {/if}
        </div>
      </Tabs.Content>

      <Tabs.Content value="customize" class="space-y-3">
        <div class="space-y-1.5">
          <Label for="cu-soul">Soul (markdown system prompt)</Label>
          <Textarea id="cu-soul" bind:value={customizeSoul} rows={6} />
        </div>
        <div class="space-y-1.5">
          <Label for="cu-identity">Identity (markdown)</Label>
          <Textarea id="cu-identity" bind:value={customizeIdentity} rows={4} />
        </div>
        <div class="grid grid-cols-3 gap-3">
          <div class="space-y-1.5">
            <Label for="cu-temp">Temperature</Label>
            <Input
              id="cu-temp"
              type="number"
              step="0.05"
              min="0"
              max="2"
              bind:value={customizeTemp}
            />
          </div>
          <div class="space-y-1.5">
            <Label for="cu-mt">Max tokens</Label>
            <Input
              id="cu-mt"
              type="number"
              min="64"
              max="200000"
              bind:value={customizeMaxTokens}
            />
          </div>
          <div class="space-y-1.5">
            <Label for="cu-mi">Max tool iter</Label>
            <Input
              id="cu-mi"
              type="number"
              min="1"
              max="32"
              bind:value={customizeMaxIter}
            />
          </div>
        </div>
        <div class="space-y-1.5">
          <Label for="cu-think">Thinking mode</Label>
          <Input
            id="cu-think"
            bind:value={customizeThinking}
            placeholder="auto / enabled / disabled"
          />
        </div>
        <Separator />
        <div class="flex items-center gap-3 pt-1">
          <Button onclick={saveCustomize} disabled={customizeSaving}>
            {customizeSaving ? "Saving…" : "Save"}
          </Button>
          {#if customizeMsg}
            <span class="text-xs text-muted-foreground">{customizeMsg}</span>
          {/if}
        </div>
      </Tabs.Content>

      {#each ["models", "context", "skills", "plugins", "channels", "cron", "usage"] as id (id)}
        <Tabs.Content value={id} class="space-y-2 text-sm">
          <p class="text-muted-foreground">
            The {tabs.find((t) => t.id === id)?.label} panel lives at
            <a href={advancedPath(id)} class="text-primary hover:underline">
              {advancedPath(id)}
            </a>
            — the full-page editor handles bulk operations and previews.
          </p>
          <p class="text-xs text-muted-foreground">
            Use the dedicated page for richer workflows; this dialog is a
            quick link.
          </p>
        </Tabs.Content>
      {/each}

      <Tabs.Content value="account" class="space-y-3">
        <div class="space-y-1.5">
          <Label for="acct-name">Display name</Label>
          <Input id="acct-name" bind:value={accountName} />
        </div>
        <div class="space-y-1.5">
          <Label for="acct-avatar">Avatar URL</Label>
          <Input id="acct-avatar" bind:value={accountAvatar} />
        </div>
        <Separator />
        <div class="space-y-1.5">
          <Label for="acct-old">Current password</Label>
          <Input id="acct-old" type="password" bind:value={accountOldPw} />
        </div>
        <div class="space-y-1.5">
          <Label for="acct-new">New password (min 8)</Label>
          <Input id="acct-new" type="password" bind:value={accountNewPw} />
        </div>
        <div class="flex items-center gap-3 pt-1">
          <Button onclick={saveAccount} disabled={accountSaving}>
            {accountSaving ? "Saving…" : "Save"}
          </Button>
          {#if accountMsg}
            <span class="text-xs text-muted-foreground">{accountMsg}</span>
          {/if}
        </div>
      </Tabs.Content>

      <Tabs.Content value="general" class="space-y-3">
        <div class="flex items-center justify-between rounded-md border border-border p-3">
          <div class="space-y-0.5">
            <Label for="gen-reg" class="text-sm">Allow public registration</Label>
            <p class="text-xs text-muted-foreground">
              When off, only the super_admin can create accounts. The dashboard's
              Sign-up link hides and /signup returns "registration closed".
            </p>
          </div>
          <Switch id="gen-reg" bind:checked={generalRegistrationOpen} />
        </div>
        <div class="flex items-center gap-3 pt-1">
          <Button onclick={saveGeneral} disabled={generalSaving}>
            {generalSaving ? "Saving…" : "Save"}
          </Button>
          {#if generalMsg}
            <span class="text-xs text-muted-foreground">{generalMsg}</span>
          {/if}
        </div>
      </Tabs.Content>

      <Tabs.Content value="about" class="space-y-2 text-sm">
        <div class="flex items-center gap-2">
          <SettingsIcon class="size-4 text-muted-foreground" />
          <span>Gateway version:</span>
          <code class="rounded bg-muted px-1.5 py-0.5 font-mono text-xs">
            {aboutVersion || "…"}
          </code>
        </div>
      </Tabs.Content>
    </div>
  </Tabs.Root>
    </div>
    <Dialog.Footer>
      <Button variant="outline" onclick={() => (open = false)}>Close</Button>
    </Dialog.Footer>
  </Dialog.Content>
</Dialog.Root>
