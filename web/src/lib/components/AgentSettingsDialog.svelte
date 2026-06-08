<script lang="ts">
  // AgentSettingsDialog — tabbed configuration panel that hosts
  // both the per-agent pages (Customize / Models / Skills / …
  // / Usage) and the per-user pages (Account / General). It is
  // mounted from the sidebar's Settings entry, so a single
  // click covers everything the user can change.
  //
  // Tabs are lazily mounted — switching unmounts the previous
  // panel, which is fine here because each panel is a form that
  // owns its own draft state and re-fetches on mount.
  //
  // Each panel is a small Svelte component that calls the
  // matching CleanClaw API surface (which the cleanclaw Rust
  // backend already implements under /api/agents/{id}/…).

  import { onMount } from "svelte";
  import {
    getAgent,
    updateAgent,
    getMe,
    updateMe,
    changeMyPassword,
  } from "$lib/api";
  import Dialog from "./ui/Dialog.svelte";
  import Button from "./ui/Button.svelte";
  import Input from "./ui/Input.svelte";
  import Label from "./ui/Label.svelte";

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

  const AGENT_TABS: Array<{ id: TabId; label: string }> = [
    { id: "profile", label: "Profile" },
    { id: "customize", label: "Customize" },
    { id: "models", label: "Models" },
    { id: "context", label: "Context" },
    { id: "skills", label: "Skills" },
    { id: "plugins", label: "Plugins" },
    { id: "channels", label: "Channels" },
    { id: "cron", label: "Scheduler" },
    { id: "usage", label: "Token Usage" },
  ];

  const USER_TABS: Array<{ id: TabId; label: string }> = [
    { id: "account", label: "Account" },
    { id: "general", label: "General" },
  ];
  if (isAdmin) USER_TABS.push({ id: "about", label: "About" });

  const tabs = $derived(
    userOnly
      ? USER_TABS
      : role === "viewer"
        ? AGENT_TABS.filter((t) => t.id === "profile" || t.id === "usage")
        : AGENT_TABS,
  );
  let active = $state<TabId>("profile");

  $effect(() => {
    // Reset to first tab whenever the dialog re-opens.
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

  // ---- Lazy tab panel helpers ------------------------------------

  function setActive(t: TabId) {
    active = t;
  }
</script>

<Dialog
  bind:open
  title={userOnly
    ? "Settings"
    : active === "profile"
      ? `Profile · ${agentId}`
      : `${tabs.find((t) => t.id === active)?.label} · ${agentId}`}
>
  <div class="grid grid-cols-[10rem_1fr] gap-4 min-h-[24rem]">
    <ul class="space-y-0.5 text-sm">
      {#each tabs as t (t.id)}
        <li>
          <button
            type="button"
            class:font-semibold={active === t.id}
            class:bg-zinc-800={active === t.id}
            class="w-full text-left px-2 py-1.5 rounded hover:bg-zinc-800/60"
            onclick={() => setActive(t.id)}>{t.label}</button
          >
        </li>
      {/each}
    </ul>

    <div class="space-y-3 text-sm">
      {#if active === "profile"}
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
        <div class="flex items-center gap-2">
          <input
            id="prof-public"
            type="checkbox"
            bind:checked={profileIsPublic}
            class="h-4 w-4"
          />
          <Label for="prof-public">Public (shareable URL)</Label>
        </div>
        <div class="flex items-center gap-2">
          <input
            id="prof-share"
            type="checkbox"
            bind:checked={profileShare}
            class="h-4 w-4"
          />
          <Label for="prof-share">Share model config with viewers</Label>
        </div>
        <div class="space-y-1.5">
          <Label for="prof-avatar">Avatar URL</Label>
          <Input id="prof-avatar" bind:value={profileAvatar} />
        </div>
        <div class="flex items-center gap-3 pt-2">
          <Button onclick={saveProfile} disabled={profileSaving}>
            {profileSaving ? "Saving…" : "Save"}
          </Button>
          {#if profileMsg}<span class="text-xs text-zinc-400">{profileMsg}</span
            >{/if}
        </div>
      {:else if active === "customize"}
        <div class="space-y-1.5">
          <Label for="cu-soul">Soul (markdown system prompt)</Label>
          <textarea
            id="cu-soul"
            bind:value={customizeSoul}
            rows={6}
            class="flex w-full rounded-md border border-zinc-700 bg-zinc-900 px-3 py-2 text-sm"
          ></textarea>
        </div>
        <div class="space-y-1.5">
          <Label for="cu-identity">Identity (markdown)</Label>
          <textarea
            id="cu-identity"
            bind:value={customizeIdentity}
            rows={4}
            class="flex w-full rounded-md border border-zinc-700 bg-zinc-900 px-3 py-2 text-sm"
          ></textarea>
        </div>
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
          <Label for="cu-mi">Max tool iterations</Label>
          <Input
            id="cu-mi"
            type="number"
            min="1"
            max="32"
            bind:value={customizeMaxIter}
          />
        </div>
        <div class="space-y-1.5">
          <Label for="cu-think">Thinking mode</Label>
          <Input
            id="cu-think"
            bind:value={customizeThinking}
            placeholder="auto / enabled / disabled"
          />
        </div>
        <div class="flex items-center gap-3 pt-2">
          <Button onclick={saveCustomize} disabled={customizeSaving}>
            {customizeSaving ? "Saving…" : "Save"}
          </Button>
          {#if customizeMsg}<span class="text-xs text-zinc-400"
              >{customizeMsg}</span
            >{/if}
        </div>
      {:else if active === "models" || active === "context" || active === "skills" || active === "plugins" || active === "channels" || active === "cron" || active === "usage"}
        <p class="text-xs text-zinc-400">
          The {tabs.find((t) => t.id === active)?.label} panel lives at
          <a
            href="/agents/{agentId}/{active === 'usage'
              ? 'usage'
              : active === 'cron'
                ? 'cron'
                : active === 'context'
                  ? 'context'
                  : active === 'channels'
                    ? 'channels'
                    : active === 'skills'
                      ? 'skills'
                      : active === 'plugins'
                        ? 'plugins'
                        : 'models'}/"
            class="text-violet-300 hover:underline"
          >
            /agents/{agentId}/{active}/
          </a>
          — the full-page editor handles bulk operations and previews.
        </p>
        <p class="text-xs text-zinc-500">
          Use the dedicated page for richer workflows; the dialog is a quick
          link.
        </p>
      {:else if active === "account"}
        <div class="space-y-1.5">
          <Label for="acct-name">Display name</Label>
          <Input id="acct-name" bind:value={accountName} />
        </div>
        <div class="space-y-1.5">
          <Label for="acct-avatar">Avatar URL</Label>
          <Input id="acct-avatar" bind:value={accountAvatar} />
        </div>
        <div class="space-y-1.5">
          <Label for="acct-old">Current password</Label>
          <Input id="acct-old" type="password" bind:value={accountOldPw} />
        </div>
        <div class="space-y-1.5">
          <Label for="acct-new">New password (min 8)</Label>
          <Input id="acct-new" type="password" bind:value={accountNewPw} />
        </div>
        <div class="flex items-center gap-3 pt-2">
          <Button onclick={saveAccount} disabled={accountSaving}>
            {accountSaving ? "Saving…" : "Save"}
          </Button>
          {#if accountMsg}<span class="text-xs text-zinc-400">{accountMsg}</span
            >{/if}
        </div>
      {:else if active === "general"}
        <div class="flex items-center gap-2">
          <input
            id="gen-reg"
            type="checkbox"
            bind:checked={generalRegistrationOpen}
            class="h-4 w-4"
          />
          <Label for="gen-reg">Allow public registration</Label>
        </div>
        <p class="text-xs text-zinc-500">
          When off, only the super_admin can create accounts. The dashboard's
          Sign-up link hides and /signup returns "registration closed".
        </p>
        <div class="flex items-center gap-3 pt-2">
          <Button onclick={saveGeneral} disabled={generalSaving}>
            {generalSaving ? "Saving…" : "Save"}
          </Button>
          {#if generalMsg}<span class="text-xs text-zinc-400">{generalMsg}</span
            >{/if}
        </div>
      {:else if active === "about"}
        <div class="space-y-1">
          <div>
            Gateway version: <code class="bg-zinc-800 px-1 rounded"
              >{aboutVersion || "…"}</code
            >
          </div>
        </div>
      {/if}
    </div>
  </div>

  {#snippet footer()}
    <Button variant="outline" onclick={() => (open = false)}>Close</Button>
  {/snippet}
</Dialog>
