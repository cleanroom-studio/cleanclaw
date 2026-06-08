<script lang="ts">
  import { dialogOverlayClasses, dialogContentClasses } from "./classes";
  let {
    open = $bindable(false),
    title = "",
    description = "",
    children,
    footer,
  }: {
    open?: boolean;
    title?: string;
    description?: string;
    children?: any;
    footer?: any;
  } = $props();

  function close() {
    open = false;
  }
</script>

{#if open}
  <div
    class={dialogOverlayClasses}
    role="presentation"
    onclick={close}
    onkeydown={(e) => {
      if (e.key === "Escape") close();
    }}
  ></div>
  <div
    class={dialogContentClasses}
    role="dialog"
    aria-modal="true"
    aria-labelledby={title ? "dialog-title" : undefined}
  >
    {#if title}
      <h2
        id="dialog-title"
        class="text-lg font-semibold leading-none tracking-tight"
      >
        {title}
      </h2>
    {/if}
    {#if description}
      <p class="text-sm text-muted-foreground">{description}</p>
    {/if}
    <div class="space-y-3">
      {@render children?.()}
    </div>
    {#if footer}
      <div
        class="flex flex-col-reverse sm:flex-row sm:justify-end sm:space-x-2"
      >
        {@render footer?.()}
      </div>
    {/if}
    <button
      type="button"
      class="absolute right-4 top-4 rounded-sm opacity-70 ring-offset-background transition-opacity hover:opacity-100 focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2"
      onclick={close}
      aria-label="Close"
    >
      <svg
        xmlns="http://www.w3.org/2000/svg"
        width="16"
        height="16"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        stroke-linejoin="round"
        class="h-4 w-4"
        ><line x1="18" y1="6" x2="6" y2="18"></line><line
          x1="6"
          y1="6"
          x2="18"
          y2="18"
        ></line></svg
      >
    </button>
  </div>
{/if}
