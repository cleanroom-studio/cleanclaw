<script lang="ts">
  import { getChannelConfig, saveChannelConfig } from "$lib/api";
  import Card from "$lib/components/ui/Card.svelte";
  import Button from "$lib/components/ui/Button.svelte";
  import Input from "$lib/components/ui/Input.svelte";
  import Label from "$lib/components/ui/Label.svelte";

  let telegramUrl = $state("");
  let telegramSecret = $state("");
  let discordPublicKey = $state("");
  let discordBotToken = $state("");
  let slackSigningSecret = $state("");
  let slackBotToken = $state("");
  let feishuAppId = $state("");
  let feishuAppSecret = $state("");
  let lineChannelSecret = $state("");
  let lineChannelToken = $state("");
  let wechatAppId = $state("");
  let wechatAppSecret = $state("");
  let wechatToken = $state("");
  let saving = $state(false);
  let message = $state("");
  let error = $state("");

  async function load() {
    try {
      const r = await getChannelConfig();
      const c = (r as any).config || {};
      telegramUrl = c.telegram?.webhook_url || "";
      telegramSecret = c.telegram?.secret || "";
      discordPublicKey = c.discord?.public_key || "";
      discordBotToken = c.discord?.bot_token || "";
      slackSigningSecret = c.slack?.signing_secret || "";
      slackBotToken = c.slack?.bot_token || "";
      feishuAppId = c.feishu?.app_id || "";
      feishuAppSecret = c.feishu?.app_secret || "";
      lineChannelSecret = c.line?.channel_secret || "";
      lineChannelToken = c.line?.channel_token || "";
      wechatAppId = c.wechat?.app_id || "";
      wechatAppSecret = c.wechat?.app_secret || "";
      wechatToken = c.wechat?.token || "";
    } catch {}
  }

  async function save() {
    saving = true;
    message = "";
    error = "";
    try {
      await saveChannelConfig({
        telegram: { webhook_url: telegramUrl, secret: telegramSecret },
        discord: { public_key: discordPublicKey, bot_token: discordBotToken },
        slack: { signing_secret: slackSigningSecret, bot_token: slackBotToken },
        feishu: { app_id: feishuAppId, app_secret: feishuAppSecret },
        line: {
          channel_secret: lineChannelSecret,
          channel_token: lineChannelToken,
        },
        wechat: {
          app_id: wechatAppId,
          app_secret: wechatAppSecret,
          token: wechatToken,
        },
      });
      message = "saved";
      setTimeout(() => (message = ""), 2000);
    } catch (e) {
      error = (e as Error).message || "save failed";
    } finally {
      saving = false;
    }
  }

  $effect(() => {
    void load();
  });
</script>

<div class="p-6 max-w-4xl mx-auto space-y-4">
  <div>
    <h2 class="text-2xl font-semibold tracking-tight">Channel config</h2>
    <p class="text-sm text-zinc-400 mt-1">
      Platform-level defaults. Per-agent bindings (bot token etc.) are still
      managed on the agent's own Channels page.
    </p>
  </div>

  <Card>
    <h3 class="text-sm font-semibold mb-3">Telegram</h3>
    <div class="grid grid-cols-2 gap-3">
      <div>
        <Label for="tg-url">Webhook URL</Label>
        <Input
          id="tg-url"
          bind:value={telegramUrl}
          placeholder="https://gateway/..."
        />
      </div>
      <div>
        <Label for="tg-secret">Secret (header token)</Label>
        <Input id="tg-secret" type="password" bind:value={telegramSecret} />
      </div>
    </div>
  </Card>

  <Card>
    <h3 class="text-sm font-semibold mb-3">Discord</h3>
    <div class="grid grid-cols-2 gap-3">
      <div>
        <Label for="dc-pk">Public key</Label><Input
          id="dc-pk"
          bind:value={discordPublicKey}
        />
      </div>
      <div>
        <Label for="dc-tk">Bot token</Label><Input
          id="dc-tk"
          type="password"
          bind:value={discordBotToken}
        />
      </div>
    </div>
  </Card>

  <Card>
    <h3 class="text-sm font-semibold mb-3">Slack</h3>
    <div class="grid grid-cols-2 gap-3">
      <div>
        <Label for="sk-ss">Signing secret</Label><Input
          id="sk-ss"
          type="password"
          bind:value={slackSigningSecret}
        />
      </div>
      <div>
        <Label for="sk-tk">Bot token</Label><Input
          id="sk-tk"
          type="password"
          bind:value={slackBotToken}
        />
      </div>
    </div>
  </Card>

  <Card>
    <h3 class="text-sm font-semibold mb-3">Feishu</h3>
    <div class="grid grid-cols-2 gap-3">
      <div>
        <Label for="fs-id">App ID</Label><Input
          id="fs-id"
          bind:value={feishuAppId}
        />
      </div>
      <div>
        <Label for="fs-ss">App secret</Label><Input
          id="fs-ss"
          type="password"
          bind:value={feishuAppSecret}
        />
      </div>
    </div>
  </Card>

  <Card>
    <h3 class="text-sm font-semibold mb-3">LINE</h3>
    <div class="grid grid-cols-2 gap-3">
      <div>
        <Label for="ln-cs">Channel secret</Label><Input
          id="ln-cs"
          type="password"
          bind:value={lineChannelSecret}
        />
      </div>
      <div>
        <Label for="ln-ct">Channel access token</Label><Input
          id="ln-ct"
          type="password"
          bind:value={lineChannelToken}
        />
      </div>
    </div>
  </Card>

  <Card>
    <h3 class="text-sm font-semibold mb-3">WeChat</h3>
    <div class="grid grid-cols-2 gap-3">
      <div>
        <Label for="wc-id">App ID</Label><Input
          id="wc-id"
          bind:value={wechatAppId}
        />
      </div>
      <div>
        <Label for="wc-as">App secret</Label><Input
          id="wc-as"
          type="password"
          bind:value={wechatAppSecret}
        />
      </div>
      <div class="col-span-2">
        <Label for="wc-tk">Token (URL validation)</Label><Input
          id="wc-tk"
          type="password"
          bind:value={wechatToken}
        />
      </div>
    </div>
  </Card>

  <div class="flex items-center gap-3">
    <Button onclick={save} disabled={saving}
      >{saving ? "Saving…" : "Save config"}</Button
    >
    {#if message}<span class="text-sm text-zinc-400">{message}</span>{/if}
    {#if error}<span class="text-sm text-red-400">{error}</span>{/if}
  </div>
</div>
