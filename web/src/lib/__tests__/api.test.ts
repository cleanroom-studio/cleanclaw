import { describe, it, expect, vi, beforeEach } from 'vitest';
import { listTools, saveTools } from '../api';

const mockFetch = vi.fn();
globalThis.fetch = mockFetch;

beforeEach(() => {
  vi.clearAllMocks();
});

describe('listTools', () => {
  it('returns tools from GET /api/tools', async () => {
    const response = {
      tools: {
        web_search: { enabled: true, provider: 'duckduckgo' },
        webfetch: { enabled: true, provider: 'direct' },
      },
    };
    mockFetch.mockResolvedValueOnce({
      ok: true,
      headers: new Map([['content-type', 'application/json']]),
      json: async () => response,
    });

    const result = await listTools();
    expect(result).toEqual(response);
    expect(mockFetch).toHaveBeenCalledWith('/api/tools', expect.objectContaining({
      credentials: 'same-origin',
    }));
  });

  it('throws on non-ok response', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: false,
      status: 500,
      statusText: 'Internal Server Error',
      headers: new Map([['content-type', 'text/plain']]),
      text: async () => 'Server Error',
    });

    await expect(listTools()).rejects.toThrow();
  });

  it('handles empty tools config', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: true,
      headers: new Map([['content-type', 'application/json']]),
      json: async () => ({ tools: {} }),
    });

    const result = await listTools();
    expect(result.tools).toEqual({});
  });
});

describe('saveTools', () => {
  it('sends PUT /api/tools with config', async () => {
    const toolsConfig = {
      web_search: { enabled: true, provider: 'brave' },
    };
    mockFetch.mockResolvedValueOnce({
      ok: true,
      headers: new Map([['content-type', 'application/json']]),
      json: async () => ({ ok: true }),
    });

    const result = await saveTools(toolsConfig);
    expect(result).toEqual({ ok: true });
    expect(mockFetch).toHaveBeenCalledWith('/api/tools', expect.objectContaining({
      method: 'PUT',
      headers: new Headers({ 'Content-Type': 'application/json' }),
      body: JSON.stringify({ tools: toolsConfig }),
    }));
  });

  it('throws on error response', async () => {
    mockFetch.mockResolvedValueOnce({
      ok: false,
      status: 400,
      headers: new Map([['content-type', 'application/json']]),
      json: async () => ({ error: { message: 'invalid config' } }),
      text: async () => 'invalid config',
    });

    await expect(saveTools({})).rejects.toThrow('invalid config');
  });

  it('preserves api_key and endpoint in saved config', async () => {
    const toolsConfig = {
      web_search: {
        enabled: true,
        provider: 'bing',
        api_key: 'sk-test',
        endpoint: '',
      },
    };
    mockFetch.mockResolvedValueOnce({
      ok: true,
      headers: new Map([['content-type', 'application/json']]),
      json: async () => ({ ok: true }),
    });

    await saveTools(toolsConfig);
    const callBody = JSON.parse(mockFetch.mock.calls[0][1].body);
    expect(callBody.tools.web_search.api_key).toBe('sk-test');
  });
});
