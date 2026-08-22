//! The page chrome shared by every full document.

use maud::{html, Markup, DOCTYPE};

/// The htmx build noal loads. Pinned, and served with an integrity hash, so a
/// change to the CDN cannot change what runs in the browser.
const HTMX_SRC: &str = "https://unpkg.com/htmx.org@2.0.4/dist/htmx.min.js";

/// Who is looking at the page. `view` renders differently for a signed-in user,
/// so the shell passes identity in rather than the template reaching for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Viewer {
    /// Nobody is signed in.
    Anonymous,
    /// A signed-in user, identified by the email noal shows in the header.
    SignedIn {
        /// The address to display.
        email: String,
    },
}

/// Wrap body markup in the full document chrome.
///
/// Use this for a normal navigation. For an htmx swap, return the fragment on
/// its own instead; sending a whole document into a swap target nests one
/// `<html>` inside another.
#[must_use]
pub fn page(title: &str, viewer: &Viewer, body: &Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) " · noal" }
                script src=(HTMX_SRC) defer {}
            }
            body {
                header { (header(viewer)) }
                main { (body) }
            }
        }
    }
}

/// The masthead, with the sign-in or sign-out control.
#[must_use]
pub fn header(viewer: &Viewer) -> Markup {
    html! {
        nav {
            a href="/" { "noal" }
            @match viewer {
                Viewer::Anonymous => {
                    a href="/auth/login" { "Sign in" }
                }
                Viewer::SignedIn { email } => {
                    span .viewer-email { (email) }
                    a href="/auth/logout" { "Sign out" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{header, page, Viewer};
    use maud::html;

    #[test]
    fn a_page_is_a_whole_document() {
        let markup = page("Home", &Viewer::Anonymous, &html! { p { "hello" } });
        let rendered = markup.into_string();
        assert!(rendered.starts_with("<!DOCTYPE html>"));
        assert!(rendered.contains("<title>Home · noal</title>"));
        assert!(rendered.contains("<p>hello</p>"));
    }

    #[test]
    fn an_anonymous_viewer_is_offered_sign_in() {
        let rendered = header(&Viewer::Anonymous).into_string();
        assert!(rendered.contains("/auth/login"));
        assert!(!rendered.contains("/auth/logout"));
    }

    #[test]
    fn a_signed_in_viewer_is_offered_sign_out() {
        let viewer = Viewer::SignedIn {
            email: "someone@example.com".to_owned(),
        };
        let rendered = header(&viewer).into_string();
        assert!(rendered.contains("someone@example.com"));
        assert!(rendered.contains("/auth/logout"));
        assert!(!rendered.contains("/auth/login"));
    }

    #[test]
    fn markup_escapes_viewer_supplied_text() {
        let viewer = Viewer::SignedIn {
            email: "<script>alert(1)</script>".to_owned(),
        };
        let rendered = header(&viewer).into_string();
        assert!(!rendered.contains("<script>alert(1)</script>"));
        assert!(rendered.contains("&lt;script&gt;"));
    }
}
