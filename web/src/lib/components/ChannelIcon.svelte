<script lang="ts">
  let { type, size = 16, class: cls = '' }: { type: string; size?: number; class?: string } = $props();
  // CleanClaw uses /public/channels/ for icons. The SvelteKit
  // static dir is /static/, so we map /channels/<filename>.
  const file = $derived(`${type}.svg`);
</script>

{#if type === 'feishu' || type === 'line' || type === 'wechat'}
  <img src={`/channels/${type}.png`} alt={type} width={size} height={size} class={cls} />
{:else}
  <img src={`/channels/${file}`} alt={type} width={size} height={size} class={cls} />
{/if}
