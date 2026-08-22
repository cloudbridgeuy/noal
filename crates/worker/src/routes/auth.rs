//! Sign in, sign out, and the WorkOS round trip.
//!
//! Three routes and one rule: the browser never holds a WorkOS token. WorkOS
//! hands noal an access token and a refresh token; noal seals both into its own
//! cookie with its own key. Every later request authenticates by unsealing that
//! cookie, so the hot path makes no network call and does no signature check.
//!
//! # Why `SendWrapper` appears here and nowhere else
//!
//! axum requires handler futures to be `Send`. The Postgres path already is.
//! The `worker::Fetch` path is not: it resolves a `JsFuture`, which holds an
//! `Rc`. `SendWrapper` asserts the value never crosses a thread — true on
//! `wasm32-unknown-unknown`, which has exactly one — and it is confined to the
//! two functions below that actually talk to WorkOS.

use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, Uri};
use axum::response::{IntoResponse, Redirect, Response};
use noal_core::auth::{
    authorize_url, claims_from_tokens, AuthError, Callback, TokenRequest, TokenResponse,
    REVOKE_ENDPOINT, STATE_COOKIE_NAME, TOKEN_ENDPOINT,
};
use noal_core::cookie;
use noal_core::session::{seal, COOKIE_NAME};
use send_wrapper::SendWrapper;
use serde::Serialize;
use wasm_bindgen::JsValue;
use worker::{Fetch, Headers, Method, Request, RequestInit};

use crate::entropy;
use crate::extract::Visitor;
use crate::failure::Failure;
use crate::state::AppState;

/// Start a sign-in.
///
/// Draws a random `state`, keeps a copy in a short-lived cookie, and sends the
/// browser to WorkOS with the other copy. The callback is only accepted when
/// the two match, which is what stops a third party from replaying a callback
/// into someone else's browser.
///
/// # Errors
///
/// Returns [`Failure::Upstream`] when the host refuses entropy.
pub async fn login(State(state): State<AppState>) -> Result<Response, Failure> {
    let token = entropy::state_token()?;

    let destination = authorize_url(
        &state.config().workos_client_id,
        &state.config().redirect_uri,
        &token,
    );

    let set_cookie = cookie::write_for(STATE_COOKIE_NAME, &token, cookie::BRIEF_MAX_AGE_SECONDS);

    Ok((cookie_header(&set_cookie)?, Redirect::to(&destination)).into_response())
}

/// Finish a sign-in.
///
/// Checks the echoed `state`, exchanges the code for tokens, seals them, sets
/// the session cookie, and clears the state cookie.
///
/// # Errors
///
/// Returns [`Failure::Auth`] when the callback is not genuine,
/// [`Failure::Upstream`] when WorkOS fails, and [`Failure::Session`] when the
/// claims cannot be sealed.
pub async fn callback(
    State(state): State<AppState>,
    uri: Uri,
    headers: HeaderMap,
) -> Result<Response, Failure> {
    let expected = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|header| cookie::read(header, STATE_COOKIE_NAME))
        .ok_or(Failure::Auth(AuthError::StateMismatch))?;

    let callback = Callback::parse(uri.query().unwrap_or_default());
    let code = callback.code(expected)?;

    let request = TokenRequest::for_code(
        &state.config().workos_client_id,
        &state.config().workos_api_key,
        code,
    );

    let body = serde_json::to_string(&request).map_err(Failure::upstream)?;
    let answer = post_json(TOKEN_ENDPOINT, &body, None).await?;

    let tokens: TokenResponse = serde_json::from_str(&answer).map_err(|error| {
        // The body may hold a WorkOS error object. Do not put it in the
        // response: it can quote the request, and the request held the code.
        Failure::Upstream(format!("token response was not usable: {error}"))
    })?;

    let claims = claims_from_tokens(&tokens)?;
    let sealed = seal(&state.config().session_key, entropy::nonce()?, &claims)?;

    let mut response_headers = cookie_header(&cookie::write(COOKIE_NAME, &sealed))?;
    append_cookie(&mut response_headers, &cookie::clear(STATE_COOKIE_NAME))?;

    Ok((response_headers, Redirect::to("/")).into_response())
}

/// Sign out here, and everywhere.
///
/// Clearing the cookie ends the session in this browser. Revoking through
/// WorkOS ends it in every other one, which is the point of holding the
/// session identifier in the seal.
///
/// A revocation that fails is logged and not surfaced: the local cookie is
/// gone either way, and telling the browser that sign-out failed would invite
/// a retry that cannot help.
pub async fn logout(State(state): State<AppState>, Visitor(session): Visitor) -> Response {
    if let Some(claims) = session {
        let body = serde_json::to_string(&RevokeRequest {
            session_id: &claims.session_id,
        });

        match body {
            Ok(body) => {
                if let Err(error) =
                    post_json(REVOKE_ENDPOINT, &body, Some(&state.config().workos_api_key)).await
                {
                    worker::console_error!("could not revoke the WorkOS session: {error}");
                }
            }
            Err(error) => worker::console_error!("could not build the revoke body: {error}"),
        }
    }

    match cookie_header(&cookie::clear(COOKIE_NAME)) {
        Ok(headers) => (headers, Redirect::to("/")).into_response(),
        Err(failure) => failure.into_response(),
    }
}

/// The body noal posts to end a session everywhere.
#[derive(Serialize)]
struct RevokeRequest<'a> {
    /// The WorkOS session identifier, taken from the sealed claims.
    session_id: &'a str,
}

// ---------------------------------------------------------------------------
// Talking to WorkOS
// ---------------------------------------------------------------------------

/// POST a JSON body and return the response body as text.
///
/// This is the only outbound HTTP call noal makes. It is `!Send` inside and
/// `Send` outside; see the module note.
///
/// # Errors
///
/// Returns [`Failure::Upstream`] when the request cannot be built, the call
/// fails, or WorkOS answers with a non-success status.
async fn post_json(url: &str, body: &str, bearer: Option<&str>) -> Result<String, Failure> {
    let url = url.to_owned();
    let body = body.to_owned();
    let bearer = bearer.map(str::to_owned);

    SendWrapper::new(async move {
        let headers = Headers::new();
        headers
            .set("Content-Type", "application/json")
            .map_err(Failure::upstream)?;
        headers
            .set("Accept", "application/json")
            .map_err(Failure::upstream)?;
        if let Some(bearer) = &bearer {
            headers
                .set("Authorization", &format!("Bearer {bearer}"))
                .map_err(Failure::upstream)?;
        }

        let mut init = RequestInit::new();
        init.with_method(Method::Post)
            .with_headers(headers)
            .with_body(Some(JsValue::from_str(&body)));

        let request = Request::new_with_init(&url, &init).map_err(Failure::upstream)?;
        let mut response = Fetch::Request(request)
            .send()
            .await
            .map_err(Failure::upstream)?;

        let status = response.status_code();
        let text = response.text().await.map_err(Failure::upstream)?;

        if (200..300).contains(&status) {
            Ok(text)
        } else {
            // The status is safe to report; the body is not, so it is logged.
            worker::console_error!("WorkOS answered {status}: {text}");
            Err(Failure::Upstream(format!("WorkOS answered {status}")))
        }
    })
    .await
}

// ---------------------------------------------------------------------------
// Header plumbing
// ---------------------------------------------------------------------------

/// Put one `Set-Cookie` value into a fresh header map.
fn cookie_header(value: &str) -> Result<HeaderMap, Failure> {
    let mut headers = HeaderMap::new();
    append_cookie(&mut headers, value)?;
    Ok(headers)
}

/// Add another `Set-Cookie` value, keeping the ones already there.
fn append_cookie(headers: &mut HeaderMap, value: &str) -> Result<(), Failure> {
    let value = HeaderValue::from_str(value).map_err(Failure::upstream)?;
    headers.append(header::SET_COOKIE, value);
    Ok(())
}
