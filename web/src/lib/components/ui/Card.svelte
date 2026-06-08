<script lang="ts">
  import { cardClasses } from './classes';
  let { children, class: cls = '' }: { children?: any; class?: string } = $props();
</script>

<div class="{cardClasses} {cls}">
  {@render children?.()}
</div>
