import { describe, it, expect } from 'vitest';

// Provider metadata (extracted from the tools page for testing)
type ProviderKey =
  | 'duckduckgo' | 'brave' | 'bing' | 'google'
  | 'baidu' | 'searxng' | 'exa' | 'none';

type ProviderMeta = {
  key: ProviderKey;
  label: string;
  needsKey: boolean;
  needsEndpoint: boolean;
};

const WEB_SEARCH_PROVIDERS: (ProviderMeta & { docs: string; description: string; endpointHint?: string })[] = [
  { key: 'duckduckgo', label: 'DuckDuckGo', description: 'Free. No key. HTML scrape.', docs: 'https://duckduckgo.com', needsKey: false, needsEndpoint: false },
  { key: 'brave', label: 'Brave Search', description: 'Independent index. 2000 free queries / month.', docs: 'https://brave.com/search/api', needsKey: true, needsEndpoint: false },
  { key: 'bing', label: 'Microsoft Bing', description: 'Web Search API v7.', docs: 'https://learn.microsoft.com/en-us/bing/search-apis/', needsKey: true, needsEndpoint: false },
  { key: 'google', label: 'Google CSE', description: 'Programmable Search Engine.', docs: 'https://developers.google.com/custom-search', needsKey: true, needsEndpoint: true, endpointHint: 'cx=<engine-id>' },
  { key: 'baidu', label: 'Baidu 百度', description: 'Free. No key. HTML scrape.', docs: 'https://www.baidu.com', needsKey: false, needsEndpoint: false },
  { key: 'searxng', label: 'SearXNG', description: 'Meta-search. Self-hosted instance URL required.', docs: 'https://searxng.org', needsKey: false, needsEndpoint: true, endpointHint: 'https://searx.example.com' },
  { key: 'exa', label: 'Exa', description: 'Neural search. 1000 free / month.', docs: 'https://exa.ai', needsKey: true, needsEndpoint: false },
  { key: 'none', label: 'Disabled (none)', description: 'Disable web search entirely.', docs: '', needsKey: false, needsEndpoint: false },
];

function providerMeta(category: string, key: string): ProviderMeta | undefined {
  if (category !== 'web_search') return undefined;
  return WEB_SEARCH_PROVIDERS.find((p) => p.key === key);
}

describe('WEB_SEARCH_PROVIDERS', () => {
  it('contains all 8 providers', () => {
    expect(WEB_SEARCH_PROVIDERS).toHaveLength(8);
  });

  it('includes duckduckgo as credential-free default', () => {
    const ddg = WEB_SEARCH_PROVIDERS.find((p) => p.key === 'duckduckgo');
    expect(ddg).toBeDefined();
    expect(ddg!.needsKey).toBe(false);
    expect(ddg!.needsEndpoint).toBe(false);
  });

  it('includes none (disabled) provider', () => {
    const none = WEB_SEARCH_PROVIDERS.find((p) => p.key === 'none');
    expect(none).toBeDefined();
    expect(none!.label).toContain('Disabled');
    expect(none!.docs).toBe('');
  });

  it('providers needing endpoint have endpointHint', () => {
    for (const p of WEB_SEARCH_PROVIDERS) {
      if (p.needsEndpoint) {
        expect(p.endpointHint).toBeTruthy();
      }
    }
  });
});

describe('providerMeta', () => {
  it('returns metadata for web_search providers', () => {
    const meta = providerMeta('web_search', 'brave');
    expect(meta).toBeDefined();
    expect(meta!.key).toBe('brave');
    expect(meta!.needsKey).toBe(true);
  });

  it('returns undefined for non-web_search categories', () => {
    expect(providerMeta('image_gen', 'openai')).toBeUndefined();
    expect(providerMeta('tts', 'elevenlabs')).toBeUndefined();
  });

  it('returns undefined for unknown provider key', () => {
    expect(providerMeta('web_search', 'unknown')).toBeUndefined();
  });

  it('finds duckduckgo as credential-free', () => {
    const meta = providerMeta('web_search', 'duckduckgo');
    expect(meta).toBeDefined();
    expect(meta!.needsKey).toBe(false);
  });

  it('handles none provider', () => {
    const meta = providerMeta('web_search', 'none');
    expect(meta).toBeDefined();
    expect(meta!.key).toBe('none');
  });
});

describe('provider categorization', () => {
  it('identifies credential-free providers', () => {
    const freeProviders = WEB_SEARCH_PROVIDERS
      .filter((p) => !p.needsKey)
      .map((p) => p.key);
    expect(freeProviders).toContain('duckduckgo');
    expect(freeProviders).toContain('baidu');
    expect(freeProviders).toContain('searxng');
    expect(freeProviders).toContain('none');
  });

  it('identifies API-key-required providers', () => {
    const keyProviders = WEB_SEARCH_PROVIDERS
      .filter((p) => p.needsKey)
      .map((p) => p.key);
    expect(keyProviders).toContain('brave');
    expect(keyProviders).toContain('bing');
    expect(keyProviders).toContain('google');
    expect(keyProviders).toContain('exa');
  });

  it('identifies endpoint-required providers', () => {
    const epProviders = WEB_SEARCH_PROVIDERS
      .filter((p) => p.needsEndpoint)
      .map((p) => p.key);
    expect(epProviders).toContain('google');
    expect(epProviders).toContain('searxng');
  });

  it('all providers have unique keys', () => {
    const keys = WEB_SEARCH_PROVIDERS.map((p) => p.key);
    expect(new Set(keys).size).toBe(keys.length);
  });
});
