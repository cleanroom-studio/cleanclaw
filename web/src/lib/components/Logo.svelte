<script lang="ts">
  // Logo — CleanClaw wordmark, used in the AppShell header. Tiny
  // mark; mirrors the CleanClaw `<Logo />` SVG.
  let { size = 20 }: { size?: number } = $props();
</script>

<svg
  xmlns="http://www.w3.org/2000/svg"
  viewBox="0 0 24 24"
  width={size}
  height={size}
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
  class="text-violet-400"
>
  <path d="M3 3h6v6H3z" />
  <path d="M15 3h6v6h-6z" />
  <path d="M3 15h6v6H3z" />
  <path d="M15 15h6v6h-6z" />
  <path d="M9 6h6" />
  <path d="M6 9v6" />
  <path d="M18 9v6" />
  <path d="M9 18h6" />
</svg>
