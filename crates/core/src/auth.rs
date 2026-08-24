//! The pure half of the WorkOS AuthKit exchange.
//!
//! Signing in is three network steps and a lot of string handling. The network
//! steps belong to the shell. Everything else — building the authorize URL,
//! reading the callback query, turning a token response into session claims —
//! is here, where it can be tested by passing strings in and comparing strings
//! out.
//!
//! # On not verifying the access token
//!
//! [`claims_from_tokens`] reads the access token's payload without checking its
//! signature. That is deliberate and it is safe *only* in this one place: the
//! token arrives in the body of a TLS response to a request noal made to
//! `api.workos.com` with its own API key. TLS already proves who sent it, so a
//! signature check would re-prove the same fact.
//!
//! Once read, the claims are sealed into the session cookie with noal's own
//! key. Every later request authenticates by unsealing that cookie, never by
//! re-reading the WorkOS token. So no untrusted input ever reaches this parser.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;
use crate::session::SessionClaims;

/// The cookie that carries the OAuth `state` across the sign-in round trip.
pub const STATE_COOKIE_NAME: &str = "__Host-noal_oauth_state";

/// The cookie that carries the post-sign-in destination across the round trip.
///
/// Short-lived like [`STATE_COOKIE_NAME`], and for the same reason: it only
/// has to survive from the login redirect to the callback that follows it.
pub const RETURN_COOKIE_NAME: &str = "__Host-noal_return";

/// The WorkOS hosted authorization endpoint.
pub const AUTHORIZE_ENDPOINT: &str = "https://api.workos.com/user_management/authorize";

/// The WorkOS code-for-token endpoint.
pub const TOKEN_ENDPOINT: &str = "https://api.workos.com/user_management/authenticate";

/// The WorkOS session revocation endpoint.
pub const REVOKE_ENDPOINT: &str = "https://api.workos.com/user_management/sessions/revoke";

/// Why an authorization exchange could not produce a session.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthError {
    /// The callback carried neither a code nor an error. Not a WorkOS redirect.
    #[error("callback query held no code and no error")]
    EmptyCallback,
    /// The callback's `state` did not match the one noal sent.
    #[error("callback state did not match")]
    StateMismatch,
    /// WorkOS reported a failure instead of issuing a code.
    #[error("WorkOS refused the sign-in: {0}")]
    Refused(String),
    /// The access token was not three dot-separated segments.
    #[error("access token is not a JWT")]
    MalformedToken,
    /// The access token's payload was not base64url of valid JSON.
    #[error("access token payload could not be read")]
    UnreadablePayload,
}

/// Build the URL that starts a sign-in.
///
/// `state` travels to WorkOS and comes back on the callback unchanged. Pass a
/// value only noal can produce, and check it with [`Callback::code`]; that is
/// what stops a third party from replaying someone else's callback.
#[must_use]
pub fn authorize_url(client_id: &str, redirect_uri: &str, state: &str) -> String {
    let client_id = percent_encode(client_id);
    let redirect_uri = percent_encode(redirect_uri);
    let state = percent_encode(state);

    format!(
        "{AUTHORIZE_ENDPOINT}\
         ?client_id={client_id}\
         &redirect_uri={redirect_uri}\
         &response_type=code\
         &provider=authkit\
         &state={state}"
    )
}

/// What came back on the `/auth/callback` query string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Callback {
    /// The authorization code, when WorkOS issued one.
    code: Option<String>,
    /// The state noal sent on the way out, echoed back.
    state: Option<String>,
    /// The failure WorkOS reported, when it issued no code.
    error: Option<String>,
}

impl Callback {
    /// Read a raw query string into a callback.
    ///
    /// Never fails. A query string that means nothing becomes a callback whose
    /// [`Callback::code`] returns [`AuthError::EmptyCallback`].
    #[must_use]
    pub fn parse(query: &str) -> Self {
        let mut callback = Self {
            code: None,
            state: None,
            error: None,
        };

        for pair in query.trim_start_matches('?').split('&') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            let value = percent_decode(value);
            match key {
                "code" => callback.code = Some(value),
                "state" => callback.state = Some(value),
                // WorkOS may send both. The description is the readable one,
                // so it wins whichever order they arrive in.
                "error" if callback.error.is_none() => callback.error = Some(value),
                "error_description" => callback.error = Some(value),
                _ => {}
            }
        }

        callback
    }

    /// The authorization code, once the state has been checked.
    ///
    /// There is no way to read the code without passing the expected state, so
    /// a handler cannot skip the check by accident.
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::Refused`] when WorkOS reported a failure,
    /// [`AuthError::StateMismatch`] when the echoed state is wrong, and
    /// [`AuthError::EmptyCallback`] when the query held nothing usable.
    pub fn code(&self, expected_state: &str) -> Result<&str, AuthError> {
        if let Some(error) = &self.error {
            return Err(AuthError::Refused(error.clone()));
        }

        let code = self.code.as_deref().ok_or(AuthError::EmptyCallback)?;

        if self.state.as_deref() != Some(expected_state) {
            return Err(AuthError::StateMismatch);
        }

        Ok(code)
    }
}

/// Read the `next` query parameter off a raw `/auth/login` query string.
///
/// Reuses the same percent-decoding [`Callback::parse`] applies to `code` and
/// `state`, so the shell never has to decode a query string itself. Returns
/// `None` when there is no `next` parameter; the value is not validated here,
/// only decoded — pass it through [`return_path`] before trusting it.
#[must_use]
pub fn next_param(query: &str) -> Option<String> {
    for pair in query.trim_start_matches('?').split('&') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        if key == "next" {
            return Some(percent_decode(value));
        }
    }
    None
}

/// Validate a post-sign-in redirect target.
///
/// Accepts a value only when it is rooted at the origin: it must start with
/// exactly one `/`, must not be followed by a second `/` or a `\` (browsers
/// treat `/\` the same as `//` when resolving a URL, so both make the value
/// protocol-relative and both are refused), and must contain no `:` anywhere
/// (which would turn it into an absolute URL with its own scheme, such as
/// `https:x`). A `;` or any control character is refused too, because the
/// accepted value is later written verbatim into a cookie value and a
/// redirect header, where either could break out of the field it is placed
/// in.
///
/// This is the whole defence against an open redirect through `/auth/login`:
/// nothing else on the sign-in path checks where `next` points.
#[must_use]
pub fn return_path(raw: &str) -> Option<&str> {
    let mut chars = raw.chars();
    if chars.next() != Some('/') {
        return None;
    }
    if matches!(chars.next(), Some('/') | Some('\\')) {
        return None;
    }
    if raw.contains([':', ';']) || raw.chars().any(char::is_control) {
        return None;
    }
    Some(raw)
}

/// The body noal posts to exchange a code for tokens.
#[derive(Debug, Clone, Serialize)]
pub struct TokenRequest<'a> {
    /// The WorkOS client identifier.
    pub client_id: &'a str,
    /// The WorkOS API key. Secret; never rendered and never logged.
    pub client_secret: &'a str,
    /// Always `authorization_code` for this exchange.
    pub grant_type: &'a str,
    /// The code from the callback.
    pub code: &'a str,
}

impl<'a> TokenRequest<'a> {
    /// Build the exchange body for an authorization code.
    #[must_use]
    pub const fn for_code(client_id: &'a str, client_secret: &'a str, code: &'a str) -> Self {
        Self {
            client_id,
            client_secret,
            grant_type: "authorization_code",
            code,
        }
    }
}

/// The user WorkOS reports alongside the tokens.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct WorkOsUser {
    /// The WorkOS user identifier.
    pub id: String,
    /// The user's email address.
    pub email: String,
}

/// What WorkOS returns from the token endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TokenResponse {
    /// The signed access token. A JWT holding `sid` and `exp`.
    pub access_token: String,
    /// The refresh token, which buys a new access token later.
    pub refresh_token: String,
    /// The authenticated user.
    pub user: WorkOsUser,
    /// The organization the user signed in through, when there was one.
    #[serde(default)]
    pub organization_id: Option<String>,
}

/// The access token payload claims noal reads.
#[derive(Debug, Clone, Deserialize)]
struct AccessTokenPayload {
    /// The WorkOS session identifier. Revocation is keyed on this.
    sid: String,
    /// Expiry, in seconds since the Unix epoch.
    exp: i64,
}

/// Turn a token response into the claims that go inside the session cookie.
///
/// # Errors
///
/// Returns [`AuthError::MalformedToken`] or [`AuthError::UnreadablePayload`]
/// when the access token is not a JWT carrying `sid` and `exp`.
pub fn claims_from_tokens(response: &TokenResponse) -> Result<SessionClaims, AuthError> {
    let payload = read_token_payload(&response.access_token)?;

    Ok(SessionClaims {
        user_id: response.user.id.clone(),
        session_id: payload.sid,
        email: response.user.email.clone(),
        organization_id: response.organization_id.clone(),
        access_token: response.access_token.clone(),
        refresh_token: response.refresh_token.clone(),
        expires_at: Timestamp::from_unix_seconds(payload.exp),
    })
}

/// Decode a JWT's middle segment. See the module note on why this does not
/// verify the signature.
fn read_token_payload(token: &str) -> Result<AccessTokenPayload, AuthError> {
    let mut segments = token.split('.');
    let (Some(_header), Some(payload), Some(_signature)) =
        (segments.next(), segments.next(), segments.next())
    else {
        return Err(AuthError::MalformedToken);
    };

    if segments.next().is_some() {
        return Err(AuthError::MalformedToken);
    }

    let decoded = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| AuthError::UnreadablePayload)?;

    serde_json::from_slice(&decoded).map_err(|_| AuthError::UnreadablePayload)
}

/// Percent-encode a query parameter value.
///
/// Leaves the unreserved set of RFC 3986 alone and escapes everything else, so
/// the result is safe in a query string whatever the input held.
fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(char::from(byte));
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// Undo percent-encoding, treating `+` as a space.
///
/// A malformed escape is kept as written rather than rejected; this parses
/// redirects from a service noal trusts, and a mangled value fails later at the
/// state check or the token exchange.
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = &value[index + 1..index + 3];
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        decoded.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        decoded.push(b'%');
                        index += 1;
                    }
                }
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }

    String::from_utf8_lossy(&decoded).into_owned()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{
        authorize_url, claims_from_tokens, next_param, percent_decode, percent_encode, return_path,
        AuthError, Callback, TokenRequest, TokenResponse, WorkOsUser,
    };
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;

    fn token_with(sid: &str, exp: i64) -> String {
        let payload = format!(r#"{{"sid":"{sid}","exp":{exp},"sub":"user_1"}}"#);
        format!("header.{}.signature", URL_SAFE_NO_PAD.encode(payload))
    }

    fn response(sid: &str, exp: i64) -> TokenResponse {
        TokenResponse {
            access_token: token_with(sid, exp),
            refresh_token: "refresh".to_owned(),
            user: WorkOsUser {
                id: "user_1".to_owned(),
                email: "ada@example.com".to_owned(),
            },
            organization_id: Some("org_1".to_owned()),
        }
    }

    #[test]
    fn authorize_url_carries_every_parameter() {
        let url = authorize_url("client_1", "https://noal.dev/auth/callback", "nonce");
        assert!(url.starts_with(super::AUTHORIZE_ENDPOINT));
        assert!(url.contains("client_id=client_1"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("provider=authkit"));
        assert!(url.contains("state=nonce"));
    }

    #[test]
    fn authorize_url_escapes_the_redirect() {
        let url = authorize_url("client_1", "https://noal.dev/auth/callback", "nonce");
        assert!(url.contains("redirect_uri=https%3A%2F%2Fnoal.dev%2Fauth%2Fcallback"));
    }

    #[test]
    fn callback_yields_the_code_when_the_state_matches() {
        let callback = Callback::parse("?code=abc&state=nonce");
        assert_eq!(callback.code("nonce"), Ok("abc"));
    }

    #[test]
    fn callback_rejects_a_mismatched_state() {
        let callback = Callback::parse("code=abc&state=other");
        assert_eq!(callback.code("nonce"), Err(AuthError::StateMismatch));
    }

    #[test]
    fn callback_rejects_a_missing_state() {
        let callback = Callback::parse("code=abc");
        assert_eq!(callback.code("nonce"), Err(AuthError::StateMismatch));
    }

    #[test]
    fn callback_reports_a_refusal_before_anything_else() {
        let callback = Callback::parse("error=access_denied&state=nonce");
        assert_eq!(
            callback.code("nonce"),
            Err(AuthError::Refused("access_denied".to_owned()))
        );
    }

    #[test]
    fn callback_prefers_the_readable_description() {
        let callback = Callback::parse("error=access_denied&error_description=user+said+no");
        assert_eq!(
            callback.code("nonce"),
            Err(AuthError::Refused("user said no".to_owned()))
        );
    }

    #[test]
    fn callback_reports_an_empty_query() {
        assert_eq!(
            Callback::parse("").code("nonce"),
            Err(AuthError::EmptyCallback)
        );
    }

    #[test]
    fn callback_ignores_parameters_it_does_not_know() {
        let callback = Callback::parse("utm=x&code=abc&state=nonce&extra");
        assert_eq!(callback.code("nonce"), Ok("abc"));
    }

    #[test]
    fn token_request_pins_the_grant_type() {
        let request = TokenRequest::for_code("client_1", "secret", "abc");
        assert_eq!(request.grant_type, "authorization_code");
    }

    #[test]
    fn claims_carry_the_session_id_from_the_token() {
        let claims = claims_from_tokens(&response("session_1", 2_000)).unwrap();
        assert_eq!(claims.session_id, "session_1");
        assert_eq!(claims.user_id, "user_1");
        assert_eq!(claims.email, "ada@example.com");
        assert_eq!(claims.organization_id.as_deref(), Some("org_1"));
        assert_eq!(claims.expires_at.as_unix_seconds(), 2_000);
    }

    #[test]
    fn claims_reject_a_token_that_is_not_a_jwt() {
        let mut broken = response("session_1", 2_000);
        broken.access_token = "not-a-jwt".to_owned();
        assert_eq!(claims_from_tokens(&broken), Err(AuthError::MalformedToken));
    }

    #[test]
    fn claims_reject_a_payload_that_is_not_json() {
        let mut broken = response("session_1", 2_000);
        broken.access_token = format!("header.{}.signature", URL_SAFE_NO_PAD.encode("nonsense"));
        assert_eq!(
            claims_from_tokens(&broken),
            Err(AuthError::UnreadablePayload)
        );
    }

    #[test]
    fn claims_reject_a_payload_missing_the_session_id() {
        let mut broken = response("session_1", 2_000);
        broken.access_token = format!("header.{}.sig", URL_SAFE_NO_PAD.encode(r#"{"exp":1}"#));
        assert_eq!(
            claims_from_tokens(&broken),
            Err(AuthError::UnreadablePayload)
        );
    }

    #[test]
    fn token_response_reads_a_body_without_an_organization() {
        let body = r#"{
            "access_token": "a.b.c",
            "refresh_token": "r",
            "user": { "id": "user_1", "email": "ada@example.com", "first_name": "Ada" }
        }"#;
        let parsed: TokenResponse = serde_json::from_str(body).unwrap();
        assert_eq!(parsed.organization_id, None);
    }

    #[test]
    fn encoding_leaves_the_unreserved_set_alone() {
        assert_eq!(percent_encode("aZ0-._~"), "aZ0-._~");
    }

    #[test]
    fn encoding_escapes_everything_else() {
        assert_eq!(percent_encode("a b/c?"), "a%20b%2Fc%3F");
    }

    #[test]
    fn decoding_undoes_encoding() {
        let original = "https://noal.dev/auth/callback?x=1";
        assert_eq!(percent_decode(&percent_encode(original)), original);
    }

    #[test]
    fn decoding_keeps_a_truncated_escape_as_written() {
        assert_eq!(percent_decode("abc%2"), "abc%2");
    }

    #[test]
    fn next_param_reads_a_plain_value() {
        assert_eq!(next_param("next=/health"), Some("/health".to_owned()));
    }

    #[test]
    fn next_param_decodes_percent_escapes() {
        assert_eq!(next_param("next=%2Fhealth"), Some("/health".to_owned()));
    }

    #[test]
    fn next_param_ignores_other_parameters() {
        assert_eq!(
            next_param("utm=x&next=/health&extra"),
            Some("/health".to_owned())
        );
    }

    #[test]
    fn next_param_is_absent_when_not_given() {
        assert_eq!(next_param("code=abc&state=nonce"), None);
    }

    #[test]
    fn next_param_is_absent_from_an_empty_query() {
        assert_eq!(next_param(""), None);
    }

    /// The real `next` value the browser sends is percent-encoded and carries
    /// a query string: `location.pathname + location.search` run through
    /// `encodeURIComponent`. Pins that the decoded value keeps the `?` and
    /// `=` characters rather than stopping at the first escape.
    #[test]
    fn next_param_decodes_a_percent_encoded_query_string() {
        assert_eq!(
            next_param("next=%2Fhealth%3Fx%3D1"),
            Some("/health?x=1".to_owned())
        );
    }

    /// The browser-side caller always percent-encodes `next`, so a raw `&`
    /// inside the value never reaches this function in practice. Pinned
    /// anyway: `next_param` splits the whole query on `&` before it looks at
    /// `=`, so an unescaped `&` inside the value is read as the start of the
    /// *next* parameter, not as part of this one. The tail after it (`y=2`)
    /// is discarded, not appended.
    #[test]
    fn next_param_truncates_an_unencoded_ampersand() {
        assert_eq!(
            next_param("next=/health?x=1&y=2"),
            Some("/health?x=1".to_owned())
        );
    }

    /// `return_path` is the whole defence against an open redirect, so every
    /// case in the security rule gets a row here rather than a handful of
    /// spot checks.
    #[test]
    fn return_path_table() {
        let cases: &[(&str, Option<&str>)] = &[
            ("/health", Some("/health")),
            ("/", Some("/")),
            ("", None),
            ("health", None),
            ("//evil.com", None),
            (r"/\evil.com", None),
            ("https:x", None),
            ("http://evil.com", None),
            ("/evil.com:8080/path", None),
            ("/a;Domain=evil.com", None),
            ("/a\r\nSet-Cookie:x=y", None),
            ("/a\nb", None),
            ("/a\tb", None),
            // The everyday shape: the browser builds `next` from
            // `location.pathname + location.search`, so a query string is
            // the common case, not an edge case.
            ("/health?x=1", Some("/health?x=1")),
            ("/w/7?tab=a&sort=b", Some("/w/7?tab=a&sort=b")),
            // A fragment never leaves the browser as part of the request
            // that follows the redirect, so accepting it here is harmless:
            // it is still rooted at the origin, carries no scheme, and
            // contains no character the cookie or `Location` header would
            // choke on.
            ("/health#top", Some("/health#top")),
        ];

        for (raw, expected) in cases {
            assert_eq!(return_path(raw), *expected, "case: {raw:?}");
        }
    }
}
