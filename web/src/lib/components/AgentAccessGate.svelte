<script lang="ts">
  // AgentAccessGate — per-agent access probe. Lives at the
  // [id]/layout level so every nested route inherits the same
  // gate.
  //
  //   • 200 → render children (caller is owner / super_admin /
  //     public-link visitor / apikey ACL grantee)
  //   • 401 → /login (handled at the auth layer)
  //   • 403 / 404 / anything else → "no access" overlay

  import { onMount } from "svelte";
  import { page } from "$app/state";
  import { getAgent } from "$lib/api";

  let { children }: { children?: any } = $props();

  function agentIdFromPath(pathname: string): string {
    const m = pathname.match(/^\/agents\/([^/]+)/);
    return m ? m[1] : "";
  }

  let state = $state<"checking" | "ok" | "denied">("checking");

  onMount(() => {
    const pathname = page.url?.pathname || "";
    const agentId = agentIdFromPath(pathname);
    // "default" is the prebuilt static-export placeholder, not a
    // real agent — skip the probe and let children render.
    if (!agentId || agentId === "default") {
      state = "ok";
      return;
    }
    let aborted = false;
    getAgent(agentId)
      .then((r) => {
        if (aborted) return;
        if (r?.agent) state = "ok";
        else state = "denied";
      })
      .catch(() => {
        if (!aborted) state = "denied";
      });
    return () => {
      aborted = true;
    };
  });
</script>

{#if state === "checking"}
  <div class="fixed inset-0 z-50 flex items-center justify-center bg-zinc-950">
    <div class="h-2 w-2 animate-pulse rounded-full bg-zinc-600"></div>
  </div>
{:else if state === "denied"}
  <div
    class="fixed inset-0 z-50 flex items-center justify-center bg-zinc-950 p-6"
  >
    <div class="max-w-md text-center space-y-4">
      <div
        class="mx-auto flex h-14 w-14 items-center justify-center rounded-2xl bg-zinc-800"
      >
        <span class="text-2xl">🤖</span>
      </div>
      <h2 class="text-lg font-semibold">No access to this agent</h2>
      <p class="text-sm text-zinc-400">
        This agent is private to its owner, or the link is no longer valid. If
        the owner shares it publicly, the chat URL will start working for you
        automatically.
      </p>
    </div>
  </div>
{:else}
  {@render children?.()}
{/if}
