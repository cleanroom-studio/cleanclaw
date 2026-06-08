<script lang="ts">
  import { selectTriggerClasses, selectContentClasses, selectItemClasses } from './classes';
  let {
    value = $bindable(''),
    options,
    placeholder = 'Select…',
    class: cls = ''
  }: {
    value?: string;
    options: { value: string; label: string }[];
    placeholder?: string;
    class?: string;
  } = $props();
  let open = $state(false);

  function pick(v: string) {
    value = v;
    open = false;
  }
  const currentLabel = $derived(options.find((o) => o.value === value)?.label ?? placeholder);
</script>

<div class="relative {cls}">
  <button
    type="button"
    class="{selectTriggerClasses}"
    aria-haspopup="listbox"
    aria-expanded={open}
    onclick={() => (open = !open)}
  >
    <span>{currentLabel}</span>
    <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="h-4 w-4 opacity-50"><polyline points="6 9 12 15 18 9"></polyline></svg>
  </button>
  {#if open}
    <ul class="{selectContentClasses}" role="listbox">
      {#each options as opt}
        <li
          class="{selectItemClasses}"
          role="option"
          aria-selected={value === opt.value}
          onclick={() => pick(opt.value)}
        >
          {opt.label}
        </li>
      {/each}
    </ul>
  {/if}
</div>
