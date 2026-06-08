<script lang="ts">
  import { badgeClasses } from './classes';
  type Variant = 'default' | 'secondary' | 'destructive' | 'outline' | 'success';
  let {
    variant = 'default',
    class: cls = '',
    children
  }: {
    variant?: Variant;
    class?: string;
    children?: any;
  } = $props();

  const variantClass = $derived(
    {
      default: 'border-transparent bg-primary text-primary-foreground',
      secondary: 'border-transparent bg-secondary text-secondary-foreground',
      destructive: 'border-transparent bg-destructive text-destructive-foreground',
      outline: 'text-foreground',
      success: 'border-transparent bg-green-600 text-white'
    }[variant]
  );
</script>

<span class="{badgeClasses} {variantClass} {cls}">
  {@render children?.()}
</span>
