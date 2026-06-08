<script lang="ts">
  // LoginScreen — flips between sign-in and sign-up inline rather
  // than navigating to /signup. Two reasons: (a) the share-URL
  // path the user landed on stays in the address bar, so a
  // successful sign-up lands them straight on the page they came
  // for; (b) /signup as a separate route leaks the authenticated
  // app chrome to a not-yet-registered visitor.
  //
  // Registration on the server sets the session cookie, so a
  // sign-up success is functionally a sign-in success — we route
  // both through `onSuccess`.

  import {
    login as apiLogin,
    register,
    getRegistrationOpen,
    type LoginResponse,
  } from "$lib/api";

  let {
    onSuccess,
    error,
    registrationOpen = $bindable(false),
  }: {
    onSuccess?: (resp: LoginResponse) => void | Promise<void>;
    error?: string | null;
    registrationOpen?: boolean;
  } = $props();

  let mode = $state<"signin" | "signup">("signin");
  let loginField = $state("");
  let password = $state("");
  let signupUsername = $state("");
  let signupEmail = $state("");
  let signupConfirm = $state("");
  let localError = $state("");
  let loading = $state(false);

  $effect(() => {
    // Probe registration toggle once on mount so we know whether
    // to surface the "Sign up" link below the sign-in form.
    (async () => {
      try {
        const s = await getRegistrationOpen();
        registrationOpen = !!s.open;
      } catch {
        // server down — hide sign-up link rather than guess.
      }
    })();
  });

  function switchMode(next: "signin" | "signup") {
    localError = "";
    mode = next;
  }

  async function handleSignIn(e: Event) {
    e.preventDefault();
    if (!loginField.trim() || !password) return;
    setLoading(true);
    setError("");
    try {
      const res = await apiLogin({ username: loginField.trim(), password });
      if (!res.ok) {
        setError((res as any).error || "Invalid credentials");
        setLoading(false);
        return;
      }
      await onSuccess?.(res);
    } catch {
      setError("Cannot reach server");
      setLoading(false);
    }
  }

  async function handleSignUp(e: Event) {
    e.preventDefault();
    setError("");
    if (!signupUsername.trim() || !signupEmail.trim() || !password) {
      setError("All fields are required");
      return;
    }
    if (password.length < 8) {
      setError("Password must be at least 8 characters");
      return;
    }
    if (password !== signupConfirm) {
      setError("Passwords don't match");
      return;
    }
    setLoading(true);
    try {
      const res = await register({
        username: signupUsername.trim(),
        email: signupEmail.trim(),
        password,
      });
      if (!res.ok) {
        setError((res as any).error || "Could not create account");
        setLoading(false);
        return;
      }
      // Register handler set the session cookie on our response;
      // reuse the same callback sign-in uses and AuthGuard will
      // render the originally-requested route.
      await onSuccess?.({ ok: true, user_id: (res as any).user_id });
    } catch {
      setError("Cannot reach server");
      setLoading(false);
    }
  }

  function setLoading(v: boolean) {
    loading = v;
  }
  function setError(e: string) {
    localError = e;
  }
</script>

<div class="flex min-h-screen items-center justify-center bg-zinc-950 p-4">
  <div class="w-full max-w-sm space-y-6">
    <div class="text-center space-y-2">
      <h1 class="text-2xl font-bold text-zinc-100">CleanClaw</h1>
      <p class="text-sm text-zinc-500">
        {mode === "signin" ? "Sign in to your account" : "Create your account"}
      </p>
    </div>

    {#if mode === "signup"}
      <form onsubmit={handleSignUp} class="space-y-4">
        <input
          type="text"
          bind:value={signupUsername}
          placeholder="username"
          autocomplete="username"
          class="w-full rounded-lg border border-zinc-800 bg-zinc-900 px-4 py-3 text-sm text-zinc-100 placeholder-zinc-600 outline-none focus:border-violet-500 focus:ring-1 focus:ring-violet-500"
        />
        <input
          type="email"
          bind:value={signupEmail}
          placeholder="email"
          autocomplete="email"
          class="w-full rounded-lg border border-zinc-800 bg-zinc-900 px-4 py-3 text-sm text-zinc-100 placeholder-zinc-600 outline-none focus:border-violet-500 focus:ring-1 focus:ring-violet-500"
        />
        <input
          type="password"
          bind:value={password}
          placeholder="password (min 8 chars)"
          autocomplete="new-password"
          class="w-full rounded-lg border border-zinc-800 bg-zinc-900 px-4 py-3 text-sm text-zinc-100 placeholder-zinc-600 outline-none focus:border-violet-500 focus:ring-1 focus:ring-violet-500"
        />
        <input
          type="password"
          bind:value={signupConfirm}
          placeholder="confirm password"
          autocomplete="new-password"
          class="w-full rounded-lg border border-zinc-800 bg-zinc-900 px-4 py-3 text-sm text-zinc-100 placeholder-zinc-600 outline-none focus:border-violet-500 focus:ring-1 focus:ring-violet-500"
        />
        {#if localError || error}
          <p class="text-sm text-red-400">{localError || error}</p>
        {/if}
        <button
          type="submit"
          disabled={loading}
          class="w-full rounded-lg bg-violet-600 px-4 py-3 text-sm font-medium text-white transition hover:bg-violet-500 disabled:opacity-50 disabled:cursor-not-allowed"
        >
          {loading ? "Creating account…" : "Create account"}
        </button>
      </form>
      <p class="text-center text-sm text-zinc-500">
        Already have an account?
        <button
          type="button"
          onclick={() => switchMode("signin")}
          class="text-violet-400 hover:text-violet-300"
        >
          Sign in
        </button>
      </p>
    {:else}
      <form onsubmit={handleSignIn} class="space-y-4">
        <input
          type="text"
          bind:value={loginField}
          placeholder="username or email"
          autocomplete="username"
          class="w-full rounded-lg border border-zinc-800 bg-zinc-900 px-4 py-3 text-sm text-zinc-100 placeholder-zinc-600 outline-none focus:border-violet-500 focus:ring-1 focus:ring-violet-500"
        />
        <input
          type="password"
          bind:value={password}
          placeholder="password"
          autocomplete="current-password"
          class="w-full rounded-lg border border-zinc-800 bg-zinc-900 px-4 py-3 text-sm text-zinc-100 placeholder-zinc-600 outline-none focus:border-violet-500 focus:ring-1 focus:ring-violet-500"
        />
        {#if localError || error}
          <p class="text-sm text-red-400">{localError || error}</p>
        {/if}
        <button
          type="submit"
          disabled={loading}
          class="w-full rounded-lg bg-violet-600 px-4 py-3 text-sm font-medium text-white transition hover:bg-violet-500 disabled:opacity-50 disabled:cursor-not-allowed"
        >
          {loading ? "Signing in…" : "Sign in"}
        </button>
      </form>
      {#if registrationOpen}
        <p class="text-center text-sm text-zinc-500">
          Don't have an account?
          <button
            type="button"
            onclick={() => switchMode("signup")}
            class="text-violet-400 hover:text-violet-300"
          >
            Sign up
          </button>
        </p>
      {/if}
    {/if}
  </div>
</div>
