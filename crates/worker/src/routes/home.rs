//! The home page, and the catch-all for unknown paths.

use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use noal_view::layout::Viewer;

use crate::extract::Visitor;

/// Render the home page for whoever is asking.
pub async fn show(visitor: Visitor) -> Html<String> {
    Html(noal_view::pages::home(&visitor.viewer()).into_string())
}

/// Render a `404` inside the ordinary layout.
///
/// This does not extract a [`Visitor`]: an axum fallback runs for paths with no
/// route, and rendering the signed-in chrome there would be an invitation to
/// probe for pages that do not exist.
pub async fn not_found() -> Response {
    let markup = noal_view::pages::failure(&Viewer::Anonymous, 404, "There is no page here.");
    (StatusCode::NOT_FOUND, Html(markup.into_string())).into_response()
}
