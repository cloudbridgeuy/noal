//! The pages noal serves today.
//!
//! This module is deliberately thin. noal's subject matter is not decided yet,
//! so it holds the home page and the error page and nothing more. Add a module
//! per domain area as the product takes shape, and keep every function pure.

use maud::{html, Markup};

use crate::layout::{page, Viewer};

/// The landing page.
#[must_use]
pub fn home(viewer: &Viewer) -> Markup {
    page(
        "Home",
        viewer,
        &html! {
            h1 { "noal" }
            @match viewer {
                Viewer::Anonymous => {
                    p { "You are not signed in." }
                }
                Viewer::SignedIn { email } => {
                    p { "Signed in as " (email) "." }
                }
            }
        },
    )
}

/// A page shown when a request could not be served.
///
/// The message is written by noal, never echoed from user input, so a failure
/// cannot become a way to put text on the screen.
#[must_use]
pub fn failure(viewer: &Viewer, status: u16, message: &str) -> Markup {
    page(
        "Something went wrong",
        viewer,
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
    fn home_greets_a_signed_in_viewer_by_email() {
        let viewer = Viewer::SignedIn {
            email: "someone@example.com".to_owned(),
        };
        let rendered = home(&viewer).into_string();
        assert!(rendered.contains("Signed in as someone@example.com."));
    }

    #[test]
    fn failure_shows_the_status_and_a_way_back() {
        let rendered = failure(&Viewer::Anonymous, 404, "No such page.").into_string();
        assert!(rendered.contains("404"));
        assert!(rendered.contains("No such page."));
        assert!(rendered.contains("href=\"/\""));
    }
}
