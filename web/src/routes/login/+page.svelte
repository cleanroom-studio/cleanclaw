<script lang="ts">
  // Login — bare sign-in form (used by the AppShell-bypass path
  // for direct `/login/` visits; the regular in-app login is
  // served by AuthGuard → LoginScreen). Mirrors CleanClaw's
  // signin form shape.

  import { goto } from "$app/navigation";
  import { login, getMe, type LoginResponse } from "$lib/api";
  import { Card } from "$lib/components/ui/card/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Input } from "$lib/components/ui/input/index.js";
  import { Label } from "$lib/components/ui/label/index.js";

  let username = $state("");
  let password = $state("");
  let error = $state("");
  let submitting = $state(false);

  async function submit(e: Event) {
    e.preventDefault();
    submitting = true;
    error = "";
    try {
      const r: LoginResponse = await login({ username, password });
      if (!r.ok) {
        error = r.error || "invalid credentials";
        submitting = false;
        return;
      }
      const m = await getMe();
      if (m.ok && m.user) {
        await goto("/overview/");
      } else {
        await goto("/overview/");
      }
    } catch (e) {
      error = (e as Error).message || "sign-in failed";
    } finally {
      submitting = false;
    }
  }
</script>

<div class="min-h-screen flex items-center justify-center bg-zinc-950 p-4">
  <Card class="w-full max-w-sm">
    <h1 class="text-xl font-bold mb-4">Sign in</h1>
    <form onsubmit={submit} class="space-y-3">
      <div>
        <Label for="li-u">Username</Label>
        <Input
          id="li-u"
          bind:value={username}
          required
          autocomplete="username"
        />
      </div>
      <div>
        <Label for="li-p">Password</Label>
        <Input
          id="li-p"
          type="password"
          bind:value={password}
          required
          autocomplete="current-password"
        />
      </div>
      {#if error}<p class="text-sm text-red-400">{error}</p>{/if}
      <Button type="submit" class="w-full" disabled={submitting}
        >{submitting ? "Signing in…" : "Sign in"}</Button
      >
      <p class="text-xs text-zinc-500 text-center">
        No account? <a href="/signup/" class="text-violet-300 hover:underline"
          >Sign up</a
        >
      </p>
    </form>
  </Card>
</div>
