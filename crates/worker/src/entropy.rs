//! Randomness, drawn from the host.
//!
//! The core takes nonces as arguments and never draws its own, so this is the
//! one place in noal that asks for entropy. On Workers `getrandom` forwards to
//! `crypto.getRandomValues`, which the runtime guarantees is present.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use noal_core::session::{Nonce, NONCE_LEN};

use crate::failure::Failure;

/// How many random bytes back an OAuth `state` value.
const STATE_LEN: usize = 32;

/// Draw a fresh nonce for sealing one session cookie.
///
/// # Errors
///
/// Returns [`Failure::Upstream`] when the host refuses entropy. Sealing with a
/// reused or predictable nonce would break the cipher, so there is no fallback.
pub fn nonce() -> Result<Nonce, Failure> {
    let mut bytes = [0_u8; NONCE_LEN];
    getrandom::fill(&mut bytes).map_err(|error| Failure::Upstream(error.to_string()))?;
    Ok(Nonce::from_bytes(bytes))
}

/// Draw a fresh OAuth `state` value.
///
/// # Errors
///
/// Returns [`Failure::Upstream`] when the host refuses entropy.
pub fn state_token() -> Result<String, Failure> {
    let mut bytes = [0_u8; STATE_LEN];
    getrandom::fill(&mut bytes).map_err(|error| Failure::Upstream(error.to_string()))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// Draw a fresh window id.
///
/// The bytes go through `uuid::Builder` rather than uuid's own `v4`
/// constructor, so this stays the only place in noal that draws randomness;
/// the `uuid` dependency is built without the `v4` feature to keep it that
/// way.
///
/// # Errors
///
/// Returns [`Failure::Upstream`] when the host refuses entropy.
pub fn window_id() -> Result<uuid::Uuid, Failure> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| Failure::Upstream(error.to_string()))?;
    Ok(uuid::Builder::from_random_bytes(bytes).into_uuid())
}
