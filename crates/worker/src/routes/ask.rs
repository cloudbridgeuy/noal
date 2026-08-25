//! `POST /ask`: one request in, one fragment out.
//!
//! [`noal_core::ask::pipeline::Pipeline`] decides what happens next; this
//! module only runs the [`Step`]s it asks for and feeds back an [`Event`] for
//! each result. A query and a render are asked for together, so they run
//! concurrently with `try_join` — render is the slow one, and there is no
//! reason to wait for it before asking Postgres.
//!
//! Every answer this handler produces carries its debug payload out of band,
//! so the palette's Debug tab stays current whichever verdict was reached. A
//! refused stage answers `200` with its toast retargeted into `#toasts`,
//! leaving the previous answer — and the typed request — alone on screen. An
//! answered ask is then saved as a window: one parameterized insert, and on
//! success the response carries a fresh tree out of band plus the window's
//! URL for htmx to push. A failed save manufactures no URL — pushing `/w/:id`
//! for a missing row would hand back a button and a reload that both 404 —
//! so the answer goes back alone with one honest line instead.

use std::future::Future;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::Form;
use maud::{html, Markup};
use noal_core::ask::outcome::{Outcome, Stage, Timing, Verdict};
use noal_core::ask::pipeline::{Event, Pipeline, Step};
use noal_core::ask::plan::Plan;
use noal_core::ask::prompt::{strip_fences, PLAN_PREAMBLE, RENDER_PREAMBLE};
use noal_core::session::SessionClaims;
use noal_core::window::Window;
use noal_view::ask::Saved;
use noal_view::windows::Current;
use serde::Deserialize;
use tokio_postgres::SimpleQueryMessage;

use crate::entropy;
use crate::extract::{Fragment, SignedIn};
use crate::failure::Failure;
use crate::llm;
use crate::respond;
use crate::state::{now_millis, AppState};

/// The form field the ask form posts.
#[derive(Debug, Deserialize)]
pub struct AskForm {
    /// What the user typed.
    pub request: String,
}

/// Run the pipeline and render whatever it produced.
///
/// Names [`Fragment<SignedIn>`] rather than [`SignedIn`] directly, so an
/// absent or stale session still refuses the request — the same guarantee
/// [`crate::extract`] documents — but the refusal renders as a toast rather
/// than a whole document landing inside `#ask-result`.
///
/// A transport-level `Failure` — the model or the database unreachable — is
/// handled the same way, through [`Failure::toast`], rather than returned:
/// every non-`200` this handler can produce must carry a toast body, and a
/// plain `Response` return type is what makes that total rather than a
/// convention a future caller could forget. A stage the pipeline refuses is
/// not a failure at all; it is an [`Outcome`] the view explains, still
/// answering `200`.
pub async fn ask(
    State(state): State<AppState>,
    Fragment(signed_in): Fragment<SignedIn>,
    Form(form): Form<AskForm>,
) -> Response {
    match run(&state, form.request.trim().to_owned()).await {
        Ok(outcome) => match &outcome.verdict {
            Verdict::Answered { html } => {
                answered_response(&state, &signed_in.0, &outcome, html).await
            }
            Verdict::Failed { stage } => refused_response(&outcome, *stage),
        },
        Err(failure) => failure.toast(),
    }
}

/// Everything an answered ask swaps in: the answer itself, the debug payload
/// out of band, and — when the window row was written — the refreshed tree
/// out of band too.
///
/// Kept pure, apart from header choice and from saving, so a native test can
/// check what each save outcome renders by comparing strings, same as any
/// other view call.
fn answered_body(outcome: &Outcome, filled: &str, saved: Saved, tree: Option<Markup>) -> Markup {
    html! {
        (noal_view::ask::answer(&outcome.request, filled, saved))
        @if let Some(tree) = tree {
            (tree)
        }
        (noal_view::layout::debug_payload(outcome))
    }
}

/// The response to an answered ask: save it, then decorate the answer.
///
/// Saved: `200`, the refreshed tree out of band, `HX-Push-Url` pointing at
/// the window, and `HX-Trigger: noal:answered` telling the palette to close
/// and clear. Unsaved: the same answer minus the address and the tree, with
/// one honest line about the save — the trigger still rides, because the
/// pipeline did produce an answer; only the save failed.
async fn answered_response(
    state: &AppState,
    viewer: &SessionClaims,
    outcome: &Outcome,
    filled: &str,
) -> Response {
    match save_window(state, viewer, outcome).await {
        Ok(id) => {
            let windows = crate::chrome::build(state, &viewer.user_id).await;
            let body = answered_body(
                outcome,
                filled,
                Saved::Yes,
                Some(noal_view::windows::oob_tree(&windows, &Current::Window(id))),
            );
            respond::with(
                StatusCode::OK,
                body,
                &[
                    ("HX-Push-Url", format!("/w/{id}")),
                    ("HX-Trigger", "noal:answered".to_owned()),
                ],
            )
        }
        Err(detail) => {
            worker::console_error!("window not saved: {detail}");
            let body = answered_body(outcome, filled, Saved::No, None);
            respond::with(
                StatusCode::OK,
                body,
                &[("HX-Trigger", "noal:answered".to_owned())],
            )
        }
    }
}

/// The body of a refused stage: the refusal as a toast, plus the debug
/// payload riding along out of band — a refused stage still has attempts and
/// timings worth showing.
fn refused_body(outcome: &Outcome, stage: Stage) -> Markup {
    html! {
        (noal_view::layout::toast(noal_view::ask::failure_text(
            stage,
            noal_core::ask::outcome::Origin::Asked,
        )))
        (noal_view::layout::debug_payload(outcome))
    }
}

/// The response to a refused pipeline stage.
///
/// It still answers `200`: that keeps the debug payload travelling the normal
/// swap path instead of the error path, and it is what leaves the previous
/// answer on screen, since `HX-Retarget`/`HX-Reswap` steer the swap away from
/// `#ask-result` entirely, appending the toast to `#toasts` instead.
///
/// No `HX-Trigger` rides along: its absence is the whole mechanism that keeps
/// the palette open with the typed text intact.
fn refused_response(outcome: &Outcome, stage: Stage) -> Response {
    respond::with(
        StatusCode::OK,
        refused_body(outcome, stage),
        &[
            ("HX-Retarget", "#toasts".to_owned()),
            ("HX-Reswap", "beforeend".to_owned()),
        ],
    )
}

/// Write one answered ask away as a window row.
///
/// Every step here can fail — entropy refused, database unreachable, insert
/// refused — and every failure is equivalent: the window was not saved.
///
/// # Errors
///
/// Returns text for the log only; the browser is told through the view, not
/// through an error variant, so the wording stays honest about what happened.
async fn save_window(
    state: &AppState,
    viewer: &SessionClaims,
    outcome: &Outcome,
) -> Result<uuid::Uuid, String> {
    let id = entropy::window_id().map_err(|failure| failure.detail())?;

    let window = Window::answered(
        id,
        &viewer.user_id,
        &outcome.request,
        outcome.debug.plan.as_ref(),
        outcome.debug.template.as_deref(),
    )
    .ok_or_else(|| "an answered ask held no plan and template".to_owned())?;

    let client = state.database().await.map_err(|failure| failure.detail())?;
    crate::window::insert(&client, &window).await?;
    Ok(id)
}

/// Drive the pipeline to completion from a fresh start.
///
/// Pops whatever steps the pipeline hands back, runs them, and feeds the
/// results in as events.
async fn run(state: &AppState, request: String) -> Result<Outcome, Failure> {
    let (pipeline, steps) = Pipeline::start(request);
    drive(pipeline, steps, state).await
}

/// Drive any pipeline — started fresh or reopened — to completion.
///
/// Pops whatever steps the pipeline hands back, runs them, and feeds the
/// results in as events. A `Query` and a `Render` step, when both are
/// pending at once, are the only pair this pipeline ever issues together, so
/// they are the only pair run concurrently; every other step runs alone.
/// Nothing here decides what a result means — that is
/// [`Pipeline::apply`]'s job.
pub(crate) async fn drive(
    mut pipeline: Pipeline,
    mut steps: Vec<Step>,
    state: &AppState,
) -> Result<Outcome, Failure> {
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
    use super::{answered_body, refused_body, refused_response, Saved};
    use axum::http::StatusCode;
    use noal_core::ask::outcome::{Debug, Origin, Outcome, Stage, Verdict};
    use noal_view::windows::{Current, Windows};

    fn outcome(verdict: Verdict) -> Outcome {
        Outcome {
            request: "open tasks".into(),
            verdict,
            origin: Origin::Asked,
            debug: Debug::default(),
        }
    }

    #[test]
    fn a_refused_outcome_renders_a_toast_and_its_debug_payload() {
        let body = refused_body(
            &outcome(Verdict::Failed {
                stage: Stage::Query,
            }),
            Stage::Query,
        )
        .into_string();
        assert!(body.contains("could not run the query"));
        assert!(body.contains("class=\"toast\""));
        assert!(body.contains("id=\"ask-debug\""));
        assert!(!body.contains("id=\"ask-result\""));
    }

    #[test]
    fn a_refused_response_retargets_the_swap_to_toasts_and_still_answers_200() {
        let response = refused_response(
            &outcome(Verdict::Failed { stage: Stage::Plan }),
            Stage::Plan,
        );
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers().get("hx-retarget").unwrap(), "#toasts");
        assert_eq!(response.headers().get("hx-reswap").unwrap(), "beforeend");
    }

    #[test]
    fn a_refused_response_sends_no_answered_trigger() {
        let response = refused_response(
            &outcome(Verdict::Failed { stage: Stage::Plan }),
            Stage::Plan,
        );
        assert!(!response.headers().contains_key("hx-trigger"));
    }

    #[test]
    fn a_saved_answer_carries_the_tree_and_no_save_warning() {
        let outcome = outcome(Verdict::Answered {
            html: "<ul><li>a</li></ul>".into(),
        });
        let body = answered_body(
            &outcome,
            "<ul><li>a</li></ul>",
            Saved::Yes,
            Some(noal_view::windows::oob_tree(
                &Windows::Tree(Vec::new()),
                &Current::Home,
            )),
        )
        .into_string();

        assert!(body.contains("<ul><li>a</li></ul>"));
        // The fresh tree rides out of band, marked for htmx to swap into the
        // palette's Windows tab.
        assert!(body.contains(r#"<nav id="window-tree" hx-swap-oob="outerHTML">"#));
        assert!(body.contains("id=\"ask-debug\""));
        assert!(!body.contains("not saved"));
    }

    #[test]
    fn an_unsaved_answer_carries_the_honest_line_but_no_tree() {
        let outcome = outcome(Verdict::Answered {
            html: "<ul><li>a</li></ul>".into(),
        });
        let body = answered_body(&outcome, "<ul><li>a</li></ul>", Saved::No, None).into_string();

        assert!(body.contains("The window was not saved."));
        // No save, no address: pushing /w/:id would 404 on reload.
        assert!(!body.contains("hx-swap-oob=\"outerHTML\"><nav"));
        assert!(!body.contains("id=\"window-tree\""));
        // The debug payload still travels; the answer happened either way.
        assert!(body.contains("id=\"ask-debug\""));
    }

    #[test]
    fn an_answered_body_swaps_into_ask_result_rather_than_the_palette() {
        let outcome = outcome(Verdict::Answered {
            html: String::new(),
        });
        let body = answered_body(&outcome, "", Saved::Yes, None).into_string();
        assert!(body.contains("id=\"ask-result\""));
        // Closing the palette early cannot disturb a swap landing outside it.
        assert!(
            !body.contains("class=\"toast\""),
            "an answer is not a failure notice"
        );
    }
}
