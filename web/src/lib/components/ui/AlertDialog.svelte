<script lang="ts">
  import { alertDialogContentClasses } from './classes';
  let { open = $bindable(false), title = '', description = '', onConfirm, onCancel, confirmLabel = 'Confirm', cancelLabel = 'Cancel', children }: {
    open?: boolean;
    title?: string;
    description?: string;
    onConfirm?: () => void;
    onCancel?: () => void;
    confirmLabel?: string;
    cancelLabel?: string;
    children?: any;
  } = $props();
  function cancel() { onCancel?.(); open = false; }
  function confirm() { onConfirm?.(); open = false; }
</script>

{#if open}
  <div class="fixed inset-0 z-50 bg-black/80" onclick={cancel} role="presentation"></div>
  <div class="{alertDialogContentClasses}" role="alertdialog" aria-modal="true">
    {#if title}<h2 class="text-lg font-semibold">{title}</h2>{/if}
    {#if description}<p class="text-sm text-muted-foreground">{description}</p>{/if}
    {@render children?.()}
    <div class="flex flex-col-reverse sm:flex-row sm:justify-end sm:space-x-2">
      <button type="button" class="h-9 px-4 rounded-md border border-input text-sm" onclick={cancel}>{cancelLabel}</button>
      <button type="button" class="h-9 px-4 rounded-md bg-destructive text-destructive-foreground text-sm" onclick={confirm}>{confirmLabel}</button>
    </div>
  </div>
{/if}
