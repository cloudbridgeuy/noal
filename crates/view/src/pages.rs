//! The pages noal serves today.
//!
//! This module is deliberately thin. noal's subject matter is not decided yet,
//! so it holds the home page and the error page and nothing more. Add a module
//! per domain area as the product takes shape, and keep every function pure.

use maud::{html, Markup};

use crate::layout::{page, Palette, Viewer};

/// The landing page.
///
/// A signed-in viewer gets the palette, open, and `#ask-result` holding a
/// plain greeting until they ask something. An anonymous viewer gets neither:
/// there is no form to post, so there is nothing for `#ask-result` to be a
/// target for.
#[must_use]
pub fn home(viewer: &Viewer) -> Markup {
    let palette = match viewer {
        Viewer::Anonymous => Palette::Closed,
        Viewer::SignedIn { .. } => Palette::Open,
    };
    page(
        "Home",
        viewer,
        palette,
        &html! {
            @match viewer {
                Viewer::Anonymous => {
                    p { "You are not signed in." }
                }
                Viewer::SignedIn { .. } => {
                    (crate::ask::greeting())
                }
            }
        },
    )
}

/// A page shown when a request could not be served.
///
/// The message is written by noal, never echoed from user input, so a failure
/// cannot become a way to put text on the screen. It carries no palette: a
/// failure page has nothing for the palette to ask about.
#[must_use]
pub fn failure(viewer: &Viewer, status: u16, message: &str) -> Markup {
    page(
        "Something went wrong",
        viewer,
        Palette::Closed,
        &html! {
            h1 { (status) }
            p { (message) }
            p { a href="/" { "Back to the start" } }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::{failure, home};
    use crate::layout::Viewer;

    #[test]
    fn home_tells_an_anonymous_viewer_they_are_signed_out() {
        let rendered = home(&Viewer::Anonymous).into_string();
        assert!(rendered.contains("You are not signed in."));
    }

    #[test]
    fn an_anonymous_home_carries_neither_palette_nor_ask_result() {
        let rendered = home(&Viewer::Anonymous).into_string();
        assert!(!rendered.contains("id=\"palette\""));
        assert!(!rendered.contains("id=\"ask-result\""));
    }

    #[test]
    fn home_offers_a_signed_in_viewer_an_open_palette() {
        let viewer = Viewer::SignedIn {
            email: "someone@example.com".to_owned(),
        };
        let rendered = home(&viewer).into_string();
        assert!(rendered.contains("id=\"palette\""));
        assert!(rendered.contains("hx-post=\"/ask\""));
    }

    #[test]
    fn home_puts_the_greeting_inside_ask_result_for_a_signed_in_viewer() {
        let viewer = Viewer::SignedIn {
            email: "someone@example.com".to_owned(),
        };
        let rendered = home(&viewer).into_string();
        assert!(rendered.contains("id=\"ask-result\""));
        assert!(rendered.contains("noal"));
    }

    #[test]
    fn a_failure_page_carries_no_palette() {
        let rendered = failure(&Viewer::Anonymous, 500, "It broke.").into_string();
        assert!(!rendered.contains("id=\"palette\""));
        assert!(!rendered.contains("id=\"ask-result\""));
    }

    #[test]
    fn failure_shows_the_status_and_a_way_back() {
        let rendered = failure(&Viewer::Anonymous, 404, "No such page.").into_string();
        assert!(rendered.contains("404"));
        assert!(rendered.contains("No such page."));
        assert!(rendered.contains("href=\"/\""));
    }
}
