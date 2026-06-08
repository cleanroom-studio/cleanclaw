<script lang="ts">
  // MarkdownLink — minimal Markdown renderer for assistant
  // bubbles.
  // (which is the `ExternalAnchor` wrapper around react-markdown).
//
  // Svelte version: uses the `marked` library (already in
  // `web/package.json` deps) to convert MD → HTML, then
  // post-processes to make links target="_blank" and
  // rel="noopener noreferrer".
//
  // For the full dashboard we wire up `prose prose-invert`
  // classes (Tailwind) so headings, code blocks, lists, and
  // tables render with the same look as CleanClaw.

  import { marked } from 'marked';

  let { content }: { content: string } = $props();

  // Configure marked once: GFM, breaks on, no mangle, headerIds
  // off (so we don't ship `id="user-content-…"` everywhere).
  marked.setOptions({ gfm: true, breaks: true });

  const html = $derived(
    (() => {
      try {
        let out = marked.parse(content || '', { async: false }) as string;
        // Force external links to open in a new tab.
        out = out.replace(
          /<a\s+([^>]*?)href="(https?:\/\/[^"]+)"([^>]*)>/g,
          (m, a1, href, a2) => {
            // Skip if rel already set
            if (/rel=/.test(a1 + a2)) {
              return m.replace(/target="[^"]*"/g, '').replace(/<a\s+/, '<a target="_blank" rel="noopener noreferrer" ');
            }
            return `<a ${a1} target="_blank" rel="noopener noreferrer" href="${href}"${a2}>`;
          },
        );
        return out;
      } catch {
        return '';
      }
    })(),
  );
</script>

<!-- eslint-disable-next-line svelte/no-at-html-tags -->
<div class="markdown-body">{@html html}</div>

<style>
  :global(.markdown-body p) { margin: 0.25em 0; }
  :global(.markdown-body pre) {
    background: rgb(24 24 27 / 0.6);
    border: 1px solid rgb(39 39 42);
    border-radius: 6px;
    padding: 0.5em 0.75em;
    overflow-x: auto;
    font-size: 0.85em;
  }
  :global(.markdown-body code) {
    background: rgb(39 39 42);
    padding: 0.05em 0.3em;
    border-radius: 4px;
    font-size: 0.9em;
  }
  :global(.markdown-body pre code) { background: transparent; padding: 0; }
  :global(.markdown-body ul, .markdown-body ol) { margin: 0.25em 0 0.25em 1.25em; }
  :global(.markdown-body li) { margin: 0.1em 0; }
  :global(.markdown-body h1, .markdown-body h2, .markdown-body h3, .markdown-body h4) {
    font-weight: 600;
    margin: 0.5em 0 0.25em;
  }
  :global(.markdown-body h1) { font-size: 1.15em; }
  :global(.markdown-body h2) { font-size: 1.05em; }
  :global(.markdown-body h3) { font-size: 0.95em; }
  :global(.markdown-body a) { color: rgb(167 139 250); text-decoration: underline; }
  :global(.markdown-body table) { border-collapse: collapse; margin: 0.25em 0; }
  :global(.markdown-body th, .markdown-body td) {
    border: 1px solid rgb(39 39 42);
    padding: 0.2em 0.5em;
  }
  :global(.markdown-body blockquote) {
    border-left: 2px solid rgb(82 82 91);
    padding-left: 0.6em;
    color: rgb(161 161 170);
    margin: 0.25em 0;
  }
</style>
