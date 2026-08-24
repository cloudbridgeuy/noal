//! The ask form and the fragment that replaces it.
//!
//! Both are fragments: htmx swaps them into the page chrome. The filled
//! template arrives as a string the view places verbatim, because the view
//! already rendered it through Tera with autoescape on.

use maud::{html, Markup, PreEscaped};
use noal_core::ask::outcome::{Outcome, Stage, Verdict};

/// The form that starts an ask. Submitting it replaces `#ask-result`, wherever
/// on the page that lives, leaving the form itself in place.
///
/// `hx-sync="this:drop"` drops a second submit made while the first is still
/// in flight, rather than queuing or replacing it — this is what stops a
/// second Enter press from firing another request. `hx-disabled-elt="find
/// button"` disables the submit button for the same span, as the visible
/// sign that an ask is already running; the input is left out on purpose so
/// the user can keep editing their request while it runs.
#[must_use]
pub fn form() -> Markup {
    html! {
        form #ask-form hx-post="/ask" hx-target="#ask-result" hx-swap="outerHTML" hx-indicator="#ask-busy"
            hx-sync="this:drop" hx-disabled-elt="find button" {
            label for="ask-input" { "What do you want to see?" }
            input #ask-input name="request" type="text" required autofocus
                placeholder="open tasks under the Render MVP epic, with comments";
            button type="submit" { "Ask" }
            span #ask-busy .htmx-indicator { "Thinking…" }
        }
    }
}

/// `#ask-result`'s resting state, before any ask has been made.
///
/// Shares its shape with [`answer`]'s rendered section — a `section
/// #ask-result` wrapping a `div.ask-answer` — so the first swap changes only
/// the content, never the element the form targets.
#[must_use]
pub fn greeting() -> Markup {
    html! {
        section #ask-result {
            div .ask-answer { "noal" }
        }
    }
}

/// Whether a rendered answer was saved as a window.
///
/// The three cases are everything that can happen to an answered ask; a
/// failed ask has no window to talk about and never reaches this value —
/// a refusal is a toast carried by the layout, not an answer.
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

/// The answer fragment: the request that produced it, the filled HTML, and —
/// when the window row could not be written — one honest line saying so.
///
/// Takes the request and the filled HTML directly rather than an
/// [`Outcome`], so a refused stage — which has no HTML to show — cannot be
/// passed in here at all. A refusal is a toast, not an answer; the caller
/// decides which to render by matching the verdict before it ever reaches
/// this function.
///
/// The debug payload deliberately does not ride inside the fragment: it is
/// page chrome (`#ask-debug`), replaced out of band by the caller with
/// [`crate::layout::debug_payload`], so it stays current wherever the answer
/// lands.
#[must_use]
pub fn answer(request: &str, html: &str, saved: Saved) -> Markup {
    html! {
        section #ask-result {
            h2 { (request) }
            div .ask-answer { (PreEscaped(html)) }
            @if saved == Saved::No {
                p #ask-toast role="status" { "The window was not saved." }
            }
        }
    }
}

/// What to tell the user when a stage gave up.
///
/// Public because the wording for a refused stage is ask knowledge; the
/// generic toast chrome that carries it lives in `layout`.
#[must_use]
pub const fn failure_text(stage: Stage) -> &'static str {
    match stage {
        Stage::Plan => "noal could not work out which data to fetch.",
        Stage::Query => "noal could not run the query it wrote.",
        Stage::Render => "noal could not design a view for this.",
        Stage::Fill => "noal could not fill its view with the data.",
    }
}

#[cfg(test)]
mod tests {
    use super::{answer, failure_text, form, greeting, outcome_view, Saved};
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
    fn the_form_posts_to_ask_and_swaps_the_result_target() {
        let html = form().into_string();
        assert!(html.contains("hx-post=\"/ask\""));
        assert!(html.contains("hx-target=\"#ask-result\""));
        assert!(html.contains("hx-swap=\"outerHTML\""));
        assert!(html.contains("name=\"request\""));
    }

    #[test]
    fn the_form_drops_a_second_submit_and_disables_only_the_button() {
        let html = form().into_string();
        assert!(html.contains("hx-sync=\"this:drop\""));
        assert!(html.contains("hx-disabled-elt=\"find button\""));

        // The input must stay out of hx-disabled-elt's reach so the user can
        // keep editing their request while the first ask is in flight.
        match html.find("<input").and_then(|start| {
            html[start..]
                .find('>')
                .map(|end| &html[start..=start + end])
        }) {
            Some(input_tag) => assert!(!input_tag.contains("disabled")),
            None => panic!("form renders an input"),
        }
    }

    #[test]
    fn the_greeting_fills_ask_result_with_the_shape_answer_will_reuse() {
        let html = greeting().into_string();
        assert!(html.contains("id=\"ask-result\""));
        assert!(html.contains("class=\"ask-answer\""));
        assert!(html.contains("noal"));
    }

    #[test]
    fn an_answer_places_the_filled_html_verbatim_and_carries_no_debug_element() {
        let html = answer("open tasks", "<ul><li>a</li></ul>", Saved::Yes).into_string();
        assert!(html.contains("<ul><li>a</li></ul>"));
        // The debug payload is chrome, swapped out of band by the caller;
        // embedding it here would duplicate the id it replaces.
        assert!(!html.contains("id=\"ask-debug\""));
        assert!(!html.contains("hx-post"), "no retry form on an answer");
    }

    #[test]
    fn a_saved_answer_carries_no_toast() {
        let html = answer("open tasks", "", Saved::Yes).into_string();
        assert!(!html.contains("not saved"));
    }

    #[test]
    fn an_unsaved_answer_says_so_once() {
        let html = answer("open tasks", "", Saved::No).into_string();
        assert!(html.contains("The window was not saved."));
        assert_eq!(html.matches("The window was not saved.").count(), 1);
    }

    #[test]
    fn the_request_is_escaped() {
        let html = answer("<img src=x>", "", Saved::Yes).into_string();
        assert!(html.contains("<h2>&lt;img src=x&gt;</h2>"));
    }

    #[test]
    fn every_stage_has_its_own_failure_wording() {
        // Distinct wording per stage, so a refusal names what actually gave
        // up rather than a generic "something went wrong".
        let plan = failure_text(Stage::Plan);
        let query = failure_text(Stage::Query);
        let render = failure_text(Stage::Render);
        let fill = failure_text(Stage::Fill);

        assert!(plan.contains("data to fetch"));
        assert!(query.contains("run the query"));
        assert!(render.contains("design a view"));
        assert!(fill.contains("fill its view"));

        let all = [plan, query, render, fill];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a, b);
            }
        }
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
