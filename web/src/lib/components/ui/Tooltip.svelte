<script lang="ts">
  let { children, label = "Tooltip" }: { children?: any; label?: string } =
    $props();
  let visible = $state(false);
</script>

<span
  class="relative inline-flex"
  onmouseenter={() => (visible = true)}
  onmouseleave={() => (visible = false)}
  onfocusin={() => (visible = true)}
  onfocusout={() => (visible = false)}
  role="tooltip"
>
  {@render children?.()}
  {#if visible}
    <span
      class="absolute z-50 px-2 py-1 text-xs font-medium text-foreground bg-popover border border-border rounded shadow-md -top-8 left-1/2 -translate-x-1/2 whitespace-nowrap"
    >
      {label}
    </span>
  {/if}
</span>
