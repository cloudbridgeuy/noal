//! Turning a request into a viewer.
//!
//! Authentication happens once, here, as an axum extractor. A handler that
//! names [`SignedIn`] in its arguments cannot run without a valid session, and
//! a handler that names [`Visitor`] gets whatever there was. There is no third
//! way to read the session cookie, so no handler can forget to check it.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use noal_core::cookie;
use noal_core::session::{unseal, SessionClaims, SessionError, COOKIE_NAME};
use noal_view::layout::Viewer;

use crate::failure::Failure;
use crate::state::{now, AppState};

/// A request that carries a valid session.
///
/// Extraction fails with `401` when the cookie is absent, forged, or expired.
#[derive(Debug, Clone)]
pub struct SignedIn(pub SessionClaims);

/// A request that may or may not carry a valid session.
///
/// Extraction never fails. Use this for pages that render for anyone.
#[derive(Debug, Clone)]
pub struct Visitor(pub Option<SessionClaims>);

impl Visitor {
    /// How the layout should describe this request.
    #[must_use]
    pub fn viewer(&self) -> Viewer {
        match &self.0 {
            Some(claims) => Viewer::SignedIn {
                email: claims.email.clone(),
            },
            None => Viewer::Anonymous,
        }
    }
}

/// Read and unseal the session cookie, if there is one.
///
/// A cookie that fails to unseal is reported rather than swallowed, so the
/// caller can choose: [`SignedIn`] refuses the request, [`Visitor`] logs the
/// reason and continues as anonymous.
fn read_session(parts: &Parts, state: &AppState) -> Result<SessionClaims, SessionError> {
    let header = parts
        .headers
        .get(axum::http::header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .ok_or(SessionError::MalformedCookie)?;

    let sealed = cookie::read(header, COOKIE_NAME).ok_or(SessionError::MalformedCookie)?;

    unseal(&state.config().session_key, sealed, now())
}

impl FromRequestParts<AppState> for SignedIn {
    type Rejection = Failure;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Failure> {
        read_session(parts, state)
            .map(Self)
            .map_err(Failure::Session)
    }
}

impl FromRequestParts<AppState> for Visitor {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, std::convert::Infallible> {
        match read_session(parts, state) {
            Ok(claims) => Ok(Self(Some(claims))),
            // A missing cookie is the ordinary case for a signed-out visitor
            // and is not worth a log line. Anything else means a cookie noal
            // wrote has gone bad, which is.
            Err(SessionError::MalformedCookie) => Ok(Self(None)),
            Err(error) => {
                worker::console_warn!("discarding session cookie: {error}");
                Ok(Self(None))
            }
        }
    }
}
