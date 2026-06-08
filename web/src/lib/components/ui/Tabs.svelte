<script lang="ts">
  import { tabsListClasses, tabsTriggerClasses } from './classes';
  let { value = $bindable(''), tabs, children }: {
    value?: string;
    tabs: { id: string; label: string }[];
    children?: any;
  } = $props();
</script>

<div class="space-y-4">
  <div class="{tabsListClasses}" role="tablist">
    {#each tabs as t}
      <button
        type="button"
        role="tab"
        aria-selected={value === t.id}
        data-state={value === t.id ? 'active' : 'inactive'}
        class="{tabsTriggerClasses}"
        onclick={() => (value = t.id)}
      >
        {t.label}
      </button>
    {/each}
  </div>
  {@render children?.()}
</div>
