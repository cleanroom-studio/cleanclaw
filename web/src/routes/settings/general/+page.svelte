<script lang="ts">
  import { adminGetRegistration, adminSetRegistration } from "$lib/api";
  import { Card } from "$lib/components/ui/card/index.js";
  import { Button } from "$lib/components/ui/button/index.js";
  import { Switch } from "$lib/components/ui/switch/index.js";

  let open = $state(false);
  let saving = $state(false);
  let msg = $state("");

  $effect(() => {
    void (async () => {
      try {
        const r = await adminGetRegistration();
        open = !!r.open;
      } catch {}
    })();
  });

  async function save() {
    saving = true;
    msg = "";
    try {
      await adminSetRegistration(open);
      msg = "saved";
    } catch (e) {
      msg = (e as Error).message;
    } finally {
      saving = false;
    }
  }
</script>

<div class="p-6 max-w-2xl mx-auto space-y-4">
  <h2 class="text-2xl font-semibold tracking-tight">General</h2>
  <Card>
    <div class="flex items-center justify-between">
      <div>
        <div class="text-sm font-medium">Allow public registration</div>
        <div class="text-xs text-zinc-500 mt-0.5">
          When off, only the super_admin can create accounts. The Sign-up link
          hides and /signup returns "registration closed".
        </div>
      </div>
      <Switch checked={open} onchange={() => (open = !open)} />
    </div>
    <div class="flex items-center gap-3 mt-3">
      <Button onclick={save} disabled={saving}
        >{saving ? "Saving…" : "Save"}</Button
      >
      {#if msg}<span class="text-xs text-zinc-400">{msg}</span>{/if}
    </div>
  </Card>
</div>
