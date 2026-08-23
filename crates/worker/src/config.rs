//! Configuration, parsed once at the edge of the request.
//!
//! Every value the Worker needs from its environment is read here and turned
//! into a type that cannot be wrong. The rest of the shell takes a [`Config`]
//! and never touches `env.secret` or `env.var` again, so a missing binding is
//! one clear failure at startup instead of a surprise deep inside a handler.

use noal_core::session::{SessionError, SessionKey};
use worker::Env;

/// Every setting the Worker reads from its environment.
#[derive(Clone)]
pub struct Config {
    /// The key that seals and unseals the session cookie.
    pub session_key: SessionKey,
    /// The WorkOS client identifier. Public; it appears in the authorize URL.
    pub workos_client_id: String,
    /// The WorkOS API key. Secret; it must never reach a response or a log.
    pub workos_api_key: String,
    /// Where WorkOS sends the browser back after a sign-in.
    pub redirect_uri: String,
    /// The Anthropic API key. Secret; it must never reach a response or a log.
    pub anthropic_api_key: String,
    /// The Cloudflare AI Gateway token, when calls go through a gateway.
    /// Secret. `None` means no `cf-aig-authorization` header is sent.
    pub ai_gateway_token: Option<String>,
    /// Where the Anthropic API is reached: the gateway's Anthropic path, or
    /// `https://api.anthropic.com` to call it directly.
    pub llm_base_url: String,
}

impl std::fmt::Debug for Config {
    /// Print the public settings and redact the secret one.
    ///
    /// `SessionKey` redacts itself, but `workos_api_key` is a plain `String`
    /// and a derived `Debug` would print it into any log line that reports a
    /// config. This one cannot.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("session_key", &self.session_key)
            .field("workos_client_id", &self.workos_client_id)
            .field("workos_api_key", &"<redacted>")
            .field("redirect_uri", &self.redirect_uri)
            .field("anthropic_api_key", &"<redacted>")
            .field(
                "ai_gateway_token",
                &self.ai_gateway_token.as_ref().map(|_| "<redacted>"),
            )
            .field("llm_base_url", &self.llm_base_url)
            .finish()
    }
}

/// Why the environment could not produce a [`Config`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// A binding was absent or empty.
    #[error("`{0}` is not set; add it to wrangler.jsonc or .dev.vars")]
    Missing(&'static str),
    /// `SESSION_KEY` was present but not a usable key.
    #[error("`SESSION_KEY` is malformed: {0}")]
    MalformedSessionKey(SessionError),
}

impl Config {
    /// Read and parse the whole environment.
    ///
    /// # Errors
    ///
    /// Returns the first [`ConfigError`] found. Bindings are checked in a fixed
    /// order so the message is the same on every cold start.
    pub fn from_env(env: &Env) -> Result<Self, ConfigError> {
        let session_key = SessionKey::from_base64(&secret(env, "SESSION_KEY")?)
            .map_err(ConfigError::MalformedSessionKey)?;

        Ok(Self {
            session_key,
            workos_client_id: var(env, "WORKOS_CLIENT_ID")?,
            workos_api_key: secret(env, "WORKOS_API_KEY")?,
            redirect_uri: var(env, "REDIRECT_URI")?,
            anthropic_api_key: secret(env, "ANTHROPIC_API_KEY")?,
            ai_gateway_token: secret(env, "AI_GATEWAY_TOKEN").ok(),
            llm_base_url: var(env, "LLM_BASE_URL")?,
        })
    }
}

/// Read a plain variable, treating empty as absent.
fn var(env: &Env, name: &'static str) -> Result<String, ConfigError> {
    env.var(name)
        .ok()
        .map(|value| value.to_string())
        .filter(|value| !value.is_empty())
        .ok_or(ConfigError::Missing(name))
}

/// Read a secret, treating empty as absent.
///
/// `wrangler dev` exposes `.dev.vars` entries as variables rather than secrets,
/// so this falls back to [`var`] and works the same in both places.
fn secret(env: &Env, name: &'static str) -> Result<String, ConfigError> {
    env.secret(name)
        .ok()
        .map(|value| value.to_string())
        .filter(|value| !value.is_empty())
        .map_or_else(|| var(env, name), Ok)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::Config;
    use noal_core::session::SessionKey;

    /// A `Config` with recognisable, non-empty values in every secret field,
    /// so a redaction bug shows up as that value appearing in `{:?}` output.
    fn config() -> Config {
        Config {
            session_key: SessionKey::from_bytes([7u8; 32]),
            workos_client_id: "client-public".to_owned(),
            workos_api_key: "workos-secret-value".to_owned(),
            redirect_uri: "http://localhost:8787/auth/callback".to_owned(),
            anthropic_api_key: "anthropic-secret-value".to_owned(),
            ai_gateway_token: Some("gateway-secret-value".to_owned()),
            llm_base_url: "https://api.anthropic.com".to_owned(),
        }
    }

    #[test]
    fn debug_output_holds_no_key_material() {
        let printed = format!("{:?}", config());

        assert!(!printed.contains("workos-secret-value"));
        assert!(!printed.contains("anthropic-secret-value"));
        assert!(!printed.contains("gateway-secret-value"));

        // The public fields still print, so the redaction is not hiding the
        // whole struct — only the secrets.
        assert!(printed.contains("client-public"));
        assert!(printed.contains("https://api.anthropic.com"));
    }

    #[test]
    fn debug_output_redacts_a_missing_gateway_token_the_same_way() {
        let mut without_gateway = config();
        without_gateway.ai_gateway_token = None;

        let printed = format!("{:?}", without_gateway);

        assert!(printed.contains("ai_gateway_token: None"));
    }
}
