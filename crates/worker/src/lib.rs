//! The imperative shell for noal.
//!
//! This crate owns every effect: the Workers runtime, the Postgres socket, the
//! WorkOS calls, the clock, and the random number generator. It compiles only
//! for `wasm32-unknown-unknown`, so nothing here can be reached from the pure
//! crates by accident.
//!
//! The shape of a request is always the same:
//!
//! 1. [`config`] parses the environment into types that cannot be wrong.
//! 2. [`extract`] turns the session cookie into a viewer, or refuses.
//! 3. A handler in [`routes`] gathers data through [`state::AppState`].
//! 4. `noal_view` renders it, and `noal_core` decides everything else.
//!
//! Handlers hold no logic worth testing. Every rule they apply lives in
//! `noal_core`, where it is tested natively; see
//! `~/.claude/patterns/functional-core-imperative-shell.md`.
#![deny(clippy::unwrap_used, clippy::expect_used)]
#![deny(missing_docs)]

pub mod config;
pub mod entropy;
pub mod extract;
pub mod failure;
pub mod llm;
pub mod respond;
pub mod routes;
pub mod state;

use axum::response::IntoResponse;
use tower_service::Service;
use worker::{event, Context, Env, HttpRequest};

/// The Worker entry point.
///
/// Cloudflare calls this once per request. Configuration is parsed here rather
/// than inside a handler, so a missing binding produces one clear `500` with a
/// log line naming the binding, instead of a different failure on every route.
///
/// # Errors
///
/// Returns a `worker::Result` because the runtime requires it. Every failure
/// noal can describe becomes a rendered response instead of an error, so this
/// only returns `Err` if axum itself cannot answer.
#[event(fetch)]
pub async fn fetch(
    request: HttpRequest,
    env: Env,
    _context: Context,
) -> worker::Result<axum::http::Response<axum::body::Body>> {
    console_error_panic_hook::set_once();

    let state = match state::AppState::new(env) {
        Ok(state) => state,
        Err(failure) => return Ok(failure.into_response()),
    };

    Ok(routes::router(state).call(request).await?)
}
