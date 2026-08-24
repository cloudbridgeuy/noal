//! Writing a saved window.
//!
//! The insert is the one place user-typed text reaches Postgres, so it is
//! parameterized end to end: `client.query` with bound values, never
//! `simple_query`, which would interpolate strings into SQL. That choice
//! makes Hyperdrive caching a deployment requirement for this table; see
//! `CLAUDE.md`.

use noal_core::window::Window;

/// Insert one window row.
///
/// # Errors
///
/// Returns the driver's error when the connection fails or Postgres refuses
/// the insert. The caller decides what a failure means — an ask whose answer
/// rendered still shows that answer, minus its URL.
pub async fn insert(client: &tokio_postgres::Client, window: &Window) -> Result<(), String> {
    const INSERT: &str = "insert into \"window\" \
                          (id, user_id, parent_id, request, sql, shape, template, name) \
                          values ($1, $2, $3, $4, $5, $6, $7, $8)";

    client
        .execute(
            INSERT,
            &[
                &window.id,
                &window.user_id,
                &window.parent_id,
                &window.request,
                &window.sql,
                &window.shape,
                &window.template,
                &window.name,
            ],
        )
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}
