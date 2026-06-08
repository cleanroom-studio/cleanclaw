<script lang="ts">
  import { dropdownContentClasses, dropdownItemClasses } from './classes';
  let { open = $bindable(false), children, trigger }: { open?: boolean; children?: any; trigger?: any } = $props();
</script>

<div class="relative inline-block">
  {#if trigger}
    <span onclick={() => (open = !open)}>
      {@render trigger?.()}
    </span>
  {/if}
  {#if open}
    <div class="{dropdownContentClasses}" role="menu">
      {@render children?.()}
    </div>
  {/if}
</div>

{#snippet item(label: string, onclick: (e: MouseEvent) => void)}
  <div class="{dropdownItemClasses}" role="menuitem" onclick={onclick}>{label}</div>
{/snippet}
