import { describe, it, expect } from 'vitest';
import { marked } from 'marked';

// Configure marked the same way as MarkdownLink.svelte
marked.setOptions({ gfm: true, breaks: true });

describe('Markdown rendering of web_search results', () => {
  it('renders numbered list with links (typical LLM output)', () => {
    const md =
      'Here are the results for "Rust programming":\n\n' +
      '1. **[Rust Programming Language](https://www.rust-lang.org)** - A language empowering everyone to build reliable and efficient software.\n' +
      '2. **[Learn Rust](https://doc.rust-lang.org/book/)** - The official Rust book.';
    const html = marked.parse(md, { async: false }) as string;

    // Should produce an ordered list
    expect(html).toContain('<ol>');
    expect(html).toContain('</ol>');
    // Should have links
    expect(html).toContain('href="https://www.rust-lang.org"');
    expect(html).toContain('href="https://doc.rust-lang.org/book/"');
    // Bold text preserved
    expect(html).toContain('<strong>');
    // Paragraph for intro text
    expect(html).toContain('<p>');
  });

  it('renders plain text search results (direct tool output)', () => {
    // This simulates what the raw tool result text looks like
    const md =
      'Search results for: Rust programming\n\n' +
      '1. Rust Programming Language\n' +
      '   https://www.rust-lang.org\n' +
      '   A language empowering everyone\n\n' +
      '2. Learn Rust\n' +
      '   https://doc.rust-lang.org/book/\n' +
      '   The official Rust book.';
    const html = marked.parse(md, { async: false }) as string;

    // With `breaks: true`, newlines become <br>
    expect(html).toContain('<br>');
    // URLs should be auto-linked by GFM
    expect(html).toContain('https://www.rust-lang.org');
    expect(html).toContain('https://doc.rust-lang.org/book/');
    // Search results text should be present
    expect(html).toContain('Search results for');
    expect(html).toContain('Rust Programming Language');
  });

  it('renders search results with code snippets', () => {
    const md =
      'Search results for "Rust match":\n\n' +
      '1. **Match in Rust** - [docs](https://doc.rust-lang.org/book/ch06-02-match.html)\n' +
      '```rust\n' +
      'match value {\n' +
      '    1 => println!("one"),\n' +
      '    _ => println!("other"),\n' +
      '}\n' +
      '```';
    const html = marked.parse(md, { async: false }) as string;

    expect(html).toContain('<pre><code class="language-rust">');
    expect(html).toContain('match value');
    expect(html).toContain('href="https://doc.rust-lang.org/book/ch06-02-match.html"');
    expect(html).toContain('<strong>');
  });

  it('renders empty/edge case content', () => {
    const html1 = marked.parse('', { async: false }) as string;
    expect(html1).toBe('');

    const html2 = marked.parse('No results found.', { async: false }) as string;
    expect(html2).toContain('No results found');
  });

  it('external links get proper target and rel attributes', () => {
    const md = 'See [Rust](https://www.rust-lang.org) for details.';
    let html = marked.parse(md, { async: false }) as string;

    // Simulate the MarkdownLink post-processing
    html = html.replace(
      /<a\s+([^>]*?)href="(https?:\/\/[^"]+)"([^>]*)>/g,
      (m, a1, href, a2) => {
        if (/rel=/.test(a1 + a2)) {
          return m
            .replace(/target="[^"]*"/g, '')
            .replace(/<a\s+/, '<a target="_blank" rel="noopener noreferrer" ');
        }
        return `<a ${a1} target="_blank" rel="noopener noreferrer" href="${href}"${a2}>`;
      },
    );

    expect(html).toContain('target="_blank"');
    expect(html).toContain('rel="noopener noreferrer"');
  });
});

describe('SSE tool_result event parsing', () => {
  it('parses web_search result from SSE data frame', () => {
    const frame =
      'event: tool_result\n' +
      'data: {"id":"tc1","result":"Search results for: Rust\\n\\n1. Rust Programming Language\\n   https://rust-lang.org\\n   A language.","is_error":false}\n\n';

    const evMatch = frame.match(/^event:\s*(\S+)/m);
    const dataMatch = frame.match(/^data:\s*(\{.*\})/m);

    expect(evMatch).not.toBeNull();
    expect(evMatch![1]).toBe('tool_result');

    expect(dataMatch).not.toBeNull();
    const data = JSON.parse(dataMatch![1]);
    expect(data.id).toBe('tc1');
    expect(data.result).toContain('Search results for: Rust');
    expect(data.result).toContain('Rust Programming Language');
    expect(data.is_error).toBe(false);
  });

  it('parses web_search result with error flag', () => {
    const frame =
      'event: tool_result\n' +
      'data: {"id":"tc2","result":"web_search: rate limit exceeded","is_error":true}\n\n';

    const dataMatch = frame.match(/^data:\s*(\{.*\})/m);
    const data = JSON.parse(dataMatch![1]);
    expect(data.is_error).toBe(true);
    expect(data.result).toContain('rate limit');
  });

  it('handles tool_call event with web_search arguments', () => {
    const frame =
      'event: tool_call\n' +
      'data: {"id":"tc1","name":"web_search","arguments":{"query":"Rust programming","limit":5}}\n\n';

    const evMatch = frame.match(/^event:\s*(\S+)/m);
    const dataMatch = frame.match(/^data:\s*(\{.*\})/m);

    expect(evMatch![1]).toBe('tool_call');
    const data = JSON.parse(dataMatch![1]);
    expect(data.name).toBe('web_search');
    expect(data.arguments.query).toBe('Rust programming');
    expect(data.arguments.limit).toBe(5);
  });
});

describe('ChatScreen tool result state management', () => {
  // Simulate the liveAssistant state updates that ChatScreen does

  function simulateToolResult(
    toolCalls: Array<{ id: string; name: string; arguments: string; result?: string }>,
    event: { id: string; result: string },
  ) {
    const tc = toolCalls.find((t) => t.id === event.id);
    if (tc) tc.result = event.result || '';
    return toolCalls;
  }

  it('matches tool_result to existing tool_call by id', () => {
    const toolCalls = [
      { id: 'tc1', name: 'web_search', arguments: '{"query":"test"}' },
      { id: 'tc2', name: 'web_fetch', arguments: '{"url":"https://example.com"}' },
    ];

    const result = simulateToolResult(toolCalls, {
      id: 'tc1',
      result: 'Search results for: test\n\n1. Result\n   https://example.com\n   Snippet',
    });

    expect(result[0].result).toContain('Search results for: test');
    expect(result[1].result).toBeUndefined();
  });

  it('does nothing when tool_call id not found', () => {
    const toolCalls = [
      { id: 'tc1', name: 'web_search', arguments: '{}' },
    ];

    const result = simulateToolResult(toolCalls, {
      id: 'unknown_id',
      result: 'some result',
    });

    expect(result[0].result).toBeUndefined();
  });

  it('stores result as plain text string', () => {
    const toolCalls = [
      { id: 'tc1', name: 'web_search', arguments: '{"query":"rust"}' },
    ];

    const result = simulateToolResult(toolCalls, {
      id: 'tc1',
      result: JSON.stringify({ results: 'Search results for: rust\n\n1. Title\n   url\n   snippet' }),
    });

    // The result should contain the raw JSON string as-is
    expect(result[0].result).toContain('Search results for: rust');
    expect(result[0].result).toContain('"results"');
  });
});
