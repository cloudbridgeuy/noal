//! The ask form and the fragment that replaces it.
//!
//! Both are fragments: htmx swaps them into the page chrome. The filled
//! template arrives as a string the view places verbatim, because the view
//! already rendered it through Tera with autoescape on.

use maud::{html, Markup, PreEscaped};
use noal_core::ask::outcome::{Outcome, Stage, Verdict};

/// The form that starts an ask. Submitting it replaces it with the answer.
#[must_use]
pub fn form() -> Markup {
    html! {
        form #ask-form hx-post="/ask" hx-target="this" hx-swap="outerHTML" hx-indicator="#ask-busy" {
            label for="ask-input" { "What do you want to see?" }
            input #ask-input name="request" type="text" required autofocus
                placeholder="open tasks under the Render MVP epic, with comments";
            button type="submit" { "Ask" }
            span #ask-busy .htmx-indicator { "Thinking…" }
        }
    }
}

/// Whether a rendered answer was saved as a window.
///
/// The three cases are everything that can happen to an answered ask; a
/// failed ask has no window to talk about and never receives this value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Saved {
    /// The window row was written. The answer carries its id out of band.
    Yes,
    /// The answer rendered, but writing the window row failed. The user gets
    /// the answer anyway, plus one honest line about the save.
    No,
}

/// The part of a result that shows the outcome itself: the filled page for
/// an answer, the plain refusal line for a failure.
///
/// The window page reuses this so reopening renders exactly what asking
/// rendered. What differs between asking and reopening — the retry form and
/// the save toast — is left to the callers.
#[must_use]
pub fn outcome_view(outcome: &Outcome) -> Markup {
    html! {
        @match &outcome.verdict {
            Verdict::Answered { html } => {
                div .ask-answer { (PreEscaped(html)) }
            }
            Verdict::Failed { stage } => {
                p .ask-failed { (failure_text(*stage)) }
            }
        }
    }
}

/// The answer fragment: the request, the result or a failure, and the debug
/// payload a later overlay reads.
///
/// `saved` only matters for an answered verdict — a failed ask saved nothing
/// by definition, so the value is ignored there. The failed-save toast lives
/// here rather than in the palette because the palette's own toast region
/// does not exist yet.
#[must_use]
pub fn answer(outcome: &Outcome, saved: Saved) -> Markup {
    html! {
        section #ask-result {
            h2 { (outcome.request) }
            (outcome_view(outcome))
            @match &outcome.verdict {
                Verdict::Answered { .. } => {
                    @if saved == Saved::No {
                        p #ask-toast role="status" { "The window was not saved." }
                    }
                }
                Verdict::Failed { .. } => {
                    (form())
                }
            }
            script #ask-debug type="application/json" { (PreEscaped(outcome.debug_json())) }
        }
    }
}

/// What to tell the user when a stage gave up.
const fn failure_text(stage: Stage) -> &'static str {
    match stage {
        Stage::Plan => "noal could not work out which data to fetch.",
        Stage::Query => "noal could not run the query it wrote.",
        Stage::Render => "noal could not design a view for this.",
        Stage::Fill => "noal could not fill its view with the data.",
    }
}

#[cfg(test)]
mod tests {
    use super::{answer, form, outcome_view, Saved};
    use noal_core::ask::outcome::{Debug, Origin, Outcome, Stage, Verdict};

    fn outcome(verdict: Verdict) -> Outcome {
        Outcome {
            request: "open tasks".into(),
            verdict,
            origin: Origin::Asked,
            debug: Debug::default(),
        }
    }

    #[test]
    fn the_form_posts_to_ask_and_swaps_itself() {
        let html = form().into_string();
        assert!(html.contains("hx-post=\"/ask\""));
        assert!(html.contains("hx-swap=\"outerHTML\""));
        assert!(html.contains("name=\"request\""));
    }

    #[test]
    fn an_answer_places_the_filled_html_verbatim_and_carries_debug_json() {
        let html = answer(
            &outcome(Verdict::Answered {
                html: "<ul><li>a</li></ul>".into(),
            }),
            Saved::Yes,
        )
        .into_string();
        assert!(html.contains("<ul><li>a</li></ul>"));
        assert!(html.contains("id=\"ask-debug\""));
        assert!(html.contains("\"request\":\"open tasks\""));
        assert!(!html.contains("hx-post"));
    }

    #[test]
    fn a_saved_answer_carries_no_toast() {
        let html = answer(
            &outcome(Verdict::Answered {
                html: String::new(),
            }),
            Saved::Yes,
        )
        .into_string();
        assert!(!html.contains("not saved"));
    }

    #[test]
    fn an_unsaved_answer_says_so_once() {
        let html = answer(
            &outcome(Verdict::Answered {
                html: String::new(),
            }),
            Saved::No,
        )
        .into_string();
        assert!(html.contains("The window was not saved."));
    }

    #[test]
    fn a_failed_ask_never_mentions_saving() {
        let html = answer(
            &outcome(Verdict::Failed {
                stage: Stage::Query,
            }),
            Saved::No,
        )
        .into_string();
        assert!(html.contains("could not run the query"));
        assert!(html.contains("hx-post=\"/ask\""));
        assert!(!html.contains("not saved"));
    }

    #[test]
    fn the_request_is_escaped() {
        let mut o = outcome(Verdict::Answered {
            html: String::new(),
        });
        o.request = "<img src=x>".into();
        let html = answer(&o, Saved::Yes).into_string();
        assert!(html.contains("<h2>&lt;img src=x&gt;</h2>"));
    }

    #[test]
    fn the_outcome_view_shows_the_filled_page_verbatim() {
        let html = outcome_view(&outcome(Verdict::Answered {
            html: "<ul><li>a</li></ul>".into(),
        }))
        .into_string();
        assert!(html.contains("<div class=\"ask-answer\"><ul><li>a</li></ul></div>"));
        assert!(!html.contains("hx-post"), "no retry form on an answer");
    }

    #[test]
    fn the_outcome_view_shows_a_plain_refusal_line_for_a_failure() {
        let html = outcome_view(&outcome(Verdict::Failed { stage: Stage::Fill })).into_string();
        assert!(html.contains("could not fill its view"));
        assert!(
            !html.contains("hx-post"),
            "the retry form belongs to the caller"
        );
    }
}
