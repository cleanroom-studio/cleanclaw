<script lang="ts">
  import { inputClasses } from "./classes";
  let {
    type = "text",
    value = $bindable(""),
    placeholder = "",
    class: cls = "",
    ...rest
  }: {
    type?: string;
    value?: string;
    placeholder?: string;
    class?: string;
    [key: string]: any;
  } = $props();
</script>

<input {type} bind:value {placeholder} class="{inputClasses} {cls}" {...rest} />
