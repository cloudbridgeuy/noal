//! Sealed session cookies.
//!
//! noal authenticates a request by unsealing its own cookie. The cookie holds
//! the WorkOS tokens and the user identity, encrypted with a key only noal
//! knows. Because the seal is authenticated encryption, a cookie that unseals
//! is a cookie noal wrote. There is no signature to verify against WorkOS on
//! the hot path, no database lookup, and no network call.
//!
//! WorkOS is contacted twice only: once at the login callback to exchange the
//! authorization code, and again when the access token expires and the refresh
//! token buys a new one. Revocation happens through the WorkOS Sessions API and
//! takes effect when the access token expires, so keep that duration short.
//!
//! Everything in this module is pure. The nonce arrives as an argument because
//! drawing randomness is an effect, and so does the current time.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce as CipherNonce};
use serde::{Deserialize, Serialize};

use crate::clock::Timestamp;

/// The name of the cookie noal sets. `__Host-` forbids a subdomain from
/// setting it, and requires Secure and Path=/.
pub const COOKIE_NAME: &str = "__Host-noal_session";

/// Bytes in the sealing key.
pub const KEY_LEN: usize = 32;

/// Bytes in the per-seal nonce.
pub const NONCE_LEN: usize = 12;

/// The secret that seals and unseals session cookies.
///
/// The inner bytes stay private so a key cannot be built from arbitrary input
/// without going through [`SessionKey::from_bytes`].
#[derive(Clone)]
pub struct SessionKey([u8; KEY_LEN]);

impl std::fmt::Debug for SessionKey {
    /// Print a placeholder, never the bytes.
    ///
    /// The key is derivable from nothing else, so anything that logs a struct
    /// holding one — a config dump, an error report — would leak every session
    /// noal has ever sealed. A hand-written `Debug` makes that impossible.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionKey(<redacted>)")
    }
}

impl SessionKey {
    /// Take ownership of raw key bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; KEY_LEN]) -> Self {
        Self(bytes)
    }

    /// Parse a key from base64url text, as it is stored in a Worker secret.
    ///
    /// # Errors
    ///
    /// Returns [`SessionError::MalformedKey`] when the text is not base64url,
    /// or does not decode to exactly [`KEY_LEN`] bytes.
    pub fn from_base64(text: &str) -> Result<Self, SessionError> {
        let raw = URL_SAFE_NO_PAD
            .decode(text.trim())
            .map_err(|_| SessionError::MalformedKey)?;
        let bytes: [u8; KEY_LEN] = raw.try_into().map_err(|_| SessionError::MalformedKey)?;
        Ok(Self(bytes))
    }
}

/// A single-use nonce. The shell draws these from the platform's randomness.
#[derive(Debug, Clone, Copy)]
pub struct Nonce([u8; NONCE_LEN]);

impl Nonce {
    /// Take ownership of raw nonce bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; NONCE_LEN]) -> Self {
        Self(bytes)
    }
}

/// What noal knows about the signed-in user, and the WorkOS tokens it holds on
/// their behalf.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionClaims {
    /// The WorkOS user identifier, from the access token's `sub` claim.
    pub user_id: String,
    /// The WorkOS session identifier, from the `sid` claim. Revoke by this.
    pub session_id: String,
    /// The user's email address.
    pub email: String,
    /// The WorkOS organization, when the user signed in through one.
    pub organization_id: Option<String>,
    /// The WorkOS access token. Present so noal can call WorkOS as the user.
    pub access_token: String,
    /// The WorkOS refresh token. Buys a new access token after expiry.
    pub refresh_token: String,
    /// When the access token stops being valid.
    pub expires_at: Timestamp,
}

impl SessionClaims {
    /// True when the access token is still valid at `now`.
    #[must_use]
    pub fn is_fresh_at(&self, now: Timestamp) -> bool {
        self.expires_at.is_after(now)
    }
}

/// Why a seal or unseal failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    /// The key was not base64url of exactly [`KEY_LEN`] bytes.
    #[error("session key must be {KEY_LEN} base64url-encoded bytes")]
    MalformedKey,
    /// The cookie was not valid base64url.
    #[error("session cookie is not valid base64url")]
    MalformedCookie,
    /// The cookie was too short to hold a nonce and a ciphertext.
    #[error("session cookie is truncated")]
    TruncatedCookie,
    /// The claims could not be serialized.
    #[error("could not serialize session claims")]
    Serialize,
    /// The ciphertext did not authenticate, so noal did not write this cookie.
    #[error("session cookie failed authentication")]
    BadSeal,
    /// The cookie unsealed but held claims noal could not read.
    #[error("session cookie holds unreadable claims")]
    UnreadableClaims,
    /// The cookie was genuine, but the access token has expired.
    #[error("session expired")]
    Expired,
}

/// Seal claims into cookie text.
///
/// The nonce is prepended to the ciphertext, so `unseal` needs only the key.
/// Callers must pass a fresh nonce for every seal; reusing one with the same
/// key destroys the security of the cipher.
///
/// # Errors
///
/// Returns [`SessionError::Serialize`] when the claims cannot be encoded, or
/// [`SessionError::BadSeal`] when the cipher rejects the input.
pub fn seal(
    key: &SessionKey,
    nonce: Nonce,
    claims: &SessionClaims,
) -> Result<String, SessionError> {
    let plaintext = serde_json::to_vec(claims).map_err(|_| SessionError::Serialize)?;

    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key.0));
    let ciphertext = cipher
        .encrypt(CipherNonce::from_slice(&nonce.0), plaintext.as_slice())
        .map_err(|_| SessionError::BadSeal)?;

    let mut sealed = Vec::with_capacity(NONCE_LEN + ciphertext.len());
    sealed.extend_from_slice(&nonce.0);
    sealed.extend_from_slice(&ciphertext);

    Ok(URL_SAFE_NO_PAD.encode(sealed))
}

/// Unseal cookie text back into claims, and reject an expired session.
///
/// A successful return means two things at once: noal wrote this cookie, and
/// the access token inside it is still valid at `now`. No other state can
/// escape this function, so callers cannot forget to check expiry.
///
/// # Errors
///
/// Returns a [`SessionError`] describing which step failed. Treat every
/// variant as "not signed in"; the distinction is for logs, not for control
/// flow that grants access.
pub fn unseal(
    key: &SessionKey,
    cookie: &str,
    now: Timestamp,
) -> Result<SessionClaims, SessionError> {
    let sealed = URL_SAFE_NO_PAD
        .decode(cookie.trim())
        .map_err(|_| SessionError::MalformedCookie)?;

    if sealed.len() <= NONCE_LEN {
        return Err(SessionError::TruncatedCookie);
    }
    let (nonce, ciphertext) = sealed.split_at(NONCE_LEN);

    let cipher = ChaCha20Poly1305::new(Key::from_slice(&key.0));
    let plaintext = cipher
        .decrypt(CipherNonce::from_slice(nonce), ciphertext)
        .map_err(|_| SessionError::BadSeal)?;

    let claims: SessionClaims =
        serde_json::from_slice(&plaintext).map_err(|_| SessionError::UnreadableClaims)?;

    if claims.is_fresh_at(now) {
        Ok(claims)
    } else {
        Err(SessionError::Expired)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{seal, unseal, Nonce, SessionClaims, SessionError, SessionKey, KEY_LEN, NONCE_LEN};
    use crate::clock::Timestamp;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;

    fn key() -> SessionKey {
        SessionKey::from_bytes([7u8; KEY_LEN])
    }

    fn nonce() -> Nonce {
        Nonce::from_bytes([3u8; NONCE_LEN])
    }

    fn claims(expires_at: i64) -> SessionClaims {
        SessionClaims {
            user_id: "user_01".to_owned(),
            session_id: "session_01".to_owned(),
            email: "someone@example.com".to_owned(),
            organization_id: Some("org_01".to_owned()),
            access_token: "access".to_owned(),
            refresh_token: "refresh".to_owned(),
            expires_at: Timestamp::from_unix_seconds(expires_at),
        }
    }

    #[test]
    fn seals_and_unseals_the_same_claims() {
        let original = claims(1_000);
        let cookie = seal(&key(), nonce(), &original).unwrap();
        let recovered = unseal(&key(), &cookie, Timestamp::from_unix_seconds(500)).unwrap();
        assert_eq!(recovered, original);
    }

    #[test]
    fn rejects_a_cookie_sealed_with_another_key() {
        let cookie = seal(&key(), nonce(), &claims(1_000)).unwrap();
        let other = SessionKey::from_bytes([9u8; KEY_LEN]);
        let error = unseal(&other, &cookie, Timestamp::from_unix_seconds(500)).unwrap_err();
        assert_eq!(error, SessionError::BadSeal);
    }

    #[test]
    fn rejects_a_tampered_cookie() {
        let cookie = seal(&key(), nonce(), &claims(1_000)).unwrap();
        let mut raw = URL_SAFE_NO_PAD.decode(&cookie).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0xff;
        let tampered = URL_SAFE_NO_PAD.encode(raw);
        let error = unseal(&key(), &tampered, Timestamp::from_unix_seconds(500)).unwrap_err();
        assert_eq!(error, SessionError::BadSeal);
    }

    #[test]
    fn rejects_an_expired_session() {
        let cookie = seal(&key(), nonce(), &claims(1_000)).unwrap();
        let error = unseal(&key(), &cookie, Timestamp::from_unix_seconds(1_000)).unwrap_err();
        assert_eq!(error, SessionError::Expired);
    }

    #[test]
    fn rejects_a_cookie_that_is_not_base64() {
        let error = unseal(&key(), "not base64!", Timestamp::from_unix_seconds(0)).unwrap_err();
        assert_eq!(error, SessionError::MalformedCookie);
    }

    #[test]
    fn rejects_a_cookie_shorter_than_a_nonce() {
        let short = URL_SAFE_NO_PAD.encode([0u8; NONCE_LEN]);
        let error = unseal(&key(), &short, Timestamp::from_unix_seconds(0)).unwrap_err();
        assert_eq!(error, SessionError::TruncatedCookie);
    }

    #[test]
    fn parses_a_base64_key_of_the_right_length() {
        let text = URL_SAFE_NO_PAD.encode([1u8; KEY_LEN]);
        assert!(SessionKey::from_base64(&text).is_ok());
    }

    #[test]
    fn rejects_a_base64_key_of_the_wrong_length() {
        let text = URL_SAFE_NO_PAD.encode([1u8; 16]);
        assert_eq!(
            SessionKey::from_base64(&text).err(),
            Some(SessionError::MalformedKey)
        );
    }

    #[test]
    fn is_fresh_at_is_strict_about_the_deadline() {
        let session = claims(1_000);
        assert!(session.is_fresh_at(Timestamp::from_unix_seconds(999)));
        assert!(!session.is_fresh_at(Timestamp::from_unix_seconds(1_000)));
    }
}
