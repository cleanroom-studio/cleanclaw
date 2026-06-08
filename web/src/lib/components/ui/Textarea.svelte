<script lang="ts">
  import { textareaClasses } from './classes';
  let {
    value = $bindable(''),
    placeholder = '',
    rows = 3,
    class: cls = '',
    ...rest
  }: {
    value?: string;
    placeholder?: string;
    rows?: number;
    class?: string;
    [key: string]: any;
  } = $props();
</script>

<textarea
  bind:value
  {placeholder}
  {rows}
  class="{textareaClasses} {cls}"
  {...rest}
></textarea>
