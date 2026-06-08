<script lang="ts">
  let { scope = 'user', id = '', allowAgent = false, allowSystem = false, value = $bindable('user'), onchange }: {
    scope?: 'user' | 'agent' | 'system';
    id?: string;
    allowAgent?: boolean;
    allowSystem?: boolean;
    value?: string;
    onchange?: (scope: string) => void;
  } = $props();

  const options = [
    { value: 'user', label: 'User' },
    ...(allowAgent ? [{ value: 'agent', label: 'Agent' }] : []),
    ...(allowSystem ? [{ value: 'system', label: 'System' }] : [])
  ];

  function pick(v: string) {
    value = v;
    onchange?.(v);
  }
</script>

<div class="inline-flex rounded-md border border-input bg-muted p-0.5 text-xs">
  {#each options as opt}
    <button
      type="button"
      class="px-2 py-1 rounded-sm {value === opt.value ? 'bg-background text-foreground shadow' : 'text-muted-foreground hover:text-foreground'}"
      onclick={() => pick(opt.value)}
    >
      {opt.label}
    </button>
  {/each}
</div>
