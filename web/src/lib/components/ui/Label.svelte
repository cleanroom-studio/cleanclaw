<script lang="ts">
  import { labelClasses } from './classes';
  let { children, class: cls = '', for: forId }: { children?: any; class?: string; for?: string } = $props();
</script>

<label for={forId} class="{labelClasses} {cls}">
  {@render children?.()}
</label>
