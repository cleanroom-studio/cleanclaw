<script lang="ts">
  import { separatorClasses } from "./classes";
  let { class: cls = "" }: { class?: string } = $props();
</script>

<div class="{separatorClasses} {cls}" role="separator"></div>
