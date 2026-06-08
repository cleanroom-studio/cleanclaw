<script lang="ts">
  import { buttonClasses } from "./classes";
  type Variant = "default" | "ghost" | "destructive" | "outline";
  type Size = "sm" | "md";
  let {
    variant = "default",
    size = "md",
    type = "button",
    class: cls = "",
    children,
    onclick,
    disabled = false,
    ...rest
  }: {
    variant?: Variant;
    size?: Size;
    type?: "button" | "submit" | "reset";
    class?: string;
    children?: any;
    onclick?: (e: MouseEvent) => void;
    disabled?: boolean;
    [key: string]: any;
  } = $props();

  const variantClass = $derived(
    {
      default: "bg-primary text-primary-foreground",
      ghost: "bg-transparent text-foreground hover:bg-accent",
      destructive: "bg-destructive text-destructive-foreground",
      outline: "border border-input bg-transparent hover:bg-accent",
    }[variant],
  );
  const sizeClass = $derived(
    size === "sm" ? "h-8 px-3 text-xs" : "h-9 px-4 text-sm",
  );
</script>

<button
  {type}
  {disabled}
  {onclick}
  class="{buttonClasses} {variantClass} {sizeClass} {cls}"
  {...rest}
>
  {@render children?.()}
</button>
