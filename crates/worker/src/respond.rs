//! How rendered markup leaves the Worker.
//!
//! Every HTML answer noal sends is built here, so they all carry the same
//! freshness rule: a page embeds per-viewer state — the signed-in address, the
//! saved-window tree — and must always re-run on the server rather than come
//! back from a cache.

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use maud::Markup;

/// Wrap markup as an HTML response browsers must never reuse.
///
/// The `no-store` header is the HTTP half of freshness; `hx-history="false"`
/// in the markup is the htmx half.
#[must_use]
pub fn html(status: StatusCode, markup: Markup) -> Response {
    let mut response = (status, Html(markup.into_string())).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
