<script lang="ts">
  // Signup — public registration form. Gated server-side by
  // /api/admin/registration; when closed, shows a placeholder
  // card and hides the form.

  import { onMount } from 'svelte';
  import { goto } from '$app/navigation';
  import { register, login, getRegistrationOpen } from '$lib/api';
  import Card from '$lib/components/ui/Card.svelte';
  import Button from '$lib/components/ui/Button.svelte';
  import Input from '$lib/components/ui/Input.svelte';
  import Label from '$lib/components/ui/Label.svelte';

  let open = $state(false);
  let username = $state('');
  let email = $state('');
  let password = $state('');
  let confirm = $state('');
  let error = $state('');
  let submitting = $state(false);

  onMount(async () => {
    try {
      const r = await getRegistrationOpen();
      open = !!r.open;
    } catch { open = false; }
  });

  async function submit(e: Event) {
    e.preventDefault();
    error = '';
    if (password !== confirm) { error = "Passwords don't match"; return; }
    if (password.length < 8) { error = "Password must be at least 8 characters"; return; }
    submitting = true;
    try {
      const r = await register({ username, email, password });
      if (!r.ok) { error = r.error || 'signup failed'; submitting = false; return; }
      // auto-login after signup
      await login(username, password);
      await goto('/overview/');
    } catch (e) {
      error = (e as Error).message || 'signup failed';
    } finally { submitting = false; }
  }
</script>

<div class="min-h-screen flex items-center justify-center bg-zinc-950 p-4">
  <div class="w-full max-w-md space-y-4">
    <h1 class="text-2xl font-bold">Sign up</h1>
    {#if !open}
      <Card>
        <h2 class="text-base font-semibold mb-2">Registration is closed</h2>
        <p class="text-sm text-zinc-400">
          Ask an administrator to enable public signups, or use an invite link if one was shared with you.
        </p>
      </Card>
    {:else}
      <Card>
        <h2 class="text-base font-semibold mb-3">Create your account</h2>
        <form onsubmit={submit} class="space-y-3">
          <div><Label for="su-u">Username</Label><Input id="su-u" bind:value={username} required /></div>
          <div><Label for="su-e">Email</Label><Input id="su-e" type="email" bind:value={email} required /></div>
          <div><Label for="su-p">Password (min 8)</Label><Input id="su-p" type="password" bind:value={password} minlength={8} required /></div>
          <div><Label for="su-c">Confirm</Label><Input id="su-c" type="password" bind:value={confirm} minlength={8} required /></div>
          {#if error}<p class="text-sm text-red-400">{error}</p>{/if}
          <Button type="submit" class="w-full" disabled={submitting}>{submitting ? 'Creating…' : 'Sign up'}</Button>
        </form>
      </Card>
    {/if}
  </div>
</div>
