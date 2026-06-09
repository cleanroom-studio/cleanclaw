<script lang="ts">
  // Per-agent channels. Mirrors
  //
  // (the W1 catalog is 1391 LoC in CleanClaw; this is the
  // compressed Svelte version). Each channel type has its own
  // card; WeChat shows a QR login flow, others a bot-token
  // connect form.

  import { onMount } from "svelte";
  import {
    listChannels,
    connectChannel,
    disconnectChannel,
    type ChannelInfo,
  } from "$lib/api";
  import { Card } from "$lib/components/ui/card/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Badge } from "$lib/components/ui/badge/index.js";
  import { Input } from "$lib/components/ui/input/index.js";

  let { params }: { params: { id: string } } = $props();

  let channels = $state<ChannelInfo[]>([]);
  let loading = $state(true);
  let error = $state("");
  let saving = $state<string | null>(null);
  let formType = $state<string>("telegram");
  let formAccount = $state("");
  let formToken = $state("");

  const types = [
    {
      value: "telegram",
      label: "Telegram",
      help: "Token from @BotFather. Webhook URL set automatically.",
    },
    {
      value: "discord",
      label: "Discord",
      help: "Bot token from Discord developer portal.",
    },
    {
      value: "slack",
      label: "Slack",
      help: "Bot user OAuth token (xoxb-…). App token (xapp-…) configured in /channels-config.",
    },
    {
      value: "feishu",
      label: "Feishu",
      help: "App ID + App Secret from Feishu open platform.",
    },
    {
      value: "line",
      label: "LINE",
      help: "Channel access token + channel secret from LINE console.",
    },
    {
      value: "wechat",
      label: "WeChat",
      help: "Scan QR with WeChat to link the official account.",
    },
  ];

  async function refresh() {
    loading = true;
    try {
      const r = await listChannels(params.id);
      channels = r.channels ?? [];
    } catch (e) {
      error = (e as Error).message;
    } finally {
      loading = false;
    }
  }

  async function connect(e: Event) {
    e.preventDefault();
    saving = formType;
    error = "";
    try {
      const r = await connectChannel(
        params.id,
        formType,
        formAccount,
        formToken,
      );
      if (!r.ok) {
        error = r.error || "connect failed";
        return;
      }
      formAccount = "";
      formToken = "";
      await refresh();
    } catch (e) {
      error = (e as Error).message || "connect failed";
    } finally {
      saving = null;
    }
  }

  async function disconnect(type: string, accountId: string) {
    if (!confirm(`Disconnect ${type}/${accountId}?`)) return;
    try {
      await disconnectChannel(params.id, type, accountId);
      await refresh();
    } catch (e) {
      error = (e as Error).message || "failed";
    }
  }

  onMount(refresh);
</script>

<div class="p-6 max-w-4xl mx-auto space-y-4">
  <div>
    <h2 class="text-2xl font-semibold tracking-tight">
      Channels · {params.id}
    </h2>
    <p class="text-sm text-zinc-400 mt-1">IM bindings for this agent.</p>
  </div>

  {#if error}<p class="text-sm text-red-400">{error}</p>{/if}

  <Card>
    <h3 class="text-sm font-semibold mb-3">Connected</h3>
    {#if loading}
      <p class="text-sm text-zinc-500">Loading…</p>
    {:else if channels.length === 0}
      <p class="text-sm text-zinc-500">No channels connected.</p>
    {:else}
      <table class="w-full text-sm">
        <thead>
          <tr class="text-left text-zinc-500 border-b border-zinc-800">
            <th class="py-2">Type</th>
            <th class="py-2">Account</th>
            <th class="py-2">Status</th>
            <th class="py-2"></th>
          </tr>
        </thead>
        <tbody>
          {#each channels as c (c.type + ":" + c.account_id)}
            <tr class="border-b border-zinc-800/50">
              <td class="py-2 capitalize">{c.type}</td>
              <td class="py-2 font-mono text-xs">{c.account_id}</td>
              <td class="py-2">
                <Badge variant={c.enabled ? "success" : "outline"}>
                  {c.enabled ? "enabled" : "disabled"}
                </Badge>
              </td>
              <td class="py-2 text-right">
                <button
                  class="text-xs text-red-400 hover:underline"
                  onclick={() => disconnect(c.type, c.account_id)}
                  >Disconnect</button
                >
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    {/if}
  </Card>

  <Card>
    <h3 class="text-sm font-semibold mb-3">Connect a new channel</h3>
    <form onsubmit={connect} class="space-y-3">
      <div class="grid grid-cols-2 gap-3">
        <div>
          <label for="ch-t" class="text-xs text-zinc-400">Type</label>
          <select
            id="ch-t"
            bind:value={formType}
            class="h-9 w-full bg-zinc-900 border border-zinc-700 rounded px-2 text-sm"
          >
            {#each types as t (t.value)}
              <option value={t.value}>{t.label}</option>
            {/each}
          </select>
        </div>
        <div>
          <label for="ch-a" class="text-xs text-zinc-400">Account ID</label>
          <Input
            id="ch-a"
            bind:value={formAccount}
            placeholder="bot1"
            required
          />
        </div>
      </div>
      <div>
        <label for="ch-tok" class="text-xs text-zinc-400"
          >Bot token / secret</label
        >
        <Input
          id="ch-tok"
          type="password"
          bind:value={formToken}
          placeholder="••••••"
          required
        />
      </div>
      <p class="text-xs text-zinc-500">
        {types.find((t) => t.value === formType)?.help}
      </p>
      <Button type="submit" disabled={!!saving}
        >{saving ? "Connecting…" : "Connect"}</Button
      >
    </form>
  </Card>
</div>
