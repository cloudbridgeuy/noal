//! `GET /w/:id` and the write leg beside it, `POST /w/:id/name`.
//!
//! The address always answers with a full document — a link from the tree,
//! an htmx-pushed URL, and a plain reload all take the same path. A segment
//! that does not parse as a uuid, an id no row owns, and a row owned by
//! someone else all answer the same 404 through [`Failure::NoSuchWindow`]:
//! an address must reveal nothing about which windows exist or whose they
//! are. The pipeline is handed the stored plan and template up front, so no
//! step of the run reaches the model.
//!
//! The rename route is the palette's one window write. It answers with the
//! fresh tree — never a page, never a redirect — so the swap happens inside
//! the still-open palette.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::Form;
use maud::{html, Markup};
use noal_core::ask::outcome::Outcome;
use noal_core::ask::pipeline::Pipeline;
use noal_core::ask::plan::{Column, Plan};
use noal_core::window::{normalize_name, Window};
use noal_view::layout::{Chrome, Viewer};
use noal_view::windows::{tree, Current};
use serde::Deserialize;

use crate::extract::{Fragment, SignedIn};
use crate::failure::Failure;
use crate::respond;
use crate::state::AppState;

/// Reopen one saved window as its own page.
pub async fn show(
    State(state): State<AppState>,
    signed_in: SignedIn,
    Path(id): Path<String>,
) -> Result<Response, Failure> {
    let id = parse_id(&id)?;

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
        // The page arrives already carrying its answer, so the palette's
        // Debug tab opens showing what produced it.
        debug_json: Some(outcome.debug_json()),
    };
    Ok(respond::html(
        StatusCode::OK,
        noal_view::pages::window(&chrome, &outcome),
    ))
}

/// The form fields the rename form posts.
#[derive(Debug, Deserialize)]
pub struct NameForm {
    /// What the viewer typed. Empty clears the stored name.
    pub name: Option<String>,
    /// Present only on the form of the window being viewed, so the fresh
    /// tree can carry that fact back out of band.
    pub current_window: Option<String>,
}

/// Store a viewer's name for one window and answer with the fresh tree.
///
/// A refused name — too long — answers as a toast for now; the palette
/// stays open through every rename path. Success swaps `#window-tree` in
/// place: no page load, no closing drawer.
///
/// When the form carried the current-window marker — it was the row of the
/// window being viewed — the same response also carries that window's
/// heading out of band, so the standing page's `<h2>` takes the new label in
/// the same swap as the tree.
///
/// [`Failure::toast`] rides [`Fragment<SignedIn>`] so an absent session is
/// a toast inside the open palette rather than a whole document landing
/// where the tree should be.
pub async fn rename(
    State(state): State<AppState>,
    Fragment(signed_in): Fragment<SignedIn>,
    Path(id): Path<String>,
    Form(form): Form<NameForm>,
) -> Response {
    match store(&state, &signed_in.0, &id, &form).await {
        Ok(current) => {
            let windows = crate::chrome::build(&state, &signed_in.0.user_id).await;
            // The form targets `#window-tree`, so the fresh tree is the
            // ordinary swap content — not an out-of-band rider like the
            // ask's copy of it. The heading rides out of band only when the
            // renamed window is the one being viewed; on Home there is no
            // standing heading to update.
            let body = if current == Current::Home {
                // No marker posted — a Home rename. No standing heading to
                // update, so nothing rides out of band.
                tree(&windows, &current)
            } else {
                renamed_body(&windows, &current)
            };
            respond::html(StatusCode::OK, body)
        }
        Err(failure) => failure.toast(),
    }
}

/// The fresh tree with the renamed window's heading appended beside it.
///
/// The entry comes from the tree just read back from the store, so the
/// heading renders name-or-cut from exactly the stored value the row shows —
/// never a guess assembled from the request.
fn renamed_body(windows: &noal_view::windows::Windows, current: &Current) -> Markup {
    let Current::Window(id) = current else {
        return tree(windows, current);
    };
    let entry = find_entry(windows, *id);
    html! {
        (tree(windows, current))
        @if let Some(entry) = entry {
            (noal_view::ask::oob_heading(&entry))
        }
    }
}

/// Find one window's entry anywhere in the tree, or `None` when it is gone.
fn find_entry(
    windows: &noal_view::windows::Windows,
    id: uuid::Uuid,
) -> Option<noal_core::window::Entry> {
    match windows {
        noal_view::windows::Windows::Tree(nodes) => nodes
            .iter()
            .flat_map(|node| node.flatten())
            .find(|node| node.entry.id == id)
            .map(|node| node.entry.clone()),
        noal_view::windows::Windows::Unavailable => None,
    }
}

/// Validate the request into a rename, or say why it cannot run.
async fn store(
    state: &AppState,
    viewer: &noal_core::session::SessionClaims,
    segment: &str,
    form: &NameForm,
) -> Result<Current, Failure> {
    // Same 404 as an unknown id: a malformed segment reveals nothing.
    let id = parse_id(segment)?;

    // Refusal over truncation, always: the stored name must be one the
    // viewer chose to the character.
    match normalize_name(form.name.as_deref().unwrap_or_default()) {
        Ok(noal_core::window::Name::Set(name)) => {
            let client = state.database().await?;
            crate::window::set_name(&client, id, &viewer.user_id, Some(&name))
                .await
                .map_err(rename_failure)?;
        }
        Ok(noal_core::window::Name::Clear) => {
            let client = state.database().await?;
            crate::window::set_name(&client, id, &viewer.user_id, None)
                .await
                .map_err(rename_failure)?;
        }
        Err(too_long) => return Err(Failure::from(too_long)),
    }

    // The marker field decides whether the fresh tree marks this row — the
    // same fact the row's hidden input carried in, and whether the response
    // carries that row's heading out of band.
    Ok(match form.current_window {
        Some(_) => Current::Window(id),
        None => Current::Home,
    })
}

/// What a failed rename means to the viewer.
///
/// A window that does not exist or belongs to someone else is the same
/// not-found the read route answers; a refused write is transport.
fn rename_failure(error: crate::window::SetNameError) -> Failure {
    match error {
        crate::window::SetNameError::NoSuch => Failure::NoSuchWindow,
        crate::window::SetNameError::Driver(detail) => Failure::Database(detail),
    }
}

/// Parse a path segment that must be a uuid, or own nothing.
fn parse_id(segment: &str) -> Result<uuid::Uuid, Failure> {
    // Parse here rather than in the route pattern: a segment that is not a
    // uuid must get the same 404 as an id nothing owns, not a router error.
    uuid::Uuid::parse_str(segment).map_err(|_| Failure::NoSuchWindow)
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
