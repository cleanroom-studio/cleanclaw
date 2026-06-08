<script lang="ts">
  import { onMount } from 'svelte';
  import { listTools, saveTools, type ToolsConfig } from '$lib/api';
  import Card from '$lib/components/ui/Card.svelte';
  import Switch from '$lib/components/ui/Switch.svelte';

  let tools = $state<ToolsConfig>({});
  let loading = $state(true);
  let message = $state('');

  const descriptions: Record<string, string> = {
    web_search: 'Search the web via Brave / Exa / SearXNG',
    image_gen: 'Generate images via DALL·E / Fal / Replicate',
    tts: 'Speech synthesis via OpenAI / ElevenLabs / Fish',
    webfetch: 'Fetch a URL via Direct / Firecrawl / Jina'
  };

  function providersFor(key: string): Array<{ value: string; label: string }> {
    switch (key) {
      case 'web_search': return [{ value: 'brave', label: 'brave' }, { value: 'exa', label: 'exa' }, { value: 'searxng', label: 'searxng' }, { value: 'none', label: 'none' }];
      case 'image_gen': return [{ value: 'openai', label: 'openai' }, { value: 'fal', label: 'fal' }, { value: 'replicate', label: 'replicate' }, { value: 'none', label: 'none' }];
      case 'tts': return [{ value: 'openai', label: 'openai' }, { value: 'elevenlabs', label: 'elevenlabs' }, { value: 'fish', label: 'fish' }, { value: 'minimax', label: 'minimax' }, { value: 'none', label: 'none' }];
      case 'webfetch': return [{ value: 'direct', label: 'direct' }, { value: 'firecrawl', label: 'firecrawl' }, { value: 'jina', label: 'jina' }, { value: 'none', label: 'none' }];
    }
    return [{ value: 'none', label: 'none' }];
  }

  async function refresh() {
    loading = true;
    try {
      const r = await listTools();
      tools = r.tools ?? {};
    } finally { loading = false; }
  }

  function toggle(key: string, value: boolean) {
    if (tools[key]) {
      tools[key] = { ...tools[key], enabled: value };
      save();
    }
  }

  function setProvider(key: string, value: string) {
    if (tools[key]) {
      tools[key] = { ...tools[key], provider: value };
      save();
    }
  }

  async function save() {
    try {
      await saveTools(tools);
      message = 'saved';
      setTimeout(() => (message = ''), 1500);
    } catch (e) {
      message = (e as Error).message || 'save failed';
    }
  }

  onMount(refresh);
</script>

<div class="p-6 max-w-4xl mx-auto space-y-4">
  <div>
    <h2 class="text-2xl font-semibold tracking-tight">Tools</h2>
    <p class="text-sm text-zinc-400 mt-1">Per-category provider routing and on/off toggles.</p>
  </div>

  <Card>
    {#if loading}
      <p class="text-sm text-zinc-500">Loading…</p>
    {:else}
      <ul class="divide-y divide-zinc-800">
        {#each Object.entries(tools) as [key, cfg] (key)}
          <li class="py-3 flex items-center gap-3">
            <div class="flex-1">
              <div class="text-sm font-medium capitalize">{key.replace('_', ' ')}</div>
              <div class="text-xs text-zinc-500">{descriptions[key] || key}</div>
            </div>
            <select
              class="h-9 bg-zinc-900 border border-zinc-700 rounded px-2 text-sm"
              value={cfg.provider}
              onchange={(e) => setProvider(key, (e.currentTarget as HTMLSelectElement).value)}
            >
              {#each providersFor(key) as p (p.value)}
                <option value={p.value}>{p.label}</option>
              {/each}
            </select>
            <Switch checked={cfg.enabled} onchange={() => toggle(key, !cfg.enabled)} />
          </li>
        {/each}
      </ul>
    {/if}
    {#if message}<p class="text-xs text-zinc-400 mt-3">{message}</p>{/if}
  </Card>
</div>
