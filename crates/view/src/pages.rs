//! The pages noal serves today.
//!
//! This module is deliberately thin. noal's subject matter is not decided yet,
//! so it holds the home page and the error page and nothing more. Add a module
//! per domain area as the product takes shape, and keep every function pure.

use maud::{html, Markup};
use noal_core::ask::outcome::Outcome;

use crate::ask;
use crate::layout::{page, Chrome, Viewer};
use crate::windows::cut;

/// The landing page.
#[must_use]
pub fn home(chrome: &Chrome) -> Markup {
    page(
        "Home",
        chrome,
        &html! {
            h1 { "noal" }
            @match &chrome.viewer {
                Viewer::Anonymous => {
                    p { "You are not signed in." }
                }
                Viewer::SignedIn { .. } => {
                    (crate::ask::form())
                }
            }
        },
    )
}

/// A saved window reopened as its own page.
///
/// A window URL answers with a full document — never a fragment — so a link
/// from the tree, a pushed URL, and a reload all land on the same complete
/// page. The palette marks this window's row, and the debug payload rides
/// along so the Debug tab shows what produced the view.
#[must_use]
pub fn window(chrome: &Chrome, outcome: &Outcome) -> Markup {
    page(
        cut(&outcome.request).as_ref(),
        chrome,
        &html! {
            section #ask-result {
                h1 { (cut(&outcome.request)) }
                (ask::outcome_view(outcome))
                script #ask-debug type="application/json" { (maud::PreEscaped(outcome.debug_json())) }
            }
        },
    )
}

/// A page shown when a request could not be served.
///
/// The message is written by noal, never echoed from user input, so a failure
/// cannot become a way to put text on the screen.
#[must_use]
pub fn failure(chrome: &Chrome, status: u16, message: &str) -> Markup {
    page(
        "Something went wrong",
        chrome,
        &html! {
            h1 { (status) }
            p { (message) }
            p { a href="/" { "Back to the start" } }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::{failure, home, window};
    use crate::layout::{Chrome, Viewer};
    use crate::windows::{Current, Entry, Node, Windows};
    use noal_core::ask::outcome::{Debug, Origin, Outcome, Stage, Verdict};

    fn signed_in(email: &str) -> Chrome {
        Chrome {
            viewer: Viewer::SignedIn {
                email: email.to_owned(),
            },
            windows: Windows::Tree(Vec::new()),
            current: Current::Home,
        }
    }

    fn outcome(verdict: Verdict) -> Outcome {
        Outcome {
            request: "open tasks".into(),
            verdict,
            origin: Origin::Reopened,
            debug: Debug::default(),
        }
    }

    #[test]
    fn home_tells_an_anonymous_viewer_they_are_signed_out() {
        let rendered = home(&Chrome::anonymous()).into_string();
        assert!(rendered.contains("You are not signed in."));
    }

    #[test]
    fn home_offers_a_signed_in_viewer_the_ask_form() {
        let rendered = home(&signed_in("someone@example.com")).into_string();
        assert!(rendered.contains("hx-post=\"/ask\""));
    }

    #[test]
    fn failure_shows_the_status_and_a_way_back() {
        let rendered = failure(&Chrome::anonymous(), 404, "No such page.").into_string();
        assert!(rendered.contains("404"));
        assert!(rendered.contains("No such page."));
        assert!(rendered.contains("href=\"/\""));
    }

    #[test]
    fn the_failure_page_carries_no_palette_when_rendered_anonymous() {
        // The shell reaches for this page through `Chrome::anonymous`, because
        // identity itself may be what failed.
        let rendered =
            failure(&Chrome::anonymous(), 500, "The database did not answer.").into_string();
        assert!(rendered.contains("The database did not answer."));
        assert!(!rendered.contains("id=\"palette\""));
    }

    #[test]
    fn a_window_page_is_a_full_document_carrying_the_answer() {
        let rendered = window(
            &signed_in("someone@example.com"),
            &outcome(Verdict::Answered {
                html: "<ul><li>a</li></ul>".into(),
            }),
        )
        .into_string();

        assert!(rendered.starts_with("<!DOCTYPE html>"));
        assert!(rendered.contains("<div class=\"ask-answer\"><ul><li>a</li></ul></div>"));
        assert!(rendered.contains("id=\"ask-debug\""));
        assert!(rendered.contains("\"origin\":\"reopened\""));
    }

    #[test]
    fn a_window_page_marks_its_own_row_in_the_tree() {
        let mut entry = Entry {
            id: uuid::Uuid::from_bytes([7; 16]),
            parent_id: None,
            request: "open tasks".into(),
            name: None,
        };
        entry.name = Some("Weekly report".to_owned());
        let chrome = Chrome {
            viewer: signed_in("someone@example.com").viewer,
            windows: Windows::Tree(vec![Node {
                entry,
                children: Vec::new(),
            }]),
            current: Current::Window(uuid::Uuid::from_bytes([7; 16])),
        };

        let rendered = window(
            &chrome,
            &outcome(Verdict::Answered {
                html: String::new(),
            }),
        )
        .into_string();
        assert!(rendered.contains("<li id=\"window-current\"><a href=\"/w/"));
        assert_eq!(rendered.matches("id=\"window-current\"").count(), 1);
    }

    #[test]
    fn a_window_page_shows_the_refusal_without_a_retry_form() {
        let rendered = window(
            &signed_in("someone@example.com"),
            &outcome(Verdict::Failed {
                stage: Stage::Query,
            }),
        )
        .into_string();
        assert!(rendered.contains("could not run the query"));
        assert!(
            !rendered.contains("hx-post"),
            "re-running is not this slice's job"
        );
    }

    #[test]
    fn the_window_page_escapes_the_request_it_titles() {
        let mut o = outcome(Verdict::Answered {
            html: String::new(),
        });
        o.request = "<script>alert(1)</script>".into();
        let rendered = window(&signed_in("someone@example.com"), &o).into_string();
        assert!(rendered.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    }
}
