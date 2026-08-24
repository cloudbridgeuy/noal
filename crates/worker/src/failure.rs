//! One failure type for the whole shell, and one way to render it.
//!
//! Handlers return `Result<T, Failure>`. Every error a handler can produce has
//! a variant here, and every variant knows its own status code and its own
//! public wording. That separation matters: [`Failure::detail`] is for the
//! Worker log and may name a binding or quote a driver, while
//! [`Failure::message`] is what the browser sees and never quotes anything the
//! request supplied.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use noal_core::auth::AuthError;
use noal_core::session::SessionError;

use crate::config::ConfigError;
use crate::respond;

/// Everything that can stop a request.
#[derive(Debug, Clone, thiserror::Error)]
pub enum Failure {
    /// The environment is not configured. The operator must fix the deploy.
    #[error("configuration: {0}")]
    Config(#[from] ConfigError),

    /// The database refused a connection or a query.
    #[error("database: {0}")]
    Database(String),

    /// WorkOS was unreachable, or answered with something unusable.
    #[error("upstream: {0}")]
    Upstream(String),

    /// The model did not answer, or answered with something unusable.
    #[error("model: {0}")]
    Model(String),

    /// The sign-in exchange failed.
    #[error("sign-in: {0}")]
    Auth(#[from] AuthError),

    /// The session cookie was absent, forged, or expired.
    #[error("session: {0}")]
    Session(#[from] SessionError),

    /// The route asked for a signed-in user and there was none.
    #[error("not signed in")]
    NotSignedIn,

    /// No saved window answers this address. Unknown id, another user's id,
    /// and a malformed segment all land here on purpose: from outside, the
    /// three are indistinguishable, so an address reveals nothing about
    /// which windows exist or whose they are.
    #[error("no such window")]
    NoSuchWindow,
}

impl Failure {
    /// Wrap any error from the Postgres driver.
    pub fn database(error: impl std::fmt::Display) -> Self {
        Self::Database(error.to_string())
    }

    /// Wrap any error from an outbound request.
    pub fn upstream(error: impl std::fmt::Display) -> Self {
        Self::Upstream(error.to_string())
    }

    /// Wrap any error from the model client.
    pub fn model(error: impl std::fmt::Display) -> Self {
        Self::Model(error.to_string())
    }

    /// The status code this failure deserves.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::Config(_) => StatusCode::INTERNAL_SERVER_ERROR,
            Self::Database(_) | Self::Upstream(_) | Self::Model(_) => StatusCode::BAD_GATEWAY,
            Self::Auth(_) => StatusCode::BAD_REQUEST,
            Self::NoSuchWindow => StatusCode::NOT_FOUND,
            Self::Session(_) | Self::NotSignedIn => StatusCode::UNAUTHORIZED,
        }
    }

    /// What the browser is told.
    ///
    /// Deliberately vague. The specific cause goes to the log through
    /// [`Failure::detail`], because a caller who can read the cause of a
    /// session failure learns whether a cookie was forged or merely stale.
    #[must_use]
    pub const fn message(&self) -> &'static str {
        match self {
            Self::Config(_) => "This deployment is not configured correctly.",
            Self::Database(_) => "The database did not answer.",
            Self::Upstream(_) => "A service noal depends on did not answer.",
            Self::Model(_) => "The model did not answer.",
            Self::Auth(_) => "That sign-in could not be completed.",
            Self::NoSuchWindow => "There is no window at this address.",
            Self::Session(_) | Self::NotSignedIn => "Please sign in.",
        }
    }

    /// The full cause, for the Worker log only.
    #[must_use]
    pub fn detail(&self) -> String {
        self.to_string()
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        worker::console_error!("{}", self.detail());

        let status = self.status();
        let markup = noal_view::pages::failure(
            &noal_view::layout::Chrome::anonymous(),
            status.as_u16(),
            self.message(),
        );

        respond::html(status, markup)
    }
}
