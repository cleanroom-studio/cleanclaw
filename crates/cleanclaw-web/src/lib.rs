//! Server-rendered web frontend. Mirrors the route table and page
//! surface of  (Next.js 16). All pages
//! are rendered to HTML on the server using `html::page` + the
//! shadcn-style helpers in `html.rs` and `layout.rs`.
//!
//! Routes are organized to match the Next.js folder structure:
//!
//! | Path                                  | Handler        | Phase |
//! |---------------------------------------|----------------|-------|
//! | `/`                                   | index          | W1    |
//! | `/overview`                           | overview       | W4    |
//! | `/signup`                             | signup         | W4    |
//! | `/onboard`                            | onboard        | W6    |
//! | `/chat`                               | chat           | W5    |
//! | `/agents`                             | agents list    | W5    |
//! | `/agents/{id}`                        | agent overview | W5    |
//! | `/agents/{id}/{tab}`                  | agent tab      | W5    |
//! | `/channels`                           | channels list  | W6    |
//! | `/channels-config`                    | channel config | W6    |
//! | `/cron`                               | scheduler      | W6    |
//! | `/models`                             | models         | W6    |
//! | `/providers`                          | providers      | W6    |
//! | `/skills`                             | skills         | W6    |
//! | `/tools`                              | tools          | W6    |
//! | `/plugins`                            | plugins        | W6    |
//! | `/apikeys`                            | api keys       | W4    |
//! | `/settings`                           | settings root  | W4    |
//! | `/settings/{about,account,general,runtime}` | settings | W4    |
//! | `/admin/{users,usage,chats}`          | admin          | W4    |
//! | `/api/me`                             | me             | W3    |
//! | `/api/login`                          | login          | W3    |
//! | `/api/logout`                         | logout         | W3    |
//! | `/api/register`                       | register       | W3    |
//! | `/api/status`                         | status         | W3    |
//! | `/api/config`                         | config         | W3    |
//! | `/api/agents/...`                     | agent CRUD     | W3    |
//! | `/api/channels/...`                   | channel CRUD   | W3    |
//! | `/api/cron/...`                       | cron CRUD      | W3    |
//! | `/api/skills/...`                     | skills         | W3    |
//! | `/api/plugins/...`                    | plugins        | W3    |
//! | `/api/tools/...`                      | tools          | W3    |
//! | `/api/usage/...`                      | usage          | W3    |
//! | `/api/projects/...`                   | projects       | W3    |
//! | `/api/sessions/...`                   | sessions       | W3    |
//! | `/api/admin/...`                      | admin          | W3    |
//! | `/api/ws/chat`                        | ws             | W5    |
//!
//! W1 only wires `/` + `/overview` + `/favicon.ico` + the global CSS.
//! Subsequent phases fill in the rest.

pub mod css;
pub mod html;
pub mod hooks;
pub mod layout;
pub mod markdown;
pub mod scope_picker;
pub mod types;
pub mod client;
pub mod pages;
pub mod server;

pub use server::serve;

/// Re-export the `Theme` enum so callers can avoid depending on
/// `cleanclaw_web::html` directly.
pub use html::Theme;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_links() {
        // Smoke test: make sure the public types compile and link.
        let _: fn() -> String = || super::layout::render("t", super::layout::NavKey::Overview, "", None, super::Theme::Light);
    }
}
