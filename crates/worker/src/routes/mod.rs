//! The route table.
//!
//! One place lists every URL noal answers. Handlers live in the sibling
//! modules; this module only says which path reaches which one.

mod auth;
mod health;
mod home;

use axum::routing::{get, post};
use axum::Router;

use crate::state::AppState;

/// Build the whole application.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(home::show))
        .route("/auth/login", get(auth::login))
        .route("/auth/callback", get(auth::callback))
        .route("/auth/logout", post(auth::logout))
        .route("/health", get(health::alive))
        .route("/health/db", get(health::database))
        .route("/health/llm", get(health::model))
        .fallback(home::not_found)
        .with_state(state)
}
