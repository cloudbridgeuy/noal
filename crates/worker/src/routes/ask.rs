//! `POST /ask`: one request in, one fragment out.
//!
//! [`noal_core::ask::pipeline::Pipeline`] decides what happens next; this
//! module only runs the [`Step`]s it asks for and feeds back an [`Event`] for
//! each result. A query and a render are asked for together, so they run
//! concurrently with `try_join` — render is the slow one, and there is no
//! reason to wait for it before asking Postgres.

use std::future::Future;

use axum::extract::State;
use axum::response::Html;
use axum::Form;
use noal_core::ask::outcome::{Outcome, Stage, Timing};
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
) -> Result<Html<String>, Failure> {
    let outcome = run(&state, form.request.trim().to_owned()).await?;
    let mut body = noal_view::ask::answer(&outcome).into_string();
    // The debug payload rides beside the answer as a top-level, out-of-band
    // element: htmx only processes `hx-swap-oob` at the top of the response,
    // not nested inside the swapped fragment.
    body.push_str(&noal_view::layout::debug_payload(&outcome).into_string());
    Ok(Html(body))
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
