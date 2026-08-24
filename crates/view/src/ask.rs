//! The ask form and the fragment that replaces it.
//!
//! Both are fragments: htmx swaps them into the page chrome. The filled
//! template arrives as a string the view places verbatim, because the view
//! already rendered it through Tera with autoescape on.

use maud::{html, Markup, PreEscaped};
use noal_core::ask::outcome::{Outcome, Stage, Verdict};

/// The form that starts an ask. Submitting it replaces `#ask-result`, wherever
/// on the page that lives, leaving the form itself in place.
#[must_use]
pub fn form() -> Markup {
    html! {
        form #ask-form hx-post="/ask" hx-target="#ask-result" hx-swap="outerHTML" hx-indicator="#ask-busy" {
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
/// Shares its shape with [`answer`]'s answered arm — a `section #ask-result`
/// wrapping a `div.ask-answer` — so the first swap changes only the content,
/// never the element the form targets.
#[must_use]
pub fn greeting() -> Markup {
    html! {
        section #ask-result {
            div .ask-answer { "noal" }
        }
    }
}

/// The answer fragment: the request, the result or a failure, and the debug
/// payload a later overlay reads.
#[must_use]
pub fn answer(outcome: &Outcome) -> Markup {
    html! {
        section #ask-result {
            h2 { (outcome.request) }
            @match &outcome.verdict {
                Verdict::Answered { html } => {
                    div .ask-answer { (PreEscaped(html)) }
                }
                Verdict::Failed { stage } => {
                    p .ask-failed { (failure_text(*stage)) }
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
    use super::{answer, form, greeting};
    use noal_core::ask::outcome::{Debug, Outcome, Stage, Verdict};

    fn outcome(verdict: Verdict) -> Outcome {
        Outcome {
            request: "open tasks".into(),
            verdict,
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
    fn the_greeting_fills_ask_result_with_the_shape_answer_will_reuse() {
        let html = greeting().into_string();
        assert!(html.contains("id=\"ask-result\""));
        assert!(html.contains("class=\"ask-answer\""));
        assert!(html.contains("noal"));
    }

    #[test]
    fn an_answer_places_the_filled_html_verbatim_and_carries_debug_json() {
        let html = answer(&outcome(Verdict::Answered {
            html: "<ul><li>a</li></ul>".into(),
        }))
        .into_string();
        assert!(html.contains("<ul><li>a</li></ul>"));
        assert!(html.contains("id=\"ask-debug\""));
        assert!(html.contains("\"request\":\"open tasks\""));
        assert!(!html.contains("hx-post"));
    }

    #[test]
    fn a_failure_explains_the_stage_and_offers_a_fresh_form() {
        let html = answer(&outcome(Verdict::Failed {
            stage: Stage::Query,
        }))
        .into_string();
        assert!(html.contains("could not run the query"));
        assert!(html.contains("hx-post=\"/ask\""));
    }

    #[test]
    fn the_request_is_escaped() {
        let mut o = outcome(Verdict::Answered {
            html: String::new(),
        });
        o.request = "<img src=x>".into();
        let html = answer(&o).into_string();
        assert!(html.contains("<h2>&lt;img src=x&gt;</h2>"));
    }
}
