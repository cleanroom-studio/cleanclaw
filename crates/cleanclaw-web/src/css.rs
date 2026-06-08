//! Inline `base.css` derived from .
//! The shadcn / Tailwind design tokens are flattened to a single CSS
//! block that gets embedded into every page response.
//!
//! The hand-written set covers everything the W1 helpers + layout shell
//! use. Later phases can grow it as new pages introduce new utility
//! classes.

/// Inline stylesheet. Mirrors the `:root` token block from
///  (light theme by default,
/// overridden in `.dark`). All shadcn utility classes referenced by
/// `html.rs` and `layout.rs` are defined here.
pub const BASE_CSS: &str = r#"
:root {
  --background: hsl(0 0% 100%);
  --foreground: hsl(240 10% 3.9%);
  --card: hsl(0 0% 100%);
  --card-foreground: hsl(240 10% 3.9%);
  --popover: hsl(0 0% 100%);
  --popover-foreground: hsl(240 10% 3.9%);
  --primary: hsl(240 5.9% 10%);
  --primary-foreground: hsl(0 0% 98%);
  --secondary: hsl(240 4.8% 95.9%);
  --secondary-foreground: hsl(240 5.9% 10%);
  --muted: hsl(240 4.8% 95.9%);
  --muted-foreground: hsl(240 3.8% 46.1%);
  --accent: hsl(240 4.8% 95.9%);
  --accent-foreground: hsl(240 5.9% 10%);
  --destructive: hsl(0 84.2% 60.2%);
  --destructive-foreground: hsl(0 0% 98%);
  --border: hsl(240 5.9% 90%);
  --input: hsl(240 5.9% 90%);
  --ring: hsl(240 5.9% 10%);
  --radius: 0.5rem;
  --sidebar: hsl(0 0% 98%);
  --sidebar-foreground: hsl(240 5.3% 26.1%);
  --sidebar-primary: hsl(240 5.9% 10%);
  --sidebar-primary-foreground: hsl(0 0% 98%);
  --sidebar-accent: hsl(240 4.8% 95.9%);
  --sidebar-accent-foreground: hsl(240 5.9% 10%);
  --sidebar-border: hsl(220 13% 91%);
  --sidebar-ring: hsl(217.2 91.2% 59.8%);
  --font-sans: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
  --font-mono: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace;
}
.dark {
  --background: hsl(240 10% 3.9%);
  --foreground: hsl(0 0% 98%);
  --card: hsl(240 10% 3.9%);
  --card-foreground: hsl(0 0% 98%);
  --popover: hsl(240 10% 3.9%);
  --popover-foreground: hsl(0 0% 98%);
  --primary: hsl(0 0% 98%);
  --primary-foreground: hsl(240 5.9% 10%);
  --secondary: hsl(240 3.7% 15.9%);
  --secondary-foreground: hsl(0 0% 98%);
  --muted: hsl(240 3.7% 15.9%);
  --muted-foreground: hsl(240 5% 64.9%);
  --accent: hsl(240 3.7% 15.9%);
  --accent-foreground: hsl(0 0% 98%);
  --destructive: hsl(0 62.8% 30.6%);
  --destructive-foreground: hsl(0 0% 98%);
  --border: hsl(240 3.7% 15.9%);
  --input: hsl(240 3.7% 15.9%);
  --ring: hsl(240 4.9% 83.9%);
  --sidebar: hsl(240 5.9% 10%);
  --sidebar-foreground: hsl(240 4.8% 95.9%);
  --sidebar-primary: hsl(0 0% 98%);
  --sidebar-primary-foreground: hsl(240 5.9% 10%);
  --sidebar-accent: hsl(240 3.7% 15.9%);
  --sidebar-accent-foreground: hsl(240 4.8% 95.9%);
  --sidebar-border: hsl(240 3.7% 15.9%);
  --sidebar-ring: hsl(217.2 91.2% 59.8%);
}
* { box-sizing: border-box; border: 0 solid var(--border); }
html, body { margin: 0; padding: 0; }
body { font-family: var(--font-sans); }
.bg-background { background-color: var(--background); }
.text-foreground { color: var(--foreground); }
.bg-card { background-color: var(--card); }
.text-card-foreground { color: var(--card-foreground); }
.bg-muted { background-color: var(--muted); }
.text-muted-foreground { color: var(--muted-foreground); }
.bg-primary { background-color: var(--primary); }
.text-primary { color: var(--primary); }
.text-primary-foreground { color: var(--primary-foreground); }
.bg-secondary { background-color: var(--secondary); }
.text-secondary-foreground { color: var(--secondary-foreground); }
.bg-accent { background-color: var(--accent); }
.text-accent-foreground { color: var(--accent-foreground); }
.bg-destructive { background-color: var(--destructive); }
.text-destructive { color: var(--destructive); }
.text-destructive-foreground { color: var(--destructive-foreground); }
.border { border-width: 1px; }
.border-input { border-color: var(--input); }
.border-destructive\/50 { border-color: hsl(0 84.2% 60.2% / 0.5); }
.bg-border { background-color: var(--border); }
.bg-input { background-color: var(--input); }
.bg-destructive\/10 { background-color: hsl(0 84.2% 60.2% / 0.1); }
.rounded-md { border-radius: var(--radius); }
.rounded-lg { border-radius: calc(var(--radius) + 2px); }
.shadow-sm { box-shadow: 0 1px 2px 0 rgb(0 0 0 / 0.05); }
.shadow { box-shadow: 0 1px 3px 0 rgb(0 0 0 / 0.1), 0 1px 2px -1px rgb(0 0 0 / 0.1); }
.shadow-lg { box-shadow: 0 10px 15px -3px rgb(0 0 0 / 0.1), 0 4px 6px -4px rgb(0 0 0 / 0.1); }
.h-px { height: 1px; }
.w-px { width: 1px; }
.h-full { height: 100%; }
.w-full { width: 100%; }
.h-1 { height: 0.25rem; }
.h-2 { height: 0.5rem; }
.h-3 { height: 0.75rem; }
.h-4 { height: 1rem; }
.h-5 { height: 1.25rem; }
.h-6 { height: 1.5rem; }
.h-7 { height: 1.75rem; }
.h-8 { height: 2rem; }
.h-9 { height: 2.25rem; }
.h-10 { height: 2.5rem; }
.h-12 { height: 3rem; }
.w-1 { width: 0.25rem; }
.w-2 { width: 0.5rem; }
.w-3 { width: 0.75rem; }
.w-4 { width: 1rem; }
.w-5 { width: 1.25rem; }
.w-6 { width: 1.5rem; }
.w-7 { width: 1.75rem; }
.w-8 { width: 2rem; }
.w-9 { width: 2.25rem; }
.w-10 { width: 2.5rem; }
.w-12 { width: 3rem; }
.size-9 { width: 2.25rem; height: 2.25rem; }
.min-h-\[60px\] { min-height: 60px; }
.min-h-screen { min-height: 100vh; }
.max-w-lg { max-width: 32rem; }
.p-2 { padding: 0.5rem; }
.p-3 { padding: 0.75rem; }
.p-4 { padding: 1rem; }
.p-6 { padding: 1.5rem; }
.px-2 { padding-left: 0.5rem; padding-right: 0.5rem; }
.px-3 { padding-left: 0.75rem; padding-right: 0.75rem; }
.px-4 { padding-left: 1rem; padding-right: 1rem; }
.px-6 { padding-left: 1.5rem; padding-right: 1.5rem; }
.py-1 { padding-top: 0.25rem; padding-bottom: 0.25rem; }
.py-2 { padding-top: 0.5rem; padding-bottom: 0.5rem; }
.py-0\.5 { padding-top: 0.125rem; padding-bottom: 0.125rem; }
.py-3 { padding-top: 0.75rem; padding-bottom: 0.75rem; }
.pt-0 { padding-top: 0; }
.m-0 { margin: 0; }
.mb-1 { margin-bottom: 0.25rem; }
.mb-2 { margin-bottom: 0.5rem; }
.mb-4 { margin-bottom: 1rem; }
.mt-1 { margin-top: 0.25rem; }
.mt-2 { margin-top: 0.5rem; }
.mt-4 { margin-top: 1rem; }
.ml-2 { margin-left: 0.5rem; }
.ml-4 { margin-left: 1rem; }
.mr-2 { margin-right: 0.5rem; }
.gap-1 { gap: 0.25rem; }
.gap-2 { gap: 0.5rem; }
.gap-4 { gap: 1rem; }
.space-y-1 > * + * { margin-top: 0.25rem; }
.space-y-2 > * + * { margin-top: 0.5rem; }
.space-y-1\.5 > * + * { margin-top: 0.375rem; }
.flex { display: flex; }
.inline-flex { display: inline-flex; }
.grid { display: grid; }
.hidden { display: none; }
.block { display: block; }
.inline-block { display: inline-block; }
.items-center { align-items: center; }
.items-start { align-items: flex-start; }
.justify-center { justify-content: center; }
.justify-between { justify-content: space-between; }
.justify-end { justify-content: flex-end; }
.flex-1 { flex: 1 1 0%; }
.flex-col { flex-direction: column; }
.flex-row { flex-direction: row; }
.flex-wrap { flex-wrap: wrap; }
.shrink-0 { flex-shrink: 0; }
.relative { position: relative; }
.fixed { position: fixed; }
.absolute { position: absolute; }
.sticky { position: sticky; }
.inset-0 { top: 0; right: 0; bottom: 0; left: 0; }
.left-\[50\%\] { left: 50%; }
.top-\[50\%\] { top: 50%; }
.z-50 { z-index: 50; }
.translate-x-\[-50\%\] { transform: translateX(-50%); }
.translate-y-\[-50\%\] { transform: translateY(-50%); }
.text-xs { font-size: 0.75rem; line-height: 1rem; }
.text-sm { font-size: 0.875rem; line-height: 1.25rem; }
.text-lg { font-size: 1.125rem; line-height: 1.75rem; }
.text-xl { font-size: 1.25rem; line-height: 1.75rem; }
.text-2xl { font-size: 1.5rem; line-height: 2rem; }
.text-3xl { font-size: 1.875rem; line-height: 2.25rem; }
.font-medium { font-weight: 500; }
.font-semibold { font-weight: 600; }
.font-bold { font-weight: 700; }
.leading-none { line-height: 1; }
.leading-tight { line-height: 1.25; }
.leading-relaxed { line-height: 1.625; }
.tracking-tight { letter-spacing: -0.025em; }
.text-center { text-align: center; }
.text-left { text-align: left; }
.text-right { text-align: right; }
.underline { text-decoration: underline; }
.underline-offset-4 { text-underline-offset: 4px; }
.italic { font-style: italic; }
.uppercase { text-transform: uppercase; }
.capitalize { text-transform: capitalize; }
.whitespace-nowrap { white-space: nowrap; }
.break-words { overflow-wrap: break-word; }
.truncate { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.rounded-full { border-radius: 9999px; }
.transition-colors { transition-property: color, background-color, border-color; transition-duration: 150ms; }
.transition-all { transition-property: all; transition-duration: 150ms; }
.transition-shadow { transition-property: box-shadow; transition-duration: 150ms; }
.transition-opacity { transition-property: opacity; transition-duration: 150ms; }
.transition-transform { transition-property: transform; transition-duration: 150ms; }
.focus\:outline-none:focus { outline: none; }
.focus-visible\:outline-none:focus-visible { outline: none; }
.focus\:ring-2:focus { box-shadow: 0 0 0 2px var(--ring); }
.focus-visible\:ring-1:focus-visible { box-shadow: 0 0 0 1px var(--ring); }
.focus-visible\:ring-ring:focus-visible { box-shadow: 0 0 0 1px var(--ring); }
.disabled\:pointer-events-none:disabled { pointer-events: none; }
.disabled\:opacity-50:disabled { opacity: 0.5; }
.hover\:bg-accent:hover { background-color: var(--accent); }
.hover\:bg-primary\/90:hover { background-color: hsl(240 5.9% 10% / 0.9); }
.hover\:bg-secondary\/80:hover { background-color: hsl(240 4.8% 95.9% / 0.8); }
.hover\:bg-destructive\/90:hover { background-color: hsl(0 84.2% 60.2% / 0.9); }
.hover\:bg-muted\/50:hover { background-color: hsl(240 4.8% 95.9% / 0.5); }
.hover\:text-accent-foreground:hover { color: var(--accent-foreground); }
.hover\:text-primary-foreground:hover { color: var(--primary-foreground); }
.hover\:underline:hover { text-decoration: underline; }
.animate-pulse { animation: pulse 2s cubic-bezier(0.4, 0, 0.6, 1) infinite; }
.animate-in { animation: anim-in 200ms ease-out; }
@keyframes pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.5; } }
@keyframes anim-in { from { opacity: 0; transform: translateY(4px); } to { opacity: 1; transform: translateY(0); } }
.antialiased { -webkit-font-smoothing: antialiased; -moz-osx-font-smoothing: grayscale; }
.cursor-not-allowed { cursor: not-allowed; }
.cursor-pointer { cursor: pointer; }
.select-none { user-select: none; }
.overflow-hidden { overflow: hidden; }
.overflow-auto { overflow: auto; }
.overflow-x-auto { overflow-x: auto; }
.aspect-square { aspect-ratio: 1 / 1; }
.caption-bottom { caption-side: bottom; }
.placeholder\:text-muted-foreground::placeholder { color: var(--muted-foreground); }
.file\:border-0::file-selector-button { border-width: 0; }
.file\:bg-transparent::file-selector-button { background-color: transparent; }
.file\:text-sm::file-selector-button { font-size: 0.875rem; }
.file\:font-medium::file-selector-button { font-weight: 500; }
.peer-disabled\:cursor-not-allowed:peer(:disabled) ~ * { cursor: not-allowed; }
.peer-disabled\:opacity-70:peer(:disabled) ~ * { opacity: 0.7; }
[data-state="open"] { display: block; }
.sm\:rounded-lg { border-radius: var(--radius); }
@media (min-width: 640px) {
  .sm\:text-left { text-align: left; }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_css_contains_required_tokens() {
        assert!(BASE_CSS.contains("--background"));
        assert!(BASE_CSS.contains("--primary"));
        assert!(BASE_CSS.contains(".bg-background"));
        assert!(BASE_CSS.contains(".text-foreground"));
        assert!(BASE_CSS.contains(".rounded-md"));
        assert!(BASE_CSS.contains(".flex"));
    }

    #[test]
    fn base_css_has_dark_mode_override() {
        assert!(BASE_CSS.contains(".dark {"));
        assert!(BASE_CSS.contains("--background: hsl(240 10% 3.9%)"));
    }
}
