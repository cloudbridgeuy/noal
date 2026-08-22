//! The state every handler shares, and the database connection it opens.
//!
//! `AppState` is cloned per request by axum, so it holds only cheap handles:
//! the Workers `Env` and a reference-counted [`Config`]. Nothing is pooled,
//! because a Worker isolate serves one request at a time and Hyperdrive keeps
//! the real pool on Cloudflare's side.

use std::sync::Arc;

use noal_core::clock::Timestamp;
use worker::postgres_tls::PassthroughTls;
use worker::{Env, SecureTransport, Socket};

use crate::config::Config;
use crate::failure::Failure;

/// Handles shared by every handler.
#[derive(Clone)]
pub struct AppState {
    /// The Workers environment: bindings, variables, and secrets.
    env: Env,
    /// The parsed configuration, shared by every clone of this state.
    config: Arc<Config>,
}

impl AppState {
    /// Parse the environment once, at the start of the fetch event.
    ///
    /// # Errors
    ///
    /// Returns [`Failure::Config`] when a binding is missing or malformed.
    pub fn new(env: Env) -> Result<Self, Failure> {
        let config = Config::from_env(&env)?;
        Ok(Self {
            env,
            config: Arc::new(config),
        })
    }

    /// The parsed configuration.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Open one Postgres connection through the Hyperdrive binding.
    ///
    /// The Workers socket terminates TLS itself (`StartTls`), so the driver is
    /// given [`PassthroughTls`] and does not negotiate a second time. The
    /// connection task is spawned with `spawn_local`, which is the Wasm
    /// equivalent of `tokio::spawn`; the returned client stops working the
    /// moment that task ends.
    ///
    /// # Errors
    ///
    /// Returns [`Failure::Database`] when the binding is absent, the socket
    /// cannot open, or the startup handshake fails.
    pub async fn database(&self) -> Result<tokio_postgres::Client, Failure> {
        let hyperdrive = self.env.hyperdrive("DB").map_err(Failure::database)?;

        let socket = Socket::builder()
            .secure_transport(SecureTransport::StartTls)
            .connect(hyperdrive.host(), hyperdrive.port())
            .map_err(Failure::database)?;

        let config = hyperdrive
            .connection_string()
            .parse::<tokio_postgres::Config>()
            .map_err(Failure::database)?;

        let (client, connection) = config
            .connect_raw(socket, PassthroughTls)
            .await
            .map_err(Failure::database)?;

        wasm_bindgen_futures::spawn_local(async move {
            if let Err(error) = connection.await {
                worker::console_error!("database connection ended: {error}");
            }
        });

        Ok(client)
    }
}

/// The current time, read from the host.
///
/// The core takes time as an argument and never reads a clock, so this is the
/// one place in noal that asks what time it is. `chrono`'s `wasmbind` feature
/// routes this through the JavaScript `Date` object.
#[must_use]
pub fn now() -> Timestamp {
    Timestamp::from_unix_seconds(chrono::Utc::now().timestamp())
}

/// The current time in milliseconds since the Unix epoch.
///
/// `now()` gives whole seconds, too coarse for timing a pipeline stage that
/// can finish in tens of milliseconds. This is the only other place in noal
/// that reads a clock; the debug panel's `Timing` values are built from it
/// and passed into `noal_core` as plain numbers.
#[must_use]
pub fn now_millis() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0)
}
