<script lang="ts">
  import { onMount } from "svelte";
  import { getMe, updateMe, changeMyPassword, type UserInfo } from "$lib/api";
  import Card from "$lib/components/ui/Card.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import Input from "$lib/components/ui/Input.svelte";
  import Label from "$lib/components/ui/Label.svelte";

  let user = $state<UserInfo | null>(null);
  let displayName = $state("");
  let avatarUrl = $state("");
  let oldPw = $state("");
  let newPw = $state("");
  let saving = $state(false);
  let msg = $state("");

  async function load() {
    try {
      const r = await getMe();
      user = r.user ?? null;
      if (user) displayName = user.display_name || user.username || "";
    } catch {}
  }

  async function saveProfile(e: Event) {
    e.preventDefault();
    saving = true;
    msg = "";
    try {
      await updateMe({ display_name: displayName, avatar_url: avatarUrl });
      msg = "saved";
    } catch (err) {
      msg = (err as Error).message;
    } finally {
      saving = false;
    }
  }

  async function changePw(e: Event) {
    e.preventDefault();
    msg = "";
    try {
      const r = await changeMyPassword(oldPw, newPw);
      if (!r.ok) {
        msg = r.error || "failed";
        return;
      }
      msg = "password updated";
      oldPw = "";
      newPw = "";
    } catch (err) {
      msg = (err as Error).message;
    }
  }

  onMount(load);
</script>

<div class="p-6 max-w-2xl mx-auto space-y-4">
  <h2 class="text-2xl font-semibold tracking-tight">Account</h2>

  <Card>
    <h3 class="text-sm font-semibold mb-3">Profile</h3>
    <form onsubmit={saveProfile} class="space-y-3">
      <div>
        <Label for="dn">Display name</Label>
        <Input id="dn" bind:value={displayName} required />
      </div>
      <div>
        <Label for="av">Avatar URL</Label>
        <Input id="av" bind:value={avatarUrl} placeholder="https://…" />
      </div>
      <div class="flex items-center gap-3">
        <Button type="submit" disabled={saving}
          >{saving ? "Saving…" : "Save"}</Button
        >
        {#if msg}<span class="text-xs text-zinc-400">{msg}</span>{/if}
      </div>
    </form>
  </Card>

  <Card>
    <h3 class="text-sm font-semibold mb-3">Change password</h3>
    <form onsubmit={changePw} class="space-y-3">
      <div>
        <Label for="opw">Current password</Label>
        <Input id="opw" type="password" bind:value={oldPw} required />
      </div>
      <div>
        <Label for="npw">New password</Label>
        <Input
          id="npw"
          type="password"
          bind:value={newPw}
          minlength={8}
          required
        />
      </div>
      <Button type="submit">Update password</Button>
    </form>
  </Card>
</div>
