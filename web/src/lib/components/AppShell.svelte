<script lang="ts">
  // AppShell — mounts the sidebar (AgentSwitcher + nav groups +
  // settings dialog) once for every authenticated page and keeps
  // that instance alive across client-side navigations.
  //
  //
  // The sidebar is the home of the role-based nav, the agent
  // switcher dropdown, the per-agent sessions list, and the
  // settings dialog (which itself hosts the 9-tab
  // AgentSettingsDialog). Building it once means role checks,
  // status polling, and session refreshes all stay coherent as
  // the user navigates between /agents, /chat, /admin/...

  import { onMount } from "svelte";
  import { page } from "$app/state";
  import Sidebar from "./Sidebar.svelte";

  let { children }: { children?: any } = $props();

  // Paths that render bare (no sidebar chrome). /onboard is here
  // because the bootstrap flow is a focused wizard, not the
  // authenticated app surface.
  const BARE_PATHS = ["/", "/onboard", "/signup"];

  function wantsSidebar(pathname: string): boolean {
    if (!pathname) return true;
    if (BARE_PATHS.includes(pathname)) return false;
    if (pathname.startsWith("/onboard/")) return false;
    if (pathname.startsWith("/signup/")) return false;
    return true;
  }

  // Status polling for the sidebar's online dot / admin flag /
  // agent switcher — run on the shell, not on the page, so it
  // survives navigations.
  let _mounted = $state(false);
  onMount(() => {
    _mounted = true;
  });
</script>

{#if !wantsSidebar(page.url?.pathname || "")}
  {@render children?.()}
{:else}
  <Sidebar>
    {@render children?.()}
  </Sidebar>
{/if}
