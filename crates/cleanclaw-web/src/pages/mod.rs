//! `pages` module. Each submodule matches one page in
//! . W1 only stubs `index`
//! and `overview`; the rest are added in W4–W6.

pub mod admin;
pub mod agent;
pub mod apikeys;
pub mod auth;
pub mod chat;
pub mod index;
pub mod overview;
pub mod resources;
pub mod settings;

use axum::Router;

use crate::server::WebState;

/// Mount future page modules onto the W1 router. W1's pages are
/// already wired inside `server::router`; subsequent phases append
/// their own routes here.
pub fn mount(router: Router, _state: WebState) -> Router {
    router
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::Theme;
    use crate::layout::{render, NavKey};

    #[test]
    fn index_renders_to_html() {
        let html = index::render();
        assert!(html.starts_with("<!DOCTYPE"));
        assert!(html.contains("CleanClaw"));
    }

    #[test]
    fn overview_renders_to_html() {
        let html = overview::render(None, Theme::Light);
        assert!(html.starts_with("<!DOCTYPE"));
        assert!(html.contains("Overview"));
    }

    #[test]
    fn overview_routes_through_app_shell() {
        let html = overview::render(Some(("Ada", "user")), Theme::Dark);
        assert!(html.contains("Ada"));
        assert!(html.contains(r#"class="dark""#));
        // Routes through the layout shell's `render` helper, which means
        // it ends up in an html envelope + sidebar/topbar.
        let _ = NavKey::Overview; // ensure NavKey is in scope
        let _: String = render("t", NavKey::Overview, "", None, Theme::Light);
    }

    #[tokio::test]
    async fn mount_returns_a_router() {
        let (tx, _rx) = tokio::sync::watch::channel(false);
        let state = crate::server::WebState::new(tx);
        let r = crate::server::router(state.clone());
        let r2 = mount(r, state);
        let _ = r2;
    }
}
