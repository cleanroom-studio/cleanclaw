<script lang="ts">
  import { tableClasses, theadClasses, thClasses, tdClasses } from './classes';
  let { children, class: cls = '' }: { children?: any; class?: string } = $props();
</script>

<div class="relative w-full overflow-auto">
  <table class="{tableClasses} {cls}">
    {@render children?.()}
  </table>
</div>
