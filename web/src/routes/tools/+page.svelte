<script lang="ts">
  import { onMount } from 'svelte';
  import { listTools, saveTools, type ToolsConfig } from '$lib/api';
  import Card from '$lib/components/ui/Card.svelte';
  import Switch from '$lib/components/ui/Switch.svelte';
  import Input from '$lib/components/ui/Input.svelte';
  import Label from '$lib/components/ui/Label.svelte';
  import Button from '$lib/components/ui/Button.svelte';

  // Per-provider metadata. `credential_free` providers don't need
  // an API key; `endpoint_required` providers need an extra
  // parameter (Google = `cx=…`, Baidu = uses the URL).
  type ProviderKey = 'duckduckgo' | 'brave' | 'bing' | 'google' | 'baidu' | 'searxng' | 'exa' | 'none';
  type Category = 'web_search' | 'webfetch' | 'image_gen' | 'tts';

  type ProviderMeta = {
    key: ProviderKey;
    label: string;
    description: string;
    docs: string;
    needsKey: boolean;
    needsEndpoint: boolean;
    endpointHint?: string;
  };

  const WEB_SEARCH_PROVIDERS: ProviderMeta[] = [
    { key: 'duckduckgo', label: 'DuckDuckGo', description: 'Free. No key. HTML scrape. Works globally — good default.', docs: 'https://duckduckgo.com', needsKey: false, needsEndpoint: false },
    { key: 'brave',      label: 'Brave Search',  description: 'Independent index. Fast, high quality. 2000 free queries / month.', docs: 'https://brave.com/search/api', needsKey: true, needsEndpoint: false },
    { key: 'bing',       label: 'Microsoft Bing', description: 'Web Search API v7. Good for enterprise.', docs: 'https://learn.microsoft.com/en-us/bing/search-apis/', needsKey: true, needsEndpoint: false },
    { key: 'google',     label: 'Google CSE',    description: 'Programmable Search Engine. Limited 100/day free.', docs: 'https://developers.google.com/custom-search', needsKey: true, needsEndpoint: true, endpointHint: 'cx=<engine-id>' },
    { key: 'baidu',      label: 'Baidu 百度',     description: 'Free. No key. HTML scrape. Best from CN IPs; may captcha elsewhere.', docs: 'https://www.baidu.com', needsKey: false, needsEndpoint: false },
    { key: 'searxng',    label: 'SearXNG',       description: 'Meta-search. Self-hosted instance URL required.', docs: 'https://searxng.org', needsKey: false, needsEndpoint: true, endpointHint: 'https://searx.example.com' },
    { key: 'exa',        label: 'Exa',           description: 'Neural search. 1000 free / month.', docs: 'https://exa.ai', needsKey: true, needsEndpoint: false },
    { key: 'none',       label: 'Disabled (none)', description: 'Disable web search entirely.', docs: '', needsKey: false, needsEndpoint: false },
  ];

  const WEBFETCH_PROVIDERS = [
    { value: 'direct', label: 'Direct' },
    { value: 'firecrawl', label: 'Firecrawl' },
    { value: 'jina', label: 'Jina' },
  ];
  const IMAGEGEN_PROVIDERS = [
    { value: 'openai', label: 'OpenAI' },
    { value: 'fal', label: 'fal' },
    { value: 'replicate', label: 'Replicate' },
    { value: 'none', label: 'none' },
  ];
  const TTS_PROVIDERS = [
    { value: 'openai', label: 'OpenAI' },
    { value: 'elevenlabs', label: 'ElevenLabs' },
    { value: 'fish', label: 'Fish' },
    { value: 'minimax', label: 'minimax' },
    { value: 'none', label: 'none' },
  ];

  type Cat = {
    key: Category;
    label: string;
    desc: string;
    options: { value: string; label: string }[];
  };
  const CATEGORIES: Cat[] = [
    { key: 'web_search', label: 'Web Search',  desc: descriptions_web_search, options: WEB_SEARCH_PROVIDERS.map(p => ({ value: p.key, label: p.label })) },
    { key: 'image_gen',  label: 'Image Gen',   desc: 'DALL·E / Fal / Replicate.',                       options: IMAGEGEN_PROVIDERS },
    { key: 'tts',        label: 'Text-to-Speech', desc: 'OpenAI / ElevenLabs / Fish / minimax.',          options: TTS_PROVIDERS },
    { key: 'webfetch',   label: 'Web Fetch',   desc: 'Direct / Firecrawl / Jina.',                     options: WEBFETCH_PROVIDERS },
  ];

  function descriptions_web_search() { return 'Search the web via DuckDuckGo / Brave / Bing / Google / Baidu / Exa / SearXNG.'; }

  let tools = $state<ToolsConfig>({});
  let apiKeys = $state<Record<string, string>>({});
  let endpoints = $state<Record<string, string>>({});
  let loading = $state(true);
  let message = $state('');
  let savingKey = $state<string | null>(null);

  async function refresh() {
    loading = true;
    try {
      const r = await listTools();
      tools = r.tools ?? {};
      // Backend returns the new shape with provider/enabled/api_key/endpoint.
      // Keep `tools` shape clean — api_key / endpoint are displayed separately.
      apiKeys = {};
      endpoints = {};
      for (const k of Object.keys(tools)) {
        const v: any = tools[k] || {};
        if (typeof v.api_key === 'string') apiKeys[k] = v.api_key;
        if (typeof v.endpoint === 'string') endpoints[k] = v.endpoint;
      }
    } finally { loading = false; }
  }

  function providerMeta(category: Category, key: string): ProviderMeta | undefined {
    if (category !== 'web_search') return undefined;
    return WEB_SEARCH_PROVIDERS.find(p => p.key === key);
  }

  async function setProvider(category: Category, value: string) {
    if (!tools[category]) tools[category] = { enabled: true, provider: value } as any;
    else (tools[category] as any).provider = value;
    tools = { ...tools };
    await persistCategory(category);
  }

  function toggle(category: Category, value: boolean) {
    if (!tools[category]) tools[category] = { enabled: value, provider: 'duckduckgo' } as any;
    else (tools[category] as any).enabled = value;
    tools = { ...tools };
    persistCategory(category);
  }

  async function setApiKey(category: Category, key: string) {
    apiKeys = { ...apiKeys, [category]: key };
    await persistCategory(category);
  }

  async function setEndpoint(category: Category, value: string) {
    endpoints = { ...endpoints, [category]: value };
    await persistCategory(category);
  }

  async function persistCategory(category: Category) {
    savingKey = category;
    try {
      // Merge api_key + endpoint into the category object.
      const t: any = { ...(tools[category] as any) };
      if (apiKeys[category] !== undefined) t.api_key = apiKeys[category];
      if (endpoints[category] !== undefined) t.endpoint = endpoints[category];
      const next: ToolsConfig = { ...tools, [category]: t };
      await saveTools(next);
      tools = next;
      message = 'saved';
      setTimeout(() => (message = ''), 1500);
    } catch (e) {
      message = (e as Error).message || 'save failed';
    } finally { savingKey = null; }
  }

  onMount(refresh);
</script>

<div class="p-6 max-w-4xl mx-auto space-y-4">
  <div>
    <h2 class="text-2xl font-semibold tracking-tight">Tools</h2>
    <p class="text-sm text-zinc-400 mt-1">Per-category provider routing and on/off toggles. Search credentials are stored system-wide; the agent picks up the row on the next turn.</p>
  </div>

  {#if message}
    <p class="text-xs text-zinc-400">{message}</p>
  {/if}

  <Card>
    {#if loading}
      <p class="text-sm text-zinc-500">Loading…</p>
    {:else}
      <ul class="divide-y divide-zinc-800">
        {#each CATEGORIES as cat (cat.key)}
          {@const cfg = (tools[cat.key] as any) || { enabled: true, provider: 'none' }}
          {@const provider = cfg.provider || 'none'}
          {@const enabled = cfg.enabled !== false}
          {@const meta = providerMeta(cat.key, provider)}
          <li class="py-4 space-y-3">
            <div class="flex items-center gap-3">
              <div class="flex-1">
                <div class="text-sm font-medium">{cat.label}</div>
                <div class="text-xs text-zinc-500">{cat.desc}</div>
              </div>
              <select
                class="h-9 bg-zinc-900 border border-zinc-700 rounded px-2 text-sm"
                value={provider}
                onchange={(e) => setProvider(cat.key, (e.currentTarget as HTMLSelectElement).value)}
              >
                {#each cat.options as o (o.value)}
                  <option value={o.value}>{o.label}</option>
                {/each}
              </select>
              <Switch checked={enabled} onchange={() => toggle(cat.key, !enabled)} />
            </div>

            {#if cat.key === 'web_search' && meta && provider !== 'none'}
              <div class="ml-2 pl-3 border-l border-zinc-800 space-y-2 text-xs text-zinc-400">
                <div>{meta.description}</div>
                {#if meta.docs}
                  <div>Docs: <a href={meta.docs} target="_blank" rel="noreferrer" class="text-violet-300 hover:underline">{meta.docs}</a></div>
                {/if}
                {#if meta.needsKey}
                  <div class="flex items-center gap-2">
                    <Label for="ws-key-{cat.key}">API key</Label>
                    <Input
                      id="ws-key-{cat.key}"
                      type="password"
                      value={apiKeys[cat.key] ?? cfg.api_key ?? ''}
                      placeholder={meta.key === 'google' ? 'AIza…' : meta.key === 'bing' ? '<key>' : 'sk-…'}
                      oninput={(e) => setApiKey(cat.key, (e.currentTarget as HTMLInputElement).value)}
                      class="flex-1"
                    />
                  </div>
                {/if}
                {#if meta.needsEndpoint}
                  <div class="flex items-center gap-2">
                    <Label for="ws-ep-{cat.key}">Endpoint / config</Label>
                    <Input
                      id="ws-ep-{cat.key}"
                      value={endpoints[cat.key] ?? cfg.endpoint ?? ''}
                      placeholder={meta.endpointHint ?? ''}
                      oninput={(e) => setEndpoint(cat.key, (e.currentTarget as HTMLInputElement).value)}
                      class="flex-1"
                    />
                  </div>
                {/if}
                {#if savingKey === cat.key}
                  <span class="text-[10px] text-zinc-500">saving…</span>
                {/if}
              </div>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </Card>
</div>
