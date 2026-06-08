<script lang="ts">
  // ConfigureSkillDialog — env / API key entry for a single
  // skill.
  // The backend stores skill entries as part of the scoped
  // config row; we round-trip through /api/config.

  import Dialog from "./ui/Dialog.svelte";
  import Button from "./ui/Button.svelte";
  import Input from "./ui/Input.svelte";
  import Label from "./ui/Label.svelte";
  import { readConfig, writeConfig, type SkillInfo } from "$lib/api";

  export type SkillEntryView = {
    apiKey?: string;
    env?: Record<string, string>;
  };

  let {
    open = $bindable(false),
    skill,
    agentId,
  }: { open?: boolean; skill: SkillInfo | null; agentId?: string } = $props();

  let apiKey = $state("");
  let envPairs = $state<Array<{ key: string; value: string }>>([]);
  let saving = $state(false);
  let message = $state("");

  $effect(() => {
    if (open && skill) {
      void (async () => {
        try {
          const r = await readConfig("agent", "skill-entries", agentId);
          const entries = (r.value as any)?.entries || {};
          const e: SkillEntryView = entries[skill.name] || {};
          apiKey = e.apiKey || "";
          envPairs = Object.entries(e.env || {}).map(([key, value]) => ({
            key,
            value,
          }));
        } catch {}
      })();
    }
  });

  function addEnv() {
    envPairs = [...envPairs, { key: "", value: "" }];
  }

  function removeEnv(idx: number) {
    envPairs = envPairs.filter((_, i) => i !== idx);
  }

  async function save() {
    if (!skill) return;
    saving = true;
    message = "";
    try {
      // Read-modify-write the entries map.
      const r = await readConfig("agent", "skill-entries", agentId);
      const entries = (r.value as any)?.entries || {};
      const envObj: Record<string, string> = {};
      for (const p of envPairs) {
        if (p.key.trim()) envObj[p.key.trim()] = p.value;
      }
      entries[skill.name] = { apiKey, env: envObj };
      await writeConfig("agent", "skill-entries", { entries }, agentId);
      message = "Saved.";
    } catch (e) {
      message = (e as Error).message || "Save failed";
    } finally {
      saving = false;
    }
  }
</script>

<Dialog bind:open title={skill ? `Configure ${skill.name}` : "Configure skill"}>
  <div class="space-y-4 text-sm">
    <div class="space-y-1.5">
      <Label for="sk-key">API key</Label>
      <Input
        id="sk-key"
        type="password"
        bind:value={apiKey}
        placeholder="sk-…"
      />
    </div>

    <div class="space-y-1.5">
      <Label>Env / extra vars</Label>
      <div class="space-y-2">
        {#each envPairs as p, i (i)}
          <div class="flex items-center gap-2">
            <Input bind:value={p.key} placeholder="KEY" class="w-32" />
            <Input bind:value={p.value} placeholder="value" />
            <button
              type="button"
              class="text-zinc-500 hover:text-red-400 text-xs"
              onclick={() => removeEnv(i)}>×</button
            >
          </div>
        {/each}
        <button type="button" class="text-xs text-violet-300" onclick={addEnv}
          >+ Add env var</button
        >
      </div>
    </div>

    {#if message}
      <p class="text-xs text-zinc-400">{message}</p>
    {/if}
  </div>
  {#snippet footer()}
    <Button variant="outline" onclick={() => (open = false)}>Cancel</Button>
    <Button onclick={save} disabled={saving}
      >{saving ? "Saving…" : "Save"}</Button
    >
  {/snippet}
</Dialog>
