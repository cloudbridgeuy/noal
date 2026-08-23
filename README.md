# noal

A Rust web application that runs on Cloudflare Workers.

It serves server-rendered HTML to [htmx](https://htmx.org), reads and writes
[Neon](https://neon.tech) Postgres through
[Hyperdrive](https://developers.cloudflare.com/hyperdrive/), and authenticates
with [WorkOS AuthKit](https://workos.com/docs/authkit).

There is no domain yet. `crates/core` holds the session and authentication
rules and nothing else; that is the seam where the application goes.
`docs/design.md` describes what will go there.

## Shape

| Crate          | Target                  | Holds                                        |
| -------------- | ----------------------- | -------------------------------------------- |
| `crates/core`  | native                  | Pure rules. No I/O, no clock, no randomness.  |
| `crates/view`  | native                  | `maud` templates. Data in, markup out.        |
| `crates/worker`| `wasm32-unknown-unknown`| The shell: axum, Postgres, WorkOS, the clock. |
| `xtask`        | native                  | The lint gate and the migration runner.       |

The core and the views compile natively, so `cargo test` runs them without the
Wasm toolchain. Only the shell needs the Workers target.

That split is a rule, not a habit. See
[Functional Core, Imperative Shell](https://www.destroyallsoftware.com/talks/boundaries).

## Requirements

- Rust 1.95.0 (pinned in `rust-toolchain.toml`, with the Wasm target)
- Node, for `wrangler`
- `worker-build`: `cargo install worker-build`

## Run it

```sh
npm install
cp .dev.vars.example .dev.vars   # then fill it in
npm run dev
```

`.dev.vars` needs a session key, a WorkOS client ID and API key, and a Postgres
URL. The example file explains each one and how to generate the key.

`wrangler dev` connects straight to the Postgres URL in
`WRANGLER_HYPERDRIVE_LOCAL_CONNECTION_STRING_DB`, so a local run needs no
Cloudflare resource. A deploy needs a real Hyperdrive `id` in `wrangler.jsonc`.

## Routes

| Method | Path              | What it does                                   |
| ------ | ----------------- | ---------------------------------------------- |
| GET    | `/`               | The home page                                  |
| GET    | `/auth/login`     | Redirects to WorkOS                            |
| GET    | `/auth/callback`  | Exchanges the code, sets the session cookie    |
| POST   | `/auth/logout`    | Revokes at WorkOS, clears the cookie           |
| GET    | `/health`         | Answers from the isolate alone                 |
| GET    | `/health/db`      | Answers only after Postgres has                |

## Database

Migrations are numbered `.sql` files in `migrations/`, applied in order and
recorded in a `schema_migrations` table:

```sh
export DATABASE_URL='postgres://...'
cargo xtask migrate --dry-run   # report what would run
cargo xtask migrate             # run it
```

Each migration runs inside a transaction together with the row that records it,
so a failure leaves neither behind. Never edit a migration that has been
applied; add a new one.

## Checks

```sh
cargo xtask lint              # everything CI runs, in one command
cargo xtask lint --fix        # and fix what can be fixed
cargo xtask lint --install-hooks
```

The gate runs formatting, `check` and `clippy` on both targets, the tests, a
1000-line file cap, and a ban on `#[allow(clippy::too_many_arguments)]`.

`bacon` watches the native crates by default; `bacon check-wasm` watches the
Worker.

## Evidence

The stack was proved before it was built. `spikes/workers-postgres/README.md`
records what was tested, what passed, and what is still unverified — including
that this has never run against a real Neon instance.

## License

MIT.
