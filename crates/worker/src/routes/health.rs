//! Health checks.
//!
//! `/health` answers from the isolate alone and proves only that the Worker is
//! running. `/health/db` opens a real connection through Hyperdrive, so it also
//! proves the binding, the socket, and the Postgres handshake. Keep them apart:
//! a load balancer wants the cheap one.

use axum::extract::State;

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
