# noal

A Rust web application on Cloudflare Workers. Read `README.md` first for the
shape of the repository; this file records the rules that are not obvious from
the code.

## The one rule

**Functional core, imperative shell.**

`crates/core` and `crates/view` are pure. They must not read a file, open a
socket, read a clock, or draw randomness. When a rule needs the time or a
nonce, the caller passes it in.

`crates/worker` owns every effect. Its handlers hold no logic worth testing;
they gather data, hand it to the core, and render what comes back.

Two things follow, and both matter:

- The core compiles natively, so `cargo test` runs it without the Wasm
  toolchain. A test that needs `wrangler` is a sign that logic is in the wrong
  crate.
- A rule is tested by passing values, not by building a world.

If you find yourself wanting a clock inside `noal_core`, add a `Timestamp`
argument instead. `state::now()` and `entropy::` are the only places in noal
that read a clock or draw randomness, and they are both in the shell.

## Two targets, never together

`cargo check --workspace` **fails**. The Worker compiles only for
`wasm32-unknown-unknown`, and the native crates cannot build for it.

Always one of:

```sh
cargo check --workspace --exclude noal_worker --all-targets
cargo check -p noal_worker --target wasm32-unknown-unknown
```

`cargo xtask lint` runs both, in that order. Use it rather than remembering.

## Wasm constraints that bite

These were found the hard way. The evidence is in
`spikes/workers-postgres/README.md`.

- **`[profile.release]` must not set `panic = "abort"` or `strip = true`.**
  wasm-bindgen emits catch wrappers that need an externref table, and either
  setting breaks the build with a message that does not name the cause.

- **axum needs `Send` handler futures; the Workers JS bindings are `Rc`-based.**
  The Postgres path is `Send` and needs nothing. The `worker::Fetch` path is
  not, because it resolves a `JsFuture`. `send_wrapper::SendWrapper` fixes it,
  and is confined to `routes::auth::post_json`. Do not spread it; if a new
  handler needs it, that handler is talking to JavaScript, and the wrapper
  belongs around that call alone.

- **`getrandom` appears twice.** Version 0.3 is used directly. Version 0.2
  arrives under `chacha20poly1305`, and refuses to compile for Wasm unless the
  `js` feature is on. `crates/core/Cargo.toml` declares it as
  `getrandom_legacy` for that reason only.

- **`jsonwebtoken` does not build for `wasm32-unknown-unknown`.** This is why
  noal never verifies a WorkOS token signature on the hot path; see below.

- **`worker` is pinned to `=0.8.3`.** From 0.8.4, `worker` requires
  `wasm-streams ^0.6`, while `reqwest` (pulled in by `rig-core` for the model
  client) requires `^0.5`. Cargo cannot unify the two, links both copies, and
  their `wasm_bindgen` glue symbols collide at the linker with duplicate
  `intounderlyingbytesource_*`/`intounderlyingsink_*`/`intounderlyingsource_*`
  errors. Lift the pin once a published `reqwest` requires `wasm-streams ^0.6`.

- **Prefer `simple_query` over `query` when Hyperdrive caching is off.**
  A prepared statement needs the cache. `/health/db` uses `simple_query` so it
  works in every configuration.

## Authentication

Unsealing the session cookie **is** the authentication. There is no per-request
signature check and no per-request network call.

WorkOS hands noal an access token and a refresh token exactly once, in the
callback. `noal_core::auth::claims_from_tokens` reads the access token's
payload **without verifying its signature** — safe there and only there,
because the token arrived in a TLS response to a request noal made with its own
API key. The claims are then sealed into noal's own cookie with noal's own key.
No untrusted input ever reaches that parser.

`session::unseal` returns `Expired` rather than a stale claim, so a successful
unseal always means "genuine and fresh". A handler cannot forget to check.

Reading the cookie happens in exactly one place: the `extract::SignedIn` and
`extract::Visitor` extractors. Do not read `COOKIE_NAME` anywhere else.

## Secrets

`SessionKey` and `Config` have hand-written `Debug` implementations that redact.
Do not replace them with derives. Rotating `SESSION_KEY` signs everyone out.

`Failure::message` is what the browser sees; `Failure::detail` is what the log
sees. Keep them apart. A caller who learns *why* a session failed learns
whether their forged cookie was close.

## Style

Follow the house rules the lint gate enforces:

- `#![deny(clippy::unwrap_used, clippy::expect_used)]` at every crate root.
  Tests may unwrap under `#[allow]`.
- No file over 1000 lines.
- `#[allow(clippy::too_many_arguments)]` is banned. Take a struct.
- Every public item has a doc comment. `missing_docs` is denied.
- Comments say *why*, not *what*.

Run `cargo xtask lint` before saying something is done.
