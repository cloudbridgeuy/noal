//! `POST /ask`: one request in, one fragment out.
//!
//! [`noal_core::ask::pipeline::Pipeline`] decides what happens next; this
//! module only runs the [`Step`]s it asks for and feeds back an [`Event`] for
//! each result. A query and a render are asked for together, so they run
//! concurrently with `try_join` — render is the slow one, and there is no
//! reason to wait for it before asking Postgres.

use std::future::Future;

use axum::extract::State;
use axum::http::{HeaderName, HeaderValue};
use axum::response::{Html, IntoResponse, Response};
use axum::Form;
use noal_core::ask::outcome::{Outcome, Stage, Timing, Verdict};
use noal_core::ask::pipeline::{Event, Pipeline, Step};
use noal_core::ask::plan::Plan;
use noal_core::ask::prompt::{strip_fences, PLAN_PREAMBLE, RENDER_PREAMBLE};
use serde::Deserialize;
use tokio_postgres::SimpleQueryMessage;

use crate::extract::SignedIn;
use crate::failure::Failure;
use crate::llm;
use crate::state::{now_millis, AppState};

/// The form field the ask form posts.
#[derive(Debug, Deserialize)]
pub struct AskForm {
    /// What the user typed.
    pub request: String,
}

/// Run the pipeline and render whatever it produced.
///
/// # Errors
///
/// Returns a [`Failure`] only for transport-level trouble: the model or the
/// database unreachable. A stage the pipeline refuses is not a failure; it is
/// an [`Outcome`] the view explains.
pub async fn ask(
    State(state): State<AppState>,
    _signed_in: SignedIn,
    Form(form): Form<AskForm>,
) -> Result<Response, Failure> {
    let outcome = run(&state, form.request.trim().to_owned()).await?;
    Ok(render_outcome(&outcome))
}

/// Render one [`Outcome`]'s body: the fragment for its verdict, plus its
/// debug payload riding along out of band.
///
/// Kept apart from header choice and from the handler so a native test can
/// check what each verdict renders by comparing strings, same as any other
/// view call.
fn render_body(outcome: &Outcome) -> String {
    let mut body = match &outcome.verdict {
        Verdict::Answered { html } => noal_view::ask::answer(&outcome.request, html).into_string(),
        Verdict::Failed { stage } => {
            noal_view::layout::toast(noal_view::ask::failure_text(*stage)).into_string()
        }
    };
    // The debug payload rides beside the main fragment as a top-level,
    // out-of-band element: htmx only processes `hx-swap-oob` at the top of
    // the response, not nested inside whatever the response retargets to.
    body.push_str(&noal_view::layout::debug_payload(outcome).into_string());
    body
}

/// Render one [`Outcome`] into the response the browser receives.
///
/// Kept separate from the handler, and taking no state or request, so the
/// header choice a refusal makes — retargeting the swap at `#toasts` instead
/// of `#ask-result` — is something a native test can pin rather than
/// something only read by eye.
///
/// An answered outcome swaps `#ask-result` as always. A refused stage still
/// answers `200`: that keeps the debug payload travelling the normal swap
/// path instead of the error path, and it is what leaves the previous answer
/// on screen, since `HX-Retarget`/`HX-Reswap` steer the swap away from
/// `#ask-result` entirely, appending the toast to `#toasts` instead.
fn render_outcome(outcome: &Outcome) -> Response {
    let body = render_body(outcome);
    match outcome.verdict {
        Verdict::Answered { .. } => Html(body).into_response(),
        Verdict::Failed { .. } => {
            let mut response = Html(body).into_response();
            let headers = response.headers_mut();
            headers.insert(
                HeaderName::from_static("hx-retarget"),
                HeaderValue::from_static("#toasts"),
            );
            headers.insert(
                HeaderName::from_static("hx-reswap"),
                HeaderValue::from_static("beforeend"),
            );
            response
        }
    }
}

/// Drive the pipeline to completion.
///
/// Pops whatever steps the pipeline hands back, runs them, and feeds the
/// results in as events. A `Query` and a `Render` step, when both are
/// pending at once, are the only pair this pipeline ever issues together, so
/// they are the only pair run concurrently; every other step runs alone.
/// Nothing here decides what a result means — that is
/// [`Pipeline::apply`]'s job.
async fn run(state: &AppState, request: String) -> Result<Outcome, Failure> {
    let (mut pipeline, mut steps) = Pipeline::start(request);

    loop {
        let mut events = Vec::new();
        let mut pending_query: Option<String> = None;
        let mut pending_render: Option<String> = None;

        for step in steps {
            match step {
                Step::Plan { prompt } => {
                    let (plan, timing) = timed(
                        Stage::Plan,
                        llm::structured::<Plan>(state.config(), PLAN_PREAMBLE, prompt),
                    )
                    .await?;
                    pipeline.record(timing);
                    events.push(Event::Planned(plan));
                }
                Step::Query { sql } => pending_query = Some(sql),
                Step::Render { prompt } => pending_render = Some(prompt),
                Step::Fill { template, rows } => {
                    let start = now_millis();
                    let result = noal_view::render::fill(&template, &rows)
                        .map_err(|error| error.to_string());
                    pipeline.record(Timing {
                        stage: Stage::Fill,
                        millis: now_millis().saturating_sub(start),
                    });
                    events.push(Event::Filled(result));
                }
                Step::Done(outcome) => return Ok(outcome),
            }
        }

        run_pending(
            state,
            pending_query,
            pending_render,
            &mut events,
            &mut pipeline,
        )
        .await?;

        steps = events
            .into_iter()
            .flat_map(|event| pipeline.apply(event))
            .collect();
    }
}

/// Run whichever of a pending query and a pending render this round holds,
/// joining the two when both are present.
///
/// A query and a render that run together overlap in wall clock time, so
/// their two [`Timing`]s do not sum to how long the pair actually took; each
/// is measured from its own start to its own finish, which is what the debug
/// panel is documented to show.
async fn run_pending(
    state: &AppState,
    query: Option<String>,
    render: Option<String>,
    events: &mut Vec<Event>,
    pipeline: &mut Pipeline,
) -> Result<(), Failure> {
    match (query, render) {
        (Some(sql), Some(prompt)) => {
            let ((queried, query_timing), (rendered, render_timing)) =
                futures_util::future::try_join(
                    timed(Stage::Query, execute(state, &sql)),
                    timed(Stage::Render, render_call(state, prompt)),
                )
                .await?;
            pipeline.record(query_timing);
            pipeline.record(render_timing);
            events.push(Event::Queried(queried));
            events.push(Event::Rendered(rendered));
        }
        (Some(sql), None) => {
            let (queried, timing) = timed(Stage::Query, execute(state, &sql)).await?;
            pipeline.record(timing);
            events.push(Event::Queried(queried));
        }
        (None, Some(prompt)) => {
            let (rendered, timing) = timed(Stage::Render, render_call(state, prompt)).await?;
            pipeline.record(timing);
            events.push(Event::Rendered(rendered));
        }
        (None, None) => {}
    }
    Ok(())
}

/// Measure how long a fallible step took, in milliseconds.
///
/// The clock is read only here, in the shell; `noal_core::ask::pipeline`
/// only ever receives the resulting [`Timing`] as a plain value.
async fn timed<F, T, E>(stage: Stage, future: F) -> Result<(T, Timing), E>
where
    F: Future<Output = Result<T, E>>,
{
    let start = now_millis();
    let value = future.await?;
    let millis = now_millis().saturating_sub(start);
    Ok((value, Timing { stage, millis }))
}

/// Ask the model for a template and drop any surrounding code fence.
async fn render_call(state: &AppState, prompt: String) -> Result<String, Failure> {
    let text = llm::text(state.config(), RENDER_PREAMBLE, prompt).await?;
    Ok(strip_fences(&text))
}

/// Run already-wrapped SQL inside a read-only transaction and parse the JSON.
///
/// The outer `Result` is transport: no connection, no answer. The inner
/// `Result` is Postgres refusing the SQL, with its message, which is what the
/// pipeline records as the reason the query stage failed.
async fn execute(
    state: &AppState,
    sql: &str,
) -> Result<Result<serde_json::Value, String>, Failure> {
    let client = state.database().await?;
    client
        .simple_query("begin read only")
        .await
        .map_err(Failure::database)?;

    let result = client.simple_query(sql).await;

    // End the transaction either way; a pooled connection must not be handed
    // back mid-transaction. The outcome of the rollback does not matter.
    let _ = client.simple_query("rollback").await;

    let messages = match result {
        Ok(messages) => messages,
        Err(error) => {
            // A Postgres error (bad SQL) is the model's to fix. Anything else
            // (socket closed) is a transport failure.
            return match error.as_db_error() {
                Some(db_error) => Ok(Err(db_error.message().to_owned())),
                None => Err(Failure::database(error)),
            };
        }
    };

    let text = messages
        .iter()
        .find_map(|message| match message {
            SimpleQueryMessage::Row(row) => row.get(0).map(str::to_owned),
            _ => None,
        })
        .ok_or_else(|| Failure::Database("the query returned no row".to_owned()))?;

    serde_json::from_str(&text)
        .map_err(Failure::database)
        .map(Ok)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use axum::http::StatusCode;
    use noal_core::ask::outcome::{Debug, Outcome, Verdict};

    use super::{render_body, render_outcome, Stage};

    fn outcome(verdict: Verdict) -> Outcome {
        Outcome {
            request: "open tasks".into(),
            verdict,
            debug: Debug::default(),
        }
    }

    #[test]
    fn an_answered_outcome_renders_the_answer_and_its_debug_payload() {
        let body = render_body(&outcome(Verdict::Answered {
            html: "<ul><li>a</li></ul>".into(),
        }));
        assert!(body.contains("<ul><li>a</li></ul>"));
        assert!(body.contains("id=\"ask-debug\""));
        assert!(!body.contains("id=\"toasts\""));
    }

    #[test]
    fn a_refused_outcome_renders_a_toast_and_its_debug_payload() {
        let body = render_body(&outcome(Verdict::Failed {
            stage: Stage::Query,
        }));
        assert!(body.contains("could not run the query"));
        assert!(body.contains("class=\"toast\""));
        assert!(body.contains("id=\"ask-debug\""));
        assert!(!body.contains("id=\"ask-result\""));
    }

    #[test]
    fn an_answered_response_sets_no_retarget_headers_and_answers_200() {
        let response = render_outcome(&outcome(Verdict::Answered {
            html: String::new(),
        }));
        assert_eq!(response.status(), StatusCode::OK);
        assert!(!response.headers().contains_key("hx-retarget"));
        assert!(!response.headers().contains_key("hx-reswap"));
    }

    #[test]
    fn a_refused_response_retargets_the_swap_to_toasts_and_still_answers_200() {
        let response = render_outcome(&outcome(Verdict::Failed { stage: Stage::Plan }));
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get("hx-retarget").unwrap(), "#toasts");
        assert_eq!(response.headers().get("hx-reswap").unwrap(), "beforeend");
    }
}
