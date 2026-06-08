//! HTML helper primitives for the 21 shadcn/ui components used by the
//! frontend. All helpers return `String` fragments (no template
//! engine). They apply the shadcn / Tailwind-style class names so the
//! inline `base.css` matches the visual treatment of the React
//! components in .
//!
//! Anti-XSS: every free-form text input is run through `html::escape`
//! before it is interpolated. Class names are static strings.

use std::fmt::Write;

/// Escape HTML special characters. The same routine is used by every
/// helper in this module.
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            _ => out.push(ch),
        }
    }
    out
}

/// A minimal class-merging helper. Mirrors the `cn()` in
///  — joins non-empty class names
/// with a single space and trims duplicate whitespace.
pub fn cn<I, S>(parts: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut out = String::new();
    for p in parts {
        let s = p.as_ref().trim();
        if s.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(s);
    }
    out
}

/// Variant → class mapping for the shadcn `Button` component. The
/// values mirror the CVA mappings in
/// .
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonVariant {
    Default,
    Destructive,
    Outline,
    Secondary,
    Ghost,
    Link,
}

impl ButtonVariant {
    pub fn class(self) -> &'static str {
        match self {
            ButtonVariant::Default => "bg-primary text-primary-foreground hover:bg-primary/90",
            ButtonVariant::Destructive => "bg-destructive text-white hover:bg-destructive/90",
            ButtonVariant::Outline => {
                "border border-input bg-background hover:bg-accent hover:text-accent-foreground"
            }
            ButtonVariant::Secondary => {
                "bg-secondary text-secondary-foreground hover:bg-secondary/80"
            }
            ButtonVariant::Ghost => "hover:bg-accent hover:text-accent-foreground",
            ButtonVariant::Link => "text-primary underline-offset-4 hover:underline",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonSize {
    Default,
    Sm,
    Lg,
    Icon,
}

impl ButtonSize {
    pub fn class(self) -> &'static str {
        match self {
            ButtonSize::Default => "h-9 px-4 py-2",
            ButtonSize::Sm => "h-8 px-3 text-xs",
            ButtonSize::Lg => "h-10 px-6",
            ButtonSize::Icon => "size-9",
        }
    }
}

/// Render a `<button>` (or `<a>` if `href` is set) with the shadcn
/// visual treatment. All text content is HTML-escaped.
pub fn button(label: &str, variant: ButtonVariant, size: ButtonSize, href: Option<&str>) -> String {
    let cls = cn([
        "inline-flex items-center justify-center gap-2 whitespace-nowrap rounded-md text-sm font-medium",
        "transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
        "disabled:pointer-events-none disabled:opacity-50",
        variant.class(),
        size.class(),
    ]);
    let label = esc(label);
    if let Some(href) = href {
        format!(r#"<a class="{cls}" href="{href}">{label}</a>"#)
    } else {
        format!(r#"<button type="button" class="{cls}">{label}</button>"#)
    }
}

/// Card wrapper — `<div class="rounded-lg border bg-card text-card-foreground shadow-sm">`.
pub fn card_open(extra: &str) -> String {
    let cls = cn([
        "rounded-lg border bg-card text-card-foreground shadow-sm",
        extra,
    ]);
    format!(r#"<div class="{cls}">"#)
}

pub fn card_close() -> &'static str {
    "</div>"
}

pub fn card_header() -> &'static str {
    r#"<div class="flex flex-col space-y-1.5 p-6">"#
}

pub fn card_title(text: &str) -> String {
    format!(
        r#"<h3 class="text-lg font-semibold leading-none tracking-tight">{}</h3>"#,
        esc(text)
    )
}

pub fn card_description(text: &str) -> String {
    format!(
        r#"<p class="text-sm text-muted-foreground">{}</p>"#,
        esc(text)
    )
}

/// Card with a title + description stacked. Convenience for the
/// common header pattern.
pub fn card_title_with_desc(title: &str, desc: &str) -> String {
    format!("{}{}", card_title(title), card_description(desc))
}

pub fn card_content(extra: &str) -> String {
    let cls = cn(["p-6 pt-0", extra]);
    format!(r#"<div class="{cls}">"#)
}

pub fn card_footer() -> &'static str {
    r#"<div class="flex items-center p-6 pt-0">"#
}

/// Badge. Variants mirror the shadcn `Badge` component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadgeVariant {
    Default,
    Secondary,
    Destructive,
    Outline,
}

impl BadgeVariant {
    pub fn class(self) -> &'static str {
        match self {
            BadgeVariant::Default => "border-transparent bg-primary text-primary-foreground",
            BadgeVariant::Secondary => "border-transparent bg-secondary text-secondary-foreground",
            BadgeVariant::Destructive => "border-transparent bg-destructive text-white",
            BadgeVariant::Outline => "text-foreground",
        }
    }
}

pub fn badge(text: &str, variant: BadgeVariant) -> String {
    let cls = cn([
        "inline-flex items-center rounded-md border px-2.5 py-0.5 text-xs font-semibold",
        "transition-colors focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2",
        variant.class(),
    ]);
    format!(r#"<span class="{cls}">{}</span>"#, esc(text))
}

/// Input — `<input>` with the shadcn visual treatment.
pub fn input(name: &str, placeholder: &str, value: &str, input_type: &str) -> String {
    let cls = "flex h-9 w-full rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-sm transition-colors file:border-0 file:bg-transparent file:text-sm file:font-medium placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50";
    format!(
        r#"<input class="{cls}" type="{ty}" name="{nm}" placeholder="{ph}" value="{val}" />"#,
        ty = esc(input_type),
        nm = esc(name),
        ph = esc(placeholder),
        val = esc(value),
    )
}

/// Textarea — same treatment as `input`.
pub fn textarea(name: &str, placeholder: &str, value: &str, rows: u32) -> String {
    let cls = "flex min-h-[60px] w-full rounded-md border border-input bg-transparent px-3 py-2 text-sm shadow-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50";
    format!(
        r#"<textarea class="{cls}" name="{nm}" placeholder="{ph}" rows="{rows}">{val}</textarea>"#,
        nm = esc(name),
        ph = esc(placeholder),
        val = esc(value),
    )
}

/// Password input — `<input type="password">` with the shadcn
/// visual treatment.
pub fn html_password(name: &str, placeholder: &str, value: &str) -> String {
    input(name, placeholder, value, "password")
}

/// Label — `<label>`.
pub fn label(text: &str, for_id: &str) -> String {
    format!(
        r#"<label for="{id}" class="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70">{txt}</label>"#,
        id = esc(for_id),
        txt = esc(text),
    )
}

/// Alert.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertVariant {
    Default,
    Destructive,
}

impl AlertVariant {
    pub fn class(self) -> &'static str {
        match self {
            AlertVariant::Default => "border bg-background text-foreground",
            AlertVariant::Destructive => {
                "border-destructive/50 text-destructive bg-destructive/10 [&>svg]:text-destructive"
            }
        }
    }
}

pub fn alert(title: &str, body: &str, variant: AlertVariant) -> String {
    let cls = cn([
        "relative w-full rounded-lg border px-4 py-3 text-sm",
        variant.class(),
    ]);
    format!(
        r#"<div class="{cls}" role="alert"><h5 class="mb-1 font-medium leading-none tracking-tight">{}</h5><div class="text-sm [&_p]:leading-relaxed">{}</div></div>"#,
        esc(title),
        esc(body),
    )
}

/// Separator.
pub fn separator(horizontal: bool) -> String {
    if horizontal {
        r#"<div class="shrink-0 bg-border h-px w-full"></div>"#.to_string()
    } else {
        r#"<div class="shrink-0 bg-border h-full w-px"></div>"#.to_string()
    }
}

/// Skeleton.
pub fn skeleton(extra: &str) -> String {
    let cls = cn(["animate-pulse rounded-md bg-muted", extra]);
    format!(r#"<div class="{cls}"></div>"#)
}

/// Avatar.
pub fn avatar(initials: &str, src: Option<&str>) -> String {
    let wrap = r#"<span class="relative flex h-8 w-8 shrink-0 overflow-hidden rounded-full">"#;
    if let Some(src) = src {
        format!(
            r#"{wrap}<img class="aspect-square h-full w-full" src="{}" alt="{}" /></span>"#,
            esc(src),
            esc(initials),
        )
    } else {
        format!(
            r#"{wrap}<span class="flex h-full w-full items-center justify-center rounded-full bg-muted text-xs font-medium">{}</span></span>"#,
            esc(initials),
        )
    }
}

/// Render a `<table>` skeleton. `rows` is `(cells...)` tuples — one
/// tuple per row. Headings live in the first tuple.
pub fn table(headings: &[&str], rows: &[Vec<String>]) -> String {
    let mut out = String::new();
    let cls = "w-full caption-bottom text-sm";
    let _ = write!(
        out,
        r#"<div class="relative w-full overflow-auto"><table class="{cls}">"#
    );
    let _ = write!(out, r#"<thead class="[&_tr]:border-b"><tr>"#);
    for h in headings {
        let _ = write!(
            out,
            r#"<th class="h-10 px-2 text-left align-middle font-medium text-muted-foreground">{}</th>"#,
            esc(h)
        );
    }
    let _ = write!(
        out,
        "</tr></thead><tbody class=\"[&_tr:last-child]:border-0\">"
    );
    for row in rows {
        let _ = write!(
            out,
            r#"<tr class="border-b transition-colors hover:bg-muted/50">"#
        );
        for cell in row {
            let _ = write!(out, r#"<td class="p-2 align-middle">{}</td>"#, esc(cell));
        }
        let _ = write!(out, "</tr>");
    }
    let _ = write!(out, "</tbody></table></div>");
    out
}

/// Dialog (open) — minimal modal wrapper. Inline-styled because the
/// shadcn modal layer is animated; the SSR version just shows the
/// content.
pub fn dialog_open(title: &str) -> String {
    format!(
        r#"<div class="fixed inset-0 z-50 bg-black/80 data-[state=open]:animate-in" data-state="open"><div class="fixed left-[50%] top-[50%] z-50 grid w-full max-w-lg translate-x-[-50%] translate-y-[-50%] gap-4 border bg-background p-6 shadow-lg sm:rounded-lg"><div class="flex flex-col space-y-1.5 text-center sm:text-left"><h2 class="text-lg font-semibold leading-none tracking-tight">{}</h2></div>"#,
        esc(title),
    )
}

pub fn dialog_close() -> &'static str {
    r#"</div></div>"#
}

/// Tabs — renders a horizontal tab strip. The active tab gets the
/// underline + bold treatment. Each label is rendered with the
/// `data-tab` attribute that vanilla JS reads in the future (W7
/// hooks port).
pub fn tabs(labels: &[(&str, &str)], active: &str) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        r#"<div class="inline-flex h-9 items-center justify-center rounded-lg bg-muted p-1 text-muted-foreground" role="tablist">"#
    );
    for (id, label) in labels {
        let is_active = *id == active;
        let cls = if is_active {
            "inline-flex items-center justify-center whitespace-nowrap rounded-md bg-background px-3 py-1 text-sm font-medium shadow"
        } else {
            "inline-flex items-center justify-center whitespace-nowrap rounded-md px-3 py-1 text-sm font-medium ring-offset-background transition-all focus-visible:outline-none"
        };
        let _ = write!(
            out,
            r#"<button type="button" role="tab" data-tab="{id}" class="{cls}">{label}</button>"#,
            id = esc(id),
            label = esc(label),
        );
    }
    out.push_str("</div>");
    out
}

/// Build a complete `<html>` document. `body` is the page-specific
/// fragment; `title` is the page `<title>`. `base_css` is the inline
/// stylesheet that ships in .
pub fn page(title: &str, body: &str, base_css: &str, theme: Theme) -> String {
    let cls = if matches!(theme, Theme::Dark) {
        "dark"
    } else {
        ""
    };
    format!(
        r#"<!DOCTYPE html>
<html lang="en" class="{cls}">
<head>
<meta charset="utf-8" />
<meta name="viewport" content="width=device-width, initial-scale=1" />
<title>{title}</title>
<style>{css}</style>
</head>
<body class="bg-background text-foreground antialiased min-h-screen">
{body}
</body>
</html>"#,
        cls = cls,
        title = esc(title),
        css = base_css,
        body = body,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

impl Theme {
    pub fn from_query(v: Option<&str>) -> Self {
        match v {
            Some("dark") => Theme::Dark,
            _ => Theme::Light,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn esc_escapes_specials() {
        assert_eq!(esc("<script>"), "&lt;script&gt;");
        assert_eq!(esc("a & b"), "a &amp; b");
        assert_eq!(esc(r#"'q'"#), "&#x27;q&#x27;");
    }

    #[test]
    fn cn_skips_empty() {
        let s = cn(["btn", "", "primary", "  ", "rounded"]);
        assert_eq!(s, "btn primary rounded");
    }

    #[test]
    fn button_class_includes_variant() {
        let s = button("Click", ButtonVariant::Default, ButtonSize::Default, None);
        assert!(s.contains("bg-primary"));
        assert!(s.contains("h-9"));
    }

    #[test]
    fn button_with_href_renders_anchor() {
        let s = button("Go", ButtonVariant::Outline, ButtonSize::Sm, Some("/x"));
        assert!(s.starts_with("<a "));
        assert!(s.contains(r#"href="/x""#));
    }

    #[test]
    fn input_escapes_value() {
        let s = input("name", "Enter", "<bad>", "text");
        assert!(s.contains(r#"value="&lt;bad&gt;""#));
    }

    #[test]
    fn badge_class_includes_variant() {
        let s = badge("ok", BadgeVariant::Destructive);
        assert!(s.contains("bg-destructive"));
    }

    #[test]
    fn alert_renders_title_and_body() {
        let s = alert("Heads up", "Something happened", AlertVariant::Destructive);
        assert!(s.contains("Heads up"));
        assert!(s.contains("Something happened"));
    }

    #[test]
    fn table_renders_headings_and_rows() {
        let s = table(&["A", "B"], &[vec!["1".into(), "2".into()]]);
        assert!(s.contains("<th"));
        assert!(s.contains(">1<"));
        assert!(s.contains(">2<"));
    }

    #[test]
    fn page_wraps_full_doc() {
        let s = page("X", "<p>hi</p>", "body{color:red}", Theme::Dark);
        assert!(s.contains(r#"class="dark""#));
        assert!(s.contains("<title>X</title>"));
        assert!(s.contains("body{color:red}"));
    }

    #[test]
    fn tabs_marks_active() {
        let s = tabs(&[("a", "A"), ("b", "B")], "b");
        assert!(s.contains(r#"data-tab="a""#));
        assert!(s.contains(r#"data-tab="b""#));
        // The active tab's <button> opening tag carries the shadow
        // utility class. Find the `<button` that precedes
        // `data-tab="b"` and the matching `>`, then check the class.
        let active_at = s.find(r#"data-tab="b""#).expect("active attr");
        let open_at = s[..active_at].rfind("<button").expect("open tag");
        let close_rel = s[open_at..].find('>').expect("close");
        let tag = &s[open_at..open_at + close_rel + 1];
        assert!(tag.contains("shadow"), "active tag missing shadow: {tag}");
    }
}
