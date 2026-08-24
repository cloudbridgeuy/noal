# Decisions

Why noal is built the way it is. Each entry records what was chosen, what it
was chosen over, and what it costs. Add to this file when a decision is made,
not when it is questioned.

Dated 2026-08-22 unless noted.

---

## Cloudflare Workers, compiled to Wasm

**Chosen over** Cloudflare Containers, Fly.io, and a plain Linux host.

The application is small, spiky, and mostly I/O. Workers bill for that shape and
start with no cold container. The cost is real: no threads, no filesystem, a
3 MB gzipped bundle limit, and a `Send` constraint that JavaScript bindings do
not satisfy.

The decision was not taken on paper. `spikes/workers-postgres/` proved the whole
stack — axum, `tokio-postgres`, Hyperdrive, outbound HTTPS, `maud` — before any
of it was written for real. It came to **338 KB gzipped**, about 11% of the
limit. The scaffold itself builds to **394 KB gzipped**, about 12%. CI reports
the number on every run, so a dependency that doubles it is visible in the pull
request rather than at deploy time.

**What is still unproved:** the spike ran against local Postgres 16, never
against Neon. Neon adds TLS and SNI through Hyperdrive. Verify before deploying.

---

## `tokio-postgres` over `worker::Socket`, not `sqlx`

**Chosen over** `sqlx` with compile-time query checking, which was the earlier
plan.

`sqlx` cannot run on `wasm32-unknown-unknown`. `worker::Socket` implements
tokio's `AsyncRead` and `AsyncWrite`, and `tokio_postgres::Config::connect_raw`
accepts anything that does — so the driver works unmodified.

The cost is losing compile-time query checking. Queries are strings again.
Mitigate by keeping row-to-domain conversion in one place per table.

Hyperdrive terminates TLS at the Workers layer, so the driver is handed
`PassthroughTls` and does not negotiate a second time.

**Note:** guidance published in 2023 — a `devsnek` fork, a hand-rolled
`EdgeHyperdrive`, no prepared statements — is obsolete as of `worker` 0.8.5.
None of it is needed.

---

## Self-sealed session cookie, not per-request token verification

**Chosen over** verifying the WorkOS RS256 access token on every request against
its JWKS.

Two reasons, in order of weight:

1. `jsonwebtoken`'s crypto backend does not build for
   `wasm32-unknown-unknown`. Per-request verification is not merely slow here;
   it is unavailable.
2. Verification on the hot path would mean a JWKS fetch, which is a network
   call inside every page load.

So noal seals the claims into its own cookie with ChaCha20-Poly1305 — pure Rust,
`Send`, and it compiles for Wasm. Unsealing *is* the authentication.

The cost: revocation is not instant on this side. A sealed cookie stays valid
until the access token's own `exp`. WorkOS session revocation handles
"sign out everywhere", but a revoked session can still read noal until its
cookie expires. Shorten the access token lifetime in WorkOS if that window
matters.

---

## WorkOS AuthKit, not Neon Auth

**Chosen over** Neon Auth (Managed Better Auth), which would have put identity
in the same database as everything else and removed a vendor.

Neon Auth is in Beta. Identity is the wrong place to carry Beta risk.

---

## `maud` and htmx, not a JSON API with a client framework

**Chosen over** an SPA.

`maud` templates are compile-time, so there is no runtime template loading —
which matters when there is no filesystem. htmx keeps the wire format HTML, so
`crates/view` stays the single source of markup and there is no second
rendering path to keep in step.

---

## `cargo xtask migrate`, not `sqlx migrate` or Atlas

**Chosen over** a dedicated migration tool.

The runner is under 200 lines of shell around a pure core, it has no install
step, and it runs natively — so it uses ordinary `tokio-postgres` with real TLS
rather than anything Wasm-specific.

Each migration runs in a transaction with the row that records it. The runner
also refuses to proceed when the ledger holds a version this checkout does not
have, which is what stops an old deploy from migrating a newer database.

---

## Four crates, split by purity

`core` and `view` are pure and native. `worker` is the shell and Wasm-only.
`xtask` is tooling.

The split is enforced by the target, not by discipline: `noal_core` cannot call
`worker::` because `worker` is not one of its dependencies, and it never will
be. That makes the boundary structural.

---

## The palette and the toast region are page chrome

**Chosen over** wiring the ask form itself to notice a failed request, and
over folding a failure message into `#palette`'s own markup.

`layout::page()` renders `#toasts` and a hidden `#toast-offline` template
beside `#palette`, all three under the same `show_palette` condition, and
`OVERLAY_SCRIPT` listens for `htmx:responseError` and `htmx:sendError` on
`document.body` rather than on `#ask-form`. One listener then answers every
htmx request a page makes, not only `/ask`'s. A `401` sends the browser to
`/auth/login?next=`; anything else appends the failed response's own toast
body, or clones the offline template when there is no body to append — a
dropped connection, or the Worker itself unreachable.

The layout now knows something about `/ask` it did not before: a `401` from
an htmx request means a session ran out. `OVERLAY_SCRIPT`'s own doc comment
used to claim otherwise; that line is corrected, not merely stale.

A refused pipeline stage still answers `200`. That is deliberate and carried
over from the slice before this one: `render_outcome` retargets the swap to
`#toasts` but keeps the status, because htmx only runs the out-of-band swap
that keeps the debug panel current on a response it treats as successful.
Moving a refusal onto its own status code was rejected for that reason — it
would cost every refused ask its debug payload. Everything that is not a
`200` is instead handled entirely in the browser, by the listener above,
never inside the pipeline.

A form in the page body that read its own errors was rejected too: it would
mean every page with an ask form re-implements the same handling, rather than
one page-chrome region — the toasts, the offline template, the listener —
answering for the whole document once. The cost is the one already paid for
the palette itself: an anonymous viewer gets no toast region either, because
`#toasts` exists only where `#palette` does.
