<script lang="ts">
  import { collapsibleContentClasses } from './classes';
  let { open = $bindable(false), children }: { open?: boolean; children?: any } = $props();
</script>

{#if open}
  <div class="{collapsibleContentClasses}">
    {@render children?.()}
  </div>
{/if}
