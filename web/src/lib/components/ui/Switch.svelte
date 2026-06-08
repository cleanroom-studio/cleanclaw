<script lang="ts">
  import { switchClasses } from "./classes";
  let {
    checked = $bindable(false),
    disabled = false,
    class: cls = "",
    onchange,
  }: {
    checked?: boolean;
    disabled?: boolean;
    class?: string;
    onchange?: (e: Event) => void;
  } = $props();

  function toggle() {
    if (disabled) return;
    checked = !checked;
  }
</script>

<button
  type="button"
  role="switch"
  aria-checked={checked}
  {disabled}
  data-state={checked ? "checked" : "unchecked"}
  class="{switchClasses} {cls}"
  onclick={toggle}
></button>
