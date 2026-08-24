//! `GET /w/:id`: one saved window, reopened.
//!
//! The address always answers with a full document — a link from the tree,
//! an htmx-pushed URL, and a plain reload all take the same path. A segment
//! that does not parse as a uuid, an id no row owns, and a row owned by
//! someone else all answer the same 404 through [`Failure::NoSuchWindow`]:
//! an address must reveal nothing about which windows exist or whose they
//! are. The pipeline is handed the stored plan and template up front, so no
//! step of the run reaches the model.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use noal_core::ask::outcome::Outcome;
use noal_core::ask::pipeline::Pipeline;
use noal_core::ask::plan::{Column, Plan};
use noal_core::window::Window;
use noal_view::layout::{Chrome, Viewer};
use noal_view::windows::Current;

use crate::extract::SignedIn;
use crate::failure::Failure;
use crate::respond;
use crate::state::AppState;

/// Reopen one saved window as its own page.
pub async fn show(
    State(state): State<AppState>,
    signed_in: SignedIn,
    Path(id): Path<String>,
) -> Result<Response, Failure> {
    // Parse here rather than in the route pattern: a segment that is not a
    // uuid must get the same 404 as an id nothing owns, not a router error.
    let Ok(id) = uuid::Uuid::parse_str(&id) else {
        return Err(Failure::NoSuchWindow);
    };

    let client = state.database().await?;
    let window = crate::window::find(&client, id, &signed_in.0.user_id)
        .await
        .map_err(Failure::database)?
        .ok_or(Failure::NoSuchWindow)?;

    let outcome = rerun(&state, &window).await?;

    let chrome = Chrome {
        viewer: Viewer::SignedIn {
            email: signed_in.0.email.clone(),
        },
        windows: crate::chrome::build(&state, &signed_in.0.user_id).await,
        current: Current::Window(window.id),
    };
    Ok(respond::html(
        StatusCode::OK,
        noal_view::pages::window(&chrome, &outcome),
    ))
}

/// Run the window's stored query through its stored template.
async fn rerun(state: &AppState, window: &Window) -> Result<Outcome, Failure> {
    let (pipeline, steps) = Pipeline::reopen(
        window.request.clone(),
        stored_plan(window)?,
        window.template.clone(),
    );
    crate::routes::ask::drive(pipeline, steps, state).await
}

/// Rebuild the plan the window saved.
///
/// The shape column stores the plan's columns as JSON. A row whose shape no
/// longer parses cannot be re-run, and there is no honest page to show for
/// it — so it is simply not found, with the cause in the log.
fn stored_plan(window: &Window) -> Result<Plan, Failure> {
    let shape: Vec<Column> = serde_json::from_value(window.shape.clone()).map_err(|error| {
        worker::console_error!("stored shape unreadable: {error}");
        Failure::NoSuchWindow
    })?;
    Ok(Plan {
        sql: window.sql.clone(),
        shape,
    })
}
