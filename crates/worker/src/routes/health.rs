//! Health checks.
//!
//! `/health` answers from the isolate alone and proves only that the Worker is
//! running. `/health/db` opens a real connection through Hyperdrive, so it also
//! proves the binding, the socket, and the Postgres handshake. `/health/llm`
//! calls the model, so it also proves the Wasm build of rig, the fetch
//! transport, and the path to Anthropic. Keep them apart: a load balancer
//! wants the cheap one.

use axum::extract::State;

use crate::extract::SignedIn;
use crate::failure::Failure;
use crate::state::AppState;

/// Answer without touching anything downstream.
pub async fn alive() -> &'static str {
    "ok"
}

/// Answer only after Postgres has answered.
///
/// Uses the simple query protocol on purpose. Hyperdrive with query caching
/// disabled cannot serve a prepared statement, and this route must work in
/// every Hyperdrive configuration.
///
/// # Errors
///
/// Returns [`Failure::Database`] when the connection or the query fails.
pub async fn database(State(state): State<AppState>) -> Result<&'static str, Failure> {
    let client = state.database().await?;
    client
        .simple_query("SELECT 1")
        .await
        .map_err(Failure::database)?;
    Ok("ok")
}

/// Answer only after the model has answered.
///
/// This costs a model call, so it requires a session: an anonymous visitor
/// cannot spend it.
///
/// # Errors
///
/// Returns [`Failure::Model`] when the call fails or the answer is unusable.
pub async fn model(State(state): State<AppState>, _signed_in: SignedIn) -> Result<String, Failure> {
    crate::llm::text(
        state.config(),
        "You answer in one word.",
        "Reply with exactly: ok".to_owned(),
    )
    .await
}
