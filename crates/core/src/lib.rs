//! The functional core of noal.
//!
//! Everything here is a pure function over owned data. This crate must not
//! read a file, open a socket, read a clock, or draw randomness. When a rule
//! needs the time or a random nonce, the caller passes it in. The shell in
//! `noal_worker` owns those effects.
//!
//! That constraint buys two things. The core compiles for the host, so
//! `cargo test` runs it natively without the Wasm toolchain. And every rule is
//! testable by supplying values instead of by building a world.
//!
//! See `~/.claude/patterns/functional-core-imperative-shell.md`.
#![deny(clippy::unwrap_used, clippy::expect_used)]
#![deny(missing_docs)]

pub mod ask;
pub mod auth;
pub mod clock;
pub mod cookie;
pub mod session;
