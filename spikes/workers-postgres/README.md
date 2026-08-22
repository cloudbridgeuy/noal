# Spike: `axum` + `tokio-postgres` on Cloudflare Workers

## Question

Can `noal` run on Cloudflare Workers as Wasm, talk to Neon Postgres, and
render htmx fragments? The 2023 write-ups said no. This spike tested it.

## Result: yes. All five claims pass.

Evidence came from `wrangler dev` against Postgres 16 in Docker.

| # | Claim | Result |
|---|-------|--------|
| 1 | The stack compiles to `wasm32-unknown-unknown` | PASS |
| 2 | `env.hyperdrive("DB")` resolves | PASS — `host=…hyperdrive.local port=5432` |
| 3 | `tokio-postgres` handshakes over `worker::Socket` | PASS — `[StartTls] PostgreSQL 16.15` |
| 4a | `simple_query` works | PASS |
| 4b | `query` (prepared statement) works | PASS — `echo=noal` |
| 4c | `SELECT` from a real table | PASS — 2 rows |
| 5 | Outbound HTTPS (WorkOS) | PASS — `status=200` |
| 6 | Bundle size | 843 KB raw, **338 KB gzipped** (11% of the 3 MB free limit) |

## What changed since the 2023 write-ups

Those posts are obsolete. Do not follow them.

- `worker` 0.8.5 exports `Hyperdrive`. No hand-rolled `EdgeHyperdrive` binding.
- `worker` has a `tokio-postgres` feature. It supplies
  `worker::postgres_tls::PassthroughTls` and `TlsConnect<Socket>`.
- Upstream `tokio-postgres` 0.7 works. The `devsnek` fork is unnecessary.
- Prepared statements work.

## Findings to carry into the scaffold

1. **The database futures are `Send`.** `Env`, `AppState`,
   `tokio_postgres::Client`, and `Socket` are all `Send + Sync` on wasm32.
   axum handlers accept them without a wrapper.

2. **The JS futures are not `Send`.** `worker::Fetch` resolves a `JsFuture`,
   which holds `Rc<RefCell<…>>`. axum demands `Send` handler futures. Wrap the
   JS-facing part in `send_wrapper::SendWrapper`. This is sound because
   `wasm32-unknown-unknown` has one thread. **This hits WorkOS, not Neon.**

3. **Do not put `panic = "abort"` or `strip = true` in `[profile.release]`.**
   `wasm-bindgen` then fails with `externref table required for catch
   wrappers`. Keep `opt-level = "z"`, `lto`, and `codegen-units = 1`.

4. **TLS transport differs by environment.** Neon needs
   `SecureTransport::StartTls`. A local Postgres speaks plaintext. `connect_any`
   tries `StartTls` first, then `Off`.

## Not yet verified

- **Real Neon.** This ran against local Postgres. The Neon endpoint adds TLS
  and SNI through Hyperdrive.
- **Hyperdrive with caching off.** Cloudflare warns that `query` needs a
  prepared statement, which fails when caching is disabled. Only a real
  Hyperdrive config can test this. Prefer `simple_query` if you disable caching.

## Repro

```sh
docker run -d --name noal-spike-pg \
  -e POSTGRES_PASSWORD=spike -e POSTGRES_USER=spike -e POSTGRES_DB=spike \
  -p 55432:5432 postgres:16
docker exec noal-spike-pg psql -U spike -d spike \
  -c "CREATE TABLE note (id SERIAL PRIMARY KEY, body TEXT NOT NULL);" \
  -c "INSERT INTO note (body) VALUES ('first note'),('second note');"

npm install
WRANGLER_HYPERDRIVE_LOCAL_CONNECTION_STRING_DB="postgres://spike:spike@localhost:55432/spike" \
  ./node_modules/.bin/wrangler dev --port 8787
```

Then open <http://localhost:8787> and press the buttons, or:

```sh
curl -s http://localhost:8787/probe
curl -s http://localhost:8787/rows
```

To test against Neon, put the Neon connection string in that same variable.
