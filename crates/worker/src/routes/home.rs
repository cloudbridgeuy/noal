//! The home page, and the catch-all for unknown paths.

use axum::http::StatusCode;
use axum::response::Response;
use noal_view::layout::Chrome;
use noal_view::windows::{Current, Windows};

use crate::extract::Visitor;
use crate::respond;

/// Render the home page for whoever is asking.
///
/// No window is stored yet, so every page's tree carries Home alone.
pub async fn show(visitor: Visitor) -> Response {
    let chrome = Chrome {
        viewer: visitor.viewer(),
        windows: Windows::Tree(Vec::new()),
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
