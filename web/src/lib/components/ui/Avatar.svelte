<script lang="ts">
  import { avatarClasses } from './classes';
  let { name = '', src = '', size = 36 }: { name?: string; src?: string; size?: number } = $props();
  const initials = $derived(
    name
      .split(/\s+/)
      .map((p) => p[0] || '')
      .join('')
      .slice(0, 2)
      .toUpperCase() || '?'
  );
</script>

<div
  class="{avatarClasses} items-center justify-center bg-muted text-xs font-medium uppercase"
  style="height: {size}px; width: {size}px;"
  aria-label={name}
>
  {#if src}
    <img {src} alt={name} class="h-full w-full object-cover" />
  {:else}
    {initials}
  {/if}
</div>
