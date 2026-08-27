//! The pages noal serves today.
//!
//! This module is deliberately thin. noal's subject matter is not decided yet,
//! so it holds the home page, the window page, and the error page and nothing
//! more. Add a module per domain area as the product takes shape, and keep
//! every function pure.

use maud::{html, Markup};
use noal_core::ask::outcome::Outcome;

use crate::ask;
use crate::layout::{page, Chrome, Viewer};
use crate::windows::{cut, Windows};

/// The landing page.
///
/// A signed-in viewer asks from the palette, so the page's own job is to hold
/// `#ask-result` — resting at a plain greeting until the first answer swaps
/// into it. An anonymous viewer gets neither: there is no session for the
/// palette's form to post against, so there is nothing for `#ask-result` to
/// be a target for.
#[must_use]
pub fn home(chrome: &Chrome) -> Markup {
    page(
        "Home",
        chrome,
        &html! {
            h1 .mt-1 { "noal" }
            @match &chrome.viewer {
                Viewer::Anonymous => {
                    p .muted { "You are not signed in." }
                }
                Viewer::SignedIn { .. } => {
                    (crate::ask::greeting())
                }
            }
        },
    )
}

/// A saved window reopened as its own page.
///
/// A window URL answers with a full document — never a fragment — so a link
/// from the tree, a pushed URL, and a reload all land on the same complete
/// page. The palette marks this window's row, and the chrome carries the
/// debug payload so the Debug tab shows what produced the view.
///
/// The heading is an `h2` carrying [`crate::ask`]'s one heading id, not an
/// `h1`, so a rename response can replace it out of band with exactly the
/// markup [`ask::oob_heading`] renders — one label everywhere means one
/// heading element, not two that happen to agree.
#[must_use]
pub fn window(
    chrome: &Chrome,
    created_at: noal_core::clock::Timestamp,
    outcome: &Outcome,
) -> Markup {
    page(
        cut(&outcome.request).as_ref(),
        chrome,
        &html! {
            section .card #ask-result {
                (ask::oob_heading(&heading_entry(chrome, outcome)))
                p .saved-date {
                    time datetime=(created_at.to_rfc3339()) {
                        "Saved "
                        (created_at.display_date())
                        " · data re-read on arrival"
                    }
                }
                (ask::outcome_view(outcome))
            }
        },
    )
}

/// The entry the window page's heading renders: its tree row when the tree
/// holds one for the window being viewed, otherwise one built from the
/// reopened request alone.
///
/// Pairing by the current id rather than by the request text means two
/// windows asked for the same thing cannot borrow each other's names.
fn heading_entry(chrome: &Chrome, outcome: &Outcome) -> noal_core::window::Entry {
    let fallback = || noal_core::window::Entry {
        id: chrome.current.window_id().unwrap_or_default(),
        parent_id: None,
        request: outcome.request.clone(),
        name: None,
    };
    match (&chrome.windows, chrome.current.window_id()) {
        (Windows::Tree(nodes), Some(id)) => nodes
            .iter()
            .flat_map(crate::windows::Node::flatten)
            .find(|node| node.entry.id == id)
            .map_or_else(fallback, |node| node.entry.clone()),
        _ => fallback(),
    }
}
/// A page shown when a request could not be served.
///
/// The message is written by noal, never echoed from user input, so a failure
/// cannot become a way to put text on the screen. It renders through
/// [`Chrome::anonymous`], because identity itself may be what failed, and so
/// carries no palette and no toast region.
#[must_use]
pub fn failure(chrome: &Chrome, status: u16, message: &str) -> Markup {
    page(
        "Something went wrong",
        chrome,
        &html! {
            div .card {
                h1 { (status) }
                p { (message) }
                p { a .btn href="/" { "Back to the start" } }
            }
        },
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
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
            debug_json: None,
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

    fn saved_at() -> noal_core::clock::Timestamp {
        // 2026-02-01T10:20:30Z, fixed so assertions can name the strings.
        noal_core::clock::Timestamp::from_unix_seconds(1_769_941_230)
    }

    #[test]
    fn home_tells_an_anonymous_viewer_they_are_signed_out() {
        let rendered = home(&Chrome::anonymous()).into_string();
        assert!(rendered.contains("You are not signed in."));
    }

    #[test]
    fn an_anonymous_home_carries_neither_palette_nor_ask_result() {
        let rendered = home(&Chrome::anonymous()).into_string();
        assert!(!rendered.contains("id=\"palette\""));
        assert!(!rendered.contains("id=\"ask-result\""));
    }

    #[test]
    fn home_gives_a_signed_in_viewer_a_palette_and_a_resting_ask_result() {
        // Asking happens from the palette; the page holds the swap target
        // the first answer lands in, resting at a greeting until then.
        let rendered = home(&signed_in("someone@example.com")).into_string();
        assert!(rendered.contains("id=\"palette\""));
        assert!(rendered.contains("hx-post=\"/ask\""));
        assert!(rendered.contains("id=\"ask-result\""));
        assert!(rendered.contains(">noal<"));
    }

    #[test]
    fn the_home_heading_and_the_anonymous_line_carry_their_visual_classes() {
        let rendered = home(&Chrome::anonymous()).into_string();
        assert!(rendered.contains("<h1 class=\"mt-1\">noal</h1>"));
        assert!(rendered.contains("<p class=\"muted\">You are not signed in.</p>"));
    }

    #[test]
    fn failure_shows_the_status_and_a_way_back() {
        let rendered = failure(&Chrome::anonymous(), 404, "No such page.").into_string();
        assert!(rendered.contains("404"));
        assert!(rendered.contains("No such page."));
        assert!(rendered.contains("href=\"/\""));
    }

    #[test]
    fn the_failure_content_sits_in_a_card_with_a_button_back_link() {
        let rendered = failure(&Chrome::anonymous(), 404, "No such page.").into_string();
        let start = rendered.find("<div class=\"card\">").unwrap();
        let end = rendered[start..].find("</div>").unwrap() + start;
        assert!(rendered[start..=end].contains("<h1>404</h1>"));
        assert!(rendered[start..=end].contains("<p>No such page.</p>"));
        assert!(rendered[start..=end].contains("<a class=\"btn\" href=\"/\">Back to the start</a>"));
    }

    #[test]
    fn the_failure_page_carries_no_palette_when_rendered_anonymous() {
        // The shell reaches for this page through `Chrome::anonymous`, because
        // identity itself may be what failed.
        let rendered =
            failure(&Chrome::anonymous(), 500, "The database did not answer.").into_string();
        assert!(rendered.contains("The database did not answer."));
        assert!(!rendered.contains("id=\"palette\""));
        assert!(!rendered.contains("id=\"toasts\""));
    }

    #[test]
    fn a_window_page_is_a_full_document_carrying_the_answer() {
        // The shell fills the chrome's debug payload from the reopened
        // outcome before rendering, exactly as `routes::window` does.
        let mut chrome = signed_in("someone@example.com");
        chrome.debug_json = Some(
            outcome(Verdict::Answered {
                html: "<ul><li>a</li></ul>".into(),
            })
            .debug_json(),
        );
        let rendered = window(
            &chrome,
            saved_at(),
            &outcome(Verdict::Answered {
                html: "<ul><li>a</li></ul>".into(),
            }),
        )
        .into_string();

        assert!(rendered.starts_with("<!DOCTYPE html>"));
        assert!(rendered.contains("<div class=\"ask-answer\"><ul><li>a</li></ul></div>"));
        // The debug payload rides in the chrome's single #ask-debug element,
        // filled by the shell, so the Debug tab shows it before any ask runs.
        assert_eq!(rendered.matches("id=\"ask-debug\"").count(), 1);
        assert!(rendered.contains("\"origin\":\"reopened\""));
    }

    #[test]
    fn a_window_pages_heading_is_the_one_replaceable_h2() {
        // The rename response replaces exactly this element out of band, so
        // the page must carry the same id and markup `ask::oob_heading`
        // renders — an `h2` with `id="ask-heading"`, never an `h1`.
        let rendered = window(
            &signed_in("someone@example.com"),
            saved_at(),
            &outcome(Verdict::Answered {
                html: String::new(),
            }),
        )
        .into_string();

        let start = rendered
            .find("<section class=\"card\" id=\"ask-result\">")
            .unwrap();
        let section = &rendered[start..];
        assert!(
            section.starts_with("<section class=\"card\" id=\"ask-result\"><h2 id=\"ask-heading\"")
        );
        assert!(section.contains("<h2 id=\"ask-heading\""));
        assert!(
            !rendered.contains("<h1>open tasks</h1>"),
            "no second heading"
        );
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
        let mut chrome = signed_in("someone@example.com");
        chrome.windows = Windows::Tree(vec![Node {
            entry,
            children: Vec::new(),
        }]);
        chrome.current = Current::Window(uuid::Uuid::from_bytes([7; 16]));

        let rendered = window(
            &chrome,
            saved_at(),
            &outcome(Verdict::Answered {
                html: String::new(),
            }),
        )
        .into_string();
        assert!(rendered.contains(
            "<li class=\"tree-row\" id=\"window-current\"><a class=\"window-label\" href=\"/w/"
        ));
        assert_eq!(rendered.matches("id=\"window-current\"").count(), 1);
    }

    #[test]
    fn a_window_pages_heading_shows_the_stored_name_over_the_cut() {
        // One label everywhere: the page heading and the tree row both render
        // name-or-cut from the same stored value.
        let entry = Entry {
            id: uuid::Uuid::from_bytes([7; 16]),
            parent_id: None,
            request: "open tasks under the Render MVP epic with many words".into(),
            name: Some("Weekly report".into()),
        };
        let mut chrome = signed_in("someone@example.com");
        chrome.windows = Windows::Tree(vec![Node {
            entry,
            children: Vec::new(),
        }]);
        chrome.current = Current::Window(uuid::Uuid::from_bytes([7; 16]));

        let rendered = window(
            &chrome,
            saved_at(),
            &outcome(Verdict::Answered {
                html: String::new(),
            }),
        )
        .into_string();
        let at = rendered.find("id=\"ask-heading\"").unwrap();
        let tag =
            &rendered[rendered[..at].rfind('<').unwrap()..rendered[at..].find('>').unwrap() + at];
        assert!(tag.starts_with("<h2 "), "the heading is an h2: {tag}");
        assert!(
            tag.contains("title="),
            "the full request rides in the title: {tag}"
        );
        assert!(rendered.contains(">Weekly report</h2>"));
        assert!(!rendered.contains("…</h2>"));
    }

    #[test]
    fn a_window_page_shows_the_refusal_without_a_retry_form() {
        let rendered = window(
            &signed_in("someone@example.com"),
            saved_at(),
            &outcome(Verdict::Failed {
                stage: Stage::Query,
            }),
        )
        .into_string();
        assert!(rendered.contains("the query it saved was refused"));
        // The palette carries the one ask form; the answer's own section
        // must not grow a second one.
        let start = rendered
            .find("<section class=\"card\" id=\"ask-result\">")
            .unwrap();
        let end = rendered[start..].find("</section>").unwrap() + start;
        let section = &rendered[start..=end];
        assert!(
            !section.contains("hx-post"),
            "re-running a refused window is not this page's job"
        );
    }

    #[test]
    fn the_window_page_escapes_the_request_it_titles() {
        let mut o = outcome(Verdict::Answered {
            html: String::new(),
        });
        o.request = "<script>alert(1)</script>".into();
        let rendered = window(&signed_in("someone@example.com"), saved_at(), &o).into_string();
        assert!(rendered.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    }

    #[test]
    fn a_window_pages_heading_falls_back_to_the_cut_when_unnamed() {
        let o = outcome(Verdict::Answered {
            html: String::new(),
        });
        let request = "x".repeat(80);
        let mut o = o;
        o.request = request.clone();
        let rendered = window(&signed_in("someone@example.com"), saved_at(), &o).into_string();
        let expected = format!("{}…", "x".repeat(60));
        assert!(rendered.contains(&format!(">{expected}</h2>")));
    }

    #[test]
    fn a_window_page_dates_itself_under_the_title() {
        let rendered = window(
            &signed_in("someone@example.com"),
            saved_at(),
            &outcome(Verdict::Answered {
                html: String::new(),
            }),
        )
        .into_string();
        assert!(rendered.contains("Saved 1 Feb 2026"));
        assert!(rendered.contains("data re-read on arrival"));
        assert!(rendered.contains("datetime=\"2026-02-01T10:20:30Z\""));
    }

    #[test]
    fn the_window_result_section_opens_exactly_as_the_ask_fragments_do() {
        // htmx swaps an answer into whichever #ask-result stands where a form
        // posted from, so this page's resting section must be shaped like
        // greeting()'s and answer()'s or the first swap would rewrite it.
        let rendered = window(
            &signed_in("someone@example.com"),
            saved_at(),
            &outcome(Verdict::Answered {
                html: String::new(),
            }),
        )
        .into_string();
        let fragment_opener = r#"<section class="card" id="ask-result">"#;
        assert!(crate::ask::greeting()
            .into_string()
            .starts_with(fragment_opener));
        assert!(rendered.contains(fragment_opener));
    }

    #[test]
    fn the_saved_date_line_carries_its_own_class() {
        let rendered = window(
            &signed_in("someone@example.com"),
            saved_at(),
            &outcome(Verdict::Answered {
                html: String::new(),
            }),
        )
        .into_string();
        assert!(rendered.contains("<p class=\"saved-date\"><time"));
    }

    #[test]
    fn the_saved_date_line_sits_between_the_title_and_the_outcome() {
        let rendered = window(
            &signed_in("someone@example.com"),
            saved_at(),
            &outcome(Verdict::Failed {
                stage: Stage::Query,
            }),
        )
        .into_string();
        let title = rendered.find("</h2>").unwrap();
        let line = rendered.find("data re-read on arrival").unwrap();
        let outcome = rendered.find("ask-failed").unwrap();
        assert!(title < line && line < outcome);
        // No link: a mark that links nowhere is the defect this initiative removes.
        assert!(!rendered[title..outcome].contains("href"));
    }
}
