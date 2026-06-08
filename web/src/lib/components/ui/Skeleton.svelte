<script lang="ts">
  import { skeletonClasses } from "./classes";
  let { class: cls = "" }: { class?: string } = $props();
</script>

<div class="{skeletonClasses} {cls}"></div>
