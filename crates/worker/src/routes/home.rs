//! The home page, and the catch-all for unknown paths.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use noal_view::layout::Chrome;
use noal_view::windows::{Current, Windows};

use crate::extract::Visitor;
use crate::respond;
use crate::state::AppState;

/// Render the home page for whoever is asking.
///
/// A signed-in viewer's tree is read for real; an anonymous viewer renders no
/// palette, so their chrome carries no tree worth reading.
pub async fn show(State(state): State<AppState>, visitor: Visitor) -> Response {
    let windows = match &visitor.0 {
        Some(claims) => crate::chrome::build(&state, &claims.user_id).await,
        None => Windows::Tree(Vec::new()),
    };

    let chrome = Chrome {
        viewer: visitor.viewer(),
        windows,
        current: Current::Home,
    };
    respond::html(StatusCode::OK, noal_view::pages::home(&chrome))
}

/// Render a `404` inside the ordinary layout.
///
/// This does not extract a [`Visitor`]: an axum fallback runs for paths with no
/// route, and rendering the signed-in chrome there would be an invitation to
/// probe for pages that do not exist.
pub async fn not_found() -> Response {
    let markup = noal_view::pages::failure(&Chrome::anonymous(), 404, "There is no page here.");
    respond::html(StatusCode::NOT_FOUND, markup)
}
