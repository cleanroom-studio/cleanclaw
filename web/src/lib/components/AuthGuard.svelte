<script lang="ts">
  // Full AuthGuard —
  //   - First call decides between three states:
  //     1) users table empty → redirect to /onboard
  //     2) users exist, caller has a session → render children
  //     3) users exist, caller has no session → render <LoginScreen>
  //   - /signup is a public route when admin opens registration; the
  //     screen itself re-checks the toggle and surfaces "registration
  //     is closed" if the operator flipped it off between page load
  //     and submit.
  //   - admin path / skill / tool / plugin / providers / etc. prefixes
  //     are gated: non-admins are redirected to /overview.

  import { onMount, untrack } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import {
    getStatus,
    getMe,
    getRegistrationOpen,
    type UserInfo,
    type LoginResponse,
  } from "$lib/api";
  import LoginScreen from "./LoginScreen.svelte";

  let { children }: { children?: any } = $props();

  const ADMIN_PATH_PREFIXES = [
    "/admin/",
    "/skills",
    "/providers",
    "/channels",
    "/channels-config",
    "/plugins",
    "/tools",
    "/cron",
  ];

  function isAdminPath(pathname: string): boolean {
    return ADMIN_PATH_PREFIXES.some(
      (p) =>
        pathname === p ||
        pathname === p.replace(/\/$/, "") ||
        pathname.startsWith(p + (p.endsWith("/") ? "" : "/")),
    );
  }

  let checked = $state(false);
  let authed = $state(false);
  let user = $state<UserInfo | null>(null);
  let registrationOpen = $state(false);
  let error = $state<string | null>(null);

  // re-run whenever the route changes so a fresh /api/me probe
  // happens after login / logout navigation.
  let lastPath = $state("");
  $effect(() => {
    const p = page.url?.pathname || "";
    if (p === lastPath && checked) return;
    lastPath = p;
    void probe();
  });

  async function probe() {
    error = null;
    let configured = true;
    try {
      const status = await getStatus();
      // The setup register handler refuses to mint a non-admin
      // when the users table is empty; the dashboard mirrors that
      // by routing everyone to /onboard so the first visitor
      // bootstraps the super_admin.
      configured = !!status.user_count || status.configured;
    } catch {
      // server down — fall through to LoginScreen
    }
    if (!configured) {
      const onOnboard =
        page.url?.pathname === "/onboard" ||
        page.url?.pathname?.startsWith("/onboard/");
      if (!onOnboard) {
        await goto("/onboard/");
        return;
      }
      authed = true;
      checked = true;
      return;
    }
    if (
      page.url?.pathname === "/signup" ||
      page.url?.pathname?.startsWith("/signup/")
    ) {
      authed = true;
      checked = true;
      try {
        const r = await getRegistrationOpen();
        registrationOpen = !!r.open;
      } catch {}
      return;
    }
    try {
      const r = await getMe();
      if (r.ok && r.user) {
        if (
          isAdminPath(page.url?.pathname || "") &&
          r.user.role !== "super_admin"
        ) {
          await goto("/overview/");
          return;
        }
        user = r.user;
        authed = true;
      }
    } catch {}
    checked = true;
  }

  async function onLoginSuccess(resp: LoginResponse) {
    // Re-probe the auth state; the server just set the session
    // cookie, so /api/me will return the user.
    checked = false;
    authed = false;
    await probe();
  }
</script>

{#if !checked}
  <div class="flex min-h-screen items-center justify-center bg-zinc-950">
    <div
      class="h-8 w-8 animate-spin rounded-full border-2 border-zinc-700 border-t-violet-500"
    ></div>
  </div>
{:else if !authed}
  <LoginScreen onSuccess={onLoginSuccess} {error} bind:registrationOpen />
{:else}
  {@render children?.()}
{/if}
