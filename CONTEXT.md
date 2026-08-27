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

## A window URL answers with a full document, not a fragment

**Chosen over** content negotiation on the `HX-Request` header, which would
let `GET /w/:id` return a bare fragment when htmx asked and a document
otherwise.

Dated 2026-08-24. A saved window is reachable three ways that must behave
alike: clicking its link in the tree, following the URL htmx pushed after the
ask, and reloading the page later. All three land on the same complete page —
chrome, palette, answer, debug payload — rendered by one function through
`layout::page`. A second rendering path for "inside htmx" would be a second
template family to keep in step, which the maud-and-htmx decision above
bans; htmx navigates to a full document perfectly well on its own.

The same page load is also where reopening happens: the server runs the
window's stored query through its stored template before rendering, so the
document always arrives filled and current (`Cache-Control: no-store`,
`hx-history="false"`), never from a cache or a snapshot. Reopening calls no
model — the stored plan and template are used as-is, and if Postgres refuses
the query the ask ends rather than retrying with the model's help.

---

## A follow-up ask refines the window it came from

**Chosen over** a hidden form field carrying the parent id, which would go
stale from the second ask onward, and over dropping to a root ask when the
named window does not exist, which would answer a question nobody asked.

Dated 2026-08-25. The address bar is the one copy of which window the user
stands on: an ask made while standing on a window (`HX-Current-URL` naming
`/w/<segment>`) is planned from that window's query and rendered in its
presentation. An ask made at the root carries nothing. There is no clear
control — a fresh line of enquiry starts at the root.

A well-formed segment that cannot be a uuid, names no row, or names another
user's row refuses as a 404 toast rather than being answered from the root:
one status and one wording for every miss keep the route from becoming a
probe for which windows exist. The saved child records its parent, so the
tree shows the refinement; depth is the immediate parent only.

---

## A re-run tells the truth about itself

**Chosen over** rendering blank cells when the stored columns no longer match
what the query returns, over reusing first-ask refusal wording on a window
whose artifacts were written days ago, and over a script-driven refresh
button, which would put navigation back into the page that d3's verification
removed it from.

Dated 2026-08-25. A reopened window states its age — "Saved <date> · data
re-read on arrival", date only, UTC, day granularity, the full RFC 3339
instant in `<time datetime>` with no href. The value is read once by the
shell (`extract(epoch from created_at)`), carried on the `Window` struct,
and formatted by pure `Timestamp` methods that never read a clock; nothing
in noal writes the column.

Shape drift refuses loudly at Fill: before filling, a reopened ask compares
the returned columns against the stored shape as sets, and any mismatch ends
the ask with the diff recorded in `debug.attempts` — never a page of blank
cells that looks like an empty result. An empty or non-array result cannot
be inspected and passes (accepted fog: all rows gone looks like a legitimate
empty answer). A first ask is not gated; the model still holds the plan it
just wrote.

Refusal wording knows where the artifact came from:
`failure_text(stage, origin)` keeps the first-ask sentences ("noal could not
run the query it wrote") and gives re-runs their own ("This window no longer
works: the query it saved was refused."), because an old artifact working
once and failing now is a different fact than a fresh mistake.

Refresh is arrival at the window you stand on: an ordinary anchor (↻) sits
beside the current tree row alone — absent from Home, other rows, and every
non-window page — and its click is a full document load of the same URL,
which *is* the re-run, with no noal script involved.

---

## The palette and the toast region are page chrome

**Chosen over** wiring the ask form itself to notice a failed request, over
folding a failure message into `#palette`'s own markup, and over giving a
refused pipeline stage its own status code.

`layout::page()` renders `#toasts` and a hidden `#toast-offline` template
beside `#palette`, and `OVERLAY_SCRIPT` listens for `htmx:responseError` and
`htmx:sendError` on `document.body` rather than on `#ask-form`. One listener
answers every htmx request a page makes, not only `/ask`'s. A `401` sends the
browser to `/auth/login?next=`; anything else appends the failed response's own
toast body, or clones the offline template when there is no body to append.

A refused pipeline stage still answers `200`. htmx only runs the out-of-band
swap that keeps the debug panel current on a response it treats as successful,
so moving a refusal onto its own status code would cost every refused ask its
debug payload. Everything that is not a `200` is handled in the browser
instead, by the listener above.

A form that read its own errors was rejected too: every page with an ask form
would re-implement the same handling, rather than one page-chrome region
answering for the whole document. The cost is the one already paid for the
palette itself: an anonymous viewer gets no toast region, because `#toasts`
exists only where `#palette` does.

---

## A rename submit drops its double, and a refused name names the limit

**Chosen over** queueing rapid submits (htmx's default), over disabling the
whole form during flight, and over `maxlength` on the input.

Dated 2026-08-26. The rename form is the second POST form in the app, so it
carries the same pair as the ask form: `hx-sync="this:drop"` drops a submit
made while the first is still in flight — per element, so renaming two windows
never interferes — and `hx-disabled-elt="find .window-rename-submit"` makes the
Save button the visible sign of the flight. Cancel stays out of the attribute's
reach: putting an editor away mid-rename stays possible. The write is idempotent
in effect either way; the guard buys consistency with the ask form and an honest
in-flight sign, not correctness.

The too-long refusal names the number — "A window name can be at most 200
characters." — because "too long to store" left the viewer guessing how far over
they went. `maxlength` was rejected: the browser silently truncates an over-long
paste, storing nothing yet telling the viewer nothing, while the core rule for
names is refuse whole, never truncate. The server stays the only gate. The
wording is a baked static with a guard test pinning `NAME_LIMIT == 200`, so the
constant cannot move without the prose following.

---

## One stylesheet carries two themes and the drawer's own dark, through custom properties

**Chosen over** Tailwind JIT vendored into the binary (≈15–25 KB gzipped for a
CSS string const), over a private utility vocabulary the model would have to
learn from scratch, and over per-component ad-hoc rules.

`const STYLE` in `crates/view/src/layout.rs` is one hand-authored sheet. `:root`
defines Zinc light variables for content pages;
`@media (prefers-color-scheme: dark)` overrides them so content follows the
system preference; the palette drawer hardcodes its own near-black zone with
sky-blue accents, reading none of the theme variables — the drawer is dark no
matter what the viewer's OS prefers. Inter loads from its CDN in `page()`'s
head, ahead of the stylesheet, with a system stack as fallback, so a blocked CDN
degrades to system-ui instead of blocking first paint. Component classes
(`.btn`, `.btn-primary`, `.btn-ghost`, `.input`, `.card`, `.toast`, `.tab-active`,
`.tree-row`) and small utilities (`.flex`, `.gap-*`, `.mt-1`, `.border-b`,
`.muted`, `.text-sm`, …) carry the shared look; every page chrome — header nav,
toasts, palette tabs and debug panel, window tree rows and rename form, ask form
and result sections, home/window/failure pages — wears them. Structural names
(ids, `hx-*`, classes scripts bind to like `window-rename-submit`) are untouched,
so htmx wiring and `OVERLAY_SCRIPT`'s assumptions never moved.

A rule can survive on the sheet without being attached anywhere yet; the sheet is
the catalog, not the markup. Shared-class rules lose to more specific selectors on
the same element, so where an older selector owned an element's styling that later
took a shared class (`#ask-form input`, `.window-rename input/button`,
`#debug-copy`, `.toast-dismiss`), the old rule was retired or narrowed in the same
change that attached the class — never left competing underneath. Source order is load-bearing where equal specificity
meets (`.card` before `.toast`; light `:root` before the dark override); tests
pin those orders so reordering fails loudly.

---

## The model learns the stylesheet's class names from its prompt, not from reading source

**Chosen over** letting templates invent class names (unstyled output), over
documenting them only in this file (the model never sees it), and over teaching
Tera or CSS in the preamble (the model already knows both).

Model-written Tera templates render inside `<section class="card"
id="ask-result">`, so anything they emit lands on the design system if they name
its classes. `noal_view::render::CSS_CLASS_GUIDE` lists exactly those: the
component and utility classes above, plus bare-element notes (tables and links
need no class). `template_preamble()` joins core's `RENDER_PREAMBLE` — the
security and honesty rules keep their lead — with that list, and the worker's
render-stage call passes it as the system message. Two parity tests walk both
directions mechanically: every non-machinery class in the stylesheet must be
offered by the guide, and the guide may offer nothing the stylesheet lacks, so
the prose cannot drift from the sheet. Machinery-only names (`htmx-request`,
`htmx-indicator`, `sign-out`, the tabs' `active`, `window-rename-open`) stay out:
templates have no use for them. `.toast-dismiss` is documented despite styling
nothing, because it is the hook `OVERLAY_SCRIPT` binds dismissal to.

