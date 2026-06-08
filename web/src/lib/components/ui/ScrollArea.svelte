<script lang="ts">
  import { scrollAreaClasses } from "./classes";
  let {
    children,
    class: cls = "",
    maxHeight = "24rem",
  }: {
    children?: any;
    class?: string;
    maxHeight?: string;
  } = $props();
</script>

<div class="{scrollAreaClasses} {cls}" style="max-height: {maxHeight};">
  {@render children?.()}
</div>
