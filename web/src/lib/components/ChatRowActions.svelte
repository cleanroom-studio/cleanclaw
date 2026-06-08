<script lang="ts">
  // ChatRowActions — copy / regenerate / delete buttons on
  // assistant bubbles. Mirrors
  // .
  // (Compressed to a small wrapper; the chat surface itself
  // listens to click events rather than passing a ref.)

  let { content, onCopy, onRegenerate, onDelete }: {
    content: string;
    onCopy?: () => void;
    onRegenerate?: () => void;
    onDelete?: () => void;
  } = $props();

  let copied = $state(false);

  async function copy() {
    try {
      await navigator.clipboard.writeText(content);
      copied = true;
      setTimeout(() => (copied = false), 1500);
      onCopy?.();
    } catch {}
  }
</script>

<div class="flex items-center gap-1 text-xs">
  <button type="button" class="px-1.5 py-0.5 rounded hover:bg-zinc-800" onclick={copy}>
    {copied ? '✓' : '⧉'}
  </button>
  {#if onRegenerate}
    <button type="button" class="px-1.5 py-0.5 rounded hover:bg-zinc-800" onclick={onRegenerate} title="Regenerate">↻</button>
  {/if}
  {#if onDelete}
    <button type="button" class="px-1.5 py-0.5 rounded hover:bg-zinc-800" onclick={onDelete} title="Delete">🗑</button>
  {/if}
</div>
