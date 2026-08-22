//! Spike: does `axum` + `tokio-postgres` + Neon (through Hyperdrive) work on
//! Cloudflare Workers, and can it render an htmx fragment from a real row?
//!
//! This crate exists to produce evidence, not to be kept. Every route reports
//! what happened instead of hiding it, so a failure is legible in the browser.
//!
//! The five claims under test:
//!
//! 1. The whole stack compiles to `wasm32-unknown-unknown`.
//! 2. `env.hyperdrive("DB")` resolves the binding.
//! 3. `tokio-postgres` completes a startup handshake over `worker::Socket`.
//! 4. Both query protocols work: `simple_query`, and the `query` prepared
//!    statement that Cloudflare warns about when Hyperdrive caching is off.
//! 5. Outbound HTTPS works, which is what WorkOS AuthKit needs.

use axum::extract::State;
use axum::response::Html;
use axum::routing::get;
use axum::Router;
use maud::{html, Markup, DOCTYPE};
use send_wrapper::SendWrapper;
use tower_service::Service;
use worker::postgres_tls::PassthroughTls;
use worker::{console_log, event, Context, Env, Fetch, HttpRequest, SecureTransport, Socket, Url};

/// One probe result. A check either produces a detail string or an error
/// string; there is no third state.
type Check = (&'static str, Result<String, String>);

#[derive(Clone)]
struct AppState {
    env: Env,
}

// ---------------------------------------------------------------------------
// Database
// ---------------------------------------------------------------------------

/// Open one Postgres connection through the Hyperdrive binding.
///
/// The socket does TLS at the Workers layer (`StartTls`), so `tokio-postgres`
/// gets `PassthroughTls` and does not try to negotiate TLS a second time.
async fn connect(env: &Env, transport: SecureTransport) -> Result<tokio_postgres::Client, String> {
    let hyperdrive = env
        .hyperdrive("DB")
        .map_err(|e| format!("hyperdrive binding: {e}"))?;

    let socket = Socket::builder()
        .secure_transport(transport)
        .connect(hyperdrive.host(), hyperdrive.port())
        .map_err(|e| format!("socket connect: {e}"))?;

    let config = hyperdrive
        .connection_string()
        .parse::<tokio_postgres::Config>()
        .map_err(|e| format!("parse connection string: {e}"))?;

    let (client, connection) = config
        .connect_raw(socket, PassthroughTls)
        .await
        .map_err(|e| format!("connect_raw: {e}"))?;

    // The connection future drives the socket. It must run for the client to
    // make progress, and `spawn_local` is the Wasm stand-in for `tokio::spawn`.
    wasm_bindgen_futures::spawn_local(async move {
        if let Err(error) = connection.await {
            console_log!("connection error: {error:?}");
        }
    });

    Ok(client)
}

/// Neon needs TLS. A local Postgres in `wrangler dev` speaks plaintext. Try the
/// production transport first and fall back, then report which one won, so the
/// spike says plainly what the environment actually did.
async fn connect_any(env: &Env) -> Result<(tokio_postgres::Client, &'static str), String> {
    let start_tls = match connect(env, SecureTransport::StartTls).await {
        Ok(client) => return Ok((client, "StartTls")),
        Err(error) => error,
    };
    match connect(env, SecureTransport::Off).await {
        Ok(client) => Ok((client, "Off (plaintext)")),
        Err(off) => Err(format!("StartTls failed: {start_tls}; Off failed: {off}")),
    }
}

/// One row of the `note` table, parsed out of the driver's dynamic row type at
/// the boundary so the view never touches a `Row`.
struct Note {
    id: i32,
    body: String,
}

// ---------------------------------------------------------------------------
// Probes
// ---------------------------------------------------------------------------

async fn probe_binding(env: &Env) -> Check {
    let detail = env
        .hyperdrive("DB")
        .map(|h| format!("host={} port={}", h.host(), h.port()))
        .map_err(|e| e.to_string());
    ("hyperdrive binding resolves", detail)
}

async fn probe_simple_query(env: &Env) -> Check {
    let detail = async {
        let (client, transport) = connect_any(env).await?;
        let messages = client
            .simple_query("SELECT version()")
            .await
            .map_err(|e| format!("simple_query: {e}"))?;
        let version = messages
            .iter()
            .find_map(|m| match m {
                tokio_postgres::SimpleQueryMessage::Row(row) => row.get(0).map(str::to_owned),
                _ => None,
            })
            .unwrap_or_else(|| "<no row>".to_owned());
        Ok(format!("[{transport}] {version}"))
    }
    .await;
    ("simple_query (no prepared statement)", detail)
}

async fn probe_prepared_query(env: &Env) -> Check {
    let detail = async {
        let (client, _) = connect_any(env).await?;
        let rows = client
            .query("SELECT $1::text AS echo", &[&"noal"])
            .await
            .map_err(|e| format!("query: {e}"))?;
        let echo: String = rows
            .first()
            .map(|r| r.get("echo"))
            .ok_or_else(|| "no rows returned".to_owned())?;
        Ok(format!("echo={echo}"))
    }
    .await;
    ("query (prepared statement)", detail)
}

/// `worker::Fetch` resolves a `JsFuture`, which holds an `Rc` and is therefore
/// `!Send`. axum requires `Send` handler futures, so the JS-facing part is
/// wrapped. `SendWrapper` panics if it is touched from another thread, which
/// cannot happen: `wasm32-unknown-unknown` has a single thread.
async fn probe_outbound_fetch() -> Check {
    let detail = SendWrapper::new(async {
        let url = Url::parse("https://api.workos.com/sso/authorize")
            .map_err(|e| format!("parse url: {e}"))?;
        let response = Fetch::Url(url)
            .send()
            .await
            .map_err(|e| format!("fetch: {e}"))?;
        Ok(format!("status={}", response.status_code()))
    })
    .await;
    ("outbound HTTPS (WorkOS reachable)", detail)
}

/// Claim 5: real rows become HTML. This is the whole htmx story in one route.
async fn load_notes(env: &Env) -> Result<Vec<Note>, String> {
    let (client, _) = connect_any(env).await?;
    let rows = client
        .query("SELECT id, body FROM note ORDER BY id", &[])
        .await
        .map_err(|e| format!("select note: {e}"))?;
    Ok(rows
        .iter()
        .map(|row| Note {
            id: row.get("id"),
            body: row.get("body"),
        })
        .collect())
}

async fn probe_notes(env: &Env) -> Check {
    let detail = load_notes(env)
        .await
        .map(|notes| format!("{} row(s) loaded", notes.len()));
    ("SELECT from a real table", detail)
}

// ---------------------------------------------------------------------------
// Views
// ---------------------------------------------------------------------------

fn layout(title: &str, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                title { (title) }
                script src="https://unpkg.com/htmx.org@2.0.4" {}
                style { "body{font:15px/1.5 ui-monospace,monospace;max-width:60rem;margin:3rem auto;padding:0 1rem}
                         .ok{color:#087f5b} .fail{color:#c92a2a} li{margin:.5rem 0} code{word-break:break-all}" }
            }
            body { (body) }
        }
    }
}

fn render_checks(checks: &[Check]) -> Markup {
    html! {
        ul {
            @for (name, result) in checks {
                li {
                    @match result {
                        Ok(detail) => {
                            span .ok { "PASS" } " " (name) " — " code { (detail) }
                        }
                        Err(error) => {
                            span .fail { "FAIL" } " " (name) " — " code { (error) }
                        }
                    }
                }
            }
        }
    }
}

fn render_notes(notes: &[Note]) -> Markup {
    html! {
        ul {
            @for note in notes {
                li { "#" (note.id) " " (note.body) }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

async fn index() -> Html<String> {
    let page = layout(
        "noal spike",
        html! {
            h1 { "noal spike: tokio-postgres on Workers" }
            p { "Press the button. htmx swaps the fragment below with live probe results." }
            button hx-get="/probe" hx-target="#out" hx-swap="innerHTML" { "Run probes" }
            " "
            button hx-get="/rows" hx-target="#out" hx-swap="innerHTML" { "Load notes" }
            div #out {}
        },
    );
    Html(page.into_string())
}

/// The htmx fragment. It returns a bare `<ul>`, not a page.
async fn probe(State(state): State<AppState>) -> Html<String> {
    let checks = vec![
        probe_binding(&state.env).await,
        probe_simple_query(&state.env).await,
        probe_prepared_query(&state.env).await,
        probe_notes(&state.env).await,
        probe_outbound_fetch().await,
    ];
    Html(render_checks(&checks).into_string())
}

/// The htmx fragment that a real page would swap in.
async fn rows(State(state): State<AppState>) -> Html<String> {
    let markup = match load_notes(&state.env).await {
        Ok(notes) => render_notes(&notes),
        Err(error) => html! { p .fail { "FAIL " (error) } },
    };
    Html(markup.into_string())
}

fn router(env: Env) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/probe", get(probe))
        .route("/rows", get(rows))
        .with_state(AppState { env })
}

#[event(fetch)]
async fn fetch(
    req: HttpRequest,
    env: Env,
    _ctx: Context,
) -> worker::Result<axum::http::Response<axum::body::Body>> {
    console_error_panic_hook::set_once();
    Ok(router(env).call(req).await?)
}
