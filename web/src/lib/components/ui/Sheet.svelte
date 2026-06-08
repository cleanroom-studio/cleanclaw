<script lang="ts">
  import {
    sheetOverlayClasses,
    sheetContentClasses,
    sheetContentRight,
  } from "./classes";
  let {
    open = $bindable(false),
    title = "",
    children,
  }: { open?: boolean; title?: string; children?: any } = $props();
  function close() {
    open = false;
  }
</script>

{#if open}
  <div class={sheetOverlayClasses} onclick={close} role="presentation"></div>
  <div
    class="{sheetContentClasses} {sheetContentRight}"
    role="dialog"
    aria-modal="true"
  >
    {#if title}<h2 class="text-lg font-semibold">{title}</h2>{/if}
    {@render children?.()}
    <button
      type="button"
      class="absolute right-4 top-4 rounded-sm opacity-70 hover:opacity-100"
      onclick={close}
      aria-label="Close">×</button
    >
  </div>
{/if}
