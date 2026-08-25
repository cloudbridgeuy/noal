//! Reading and writing saved windows.
//!
//! Both statements here take values the request supplied — a user id on
//! every write, an id from the URL on every read — so both are parameterized
//! end to end: `client.query` with bound values, never `simple_query`, which
//! would interpolate strings into SQL. That choice makes Hyperdrive caching
//! a deployment requirement for this table; see `CLAUDE.md`.

use noal_core::window::Window;
use uuid::Uuid;

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

/// Read one window by id, scoped to its owner.
///
/// A row that does not exist and a row that belongs to someone else both
/// answer `None`: the caller cannot tell them apart, which is the point.
/// The stored shape comes back as the JSON it was written as; turning it
/// back into a plan is the caller's concern.
///
/// # Errors
///
/// Returns the driver's error when the connection fails or Postgres refuses
/// the read. The caller decides what a failure means.
pub async fn find(
    client: &tokio_postgres::Client,
    id: Uuid,
    user_id: &str,
) -> Result<Option<Window>, String> {
    const FIND: &str = "select id, user_id, parent_id, request, sql, shape, template, name, \
                        extract(epoch from created_at)::bigint as created_epoch \
                        from \"window\" where id = $1 and user_id = $2";

    let rows = client
        .query(FIND, &[&id, &user_id])
        .await
        .map_err(|error| error.to_string())?;

    let Some(row) = rows.first() else {
        return Ok(None);
    };

    Ok(Some(Window {
        id: row.get(0),
        user_id: row.get(1),
        parent_id: row.get(2),
        request: row.get(3),
        sql: row.get(4),
        shape: row.get(5),
        template: row.get(6),
        name: row.get(7),
        created_at: noal_core::clock::Timestamp::from_unix_seconds(row.get(8)),
    }))
}

/// Store one window's viewer-given name, or clear it with `None`.
///
/// Scoped on both sides — the window's id and its owner — so a name can
/// only ever land in a row the session belongs to. Parameterized like every
/// statement here: `name` is user input.
///
/// A scoped update that touches no row means no such window: an unknown id
/// and another viewer's id are indistinguishable here, exactly as they are
/// in [`find`], and `Err(NoSuch)` is what says so.
///
/// # Errors
///
/// Returns the driver's error when the connection fails or Postgres refuses
/// the update, and [`SetNameError::NoSuch`] when no row owned by this
/// viewer matched.
pub async fn set_name(
    client: &tokio_postgres::Client,
    id: Uuid,
    user_id: &str,
    name: Option<&str>,
) -> Result<(), SetNameError> {
    const SET_NAME: &str = "update \"window\" set name = $1 where id = $2 and user_id = $3";

    let updated = client
        .execute(SET_NAME, &[&name, &id, &user_id])
        .await
        .map_err(|error| SetNameError::Driver(error.to_string()))?;

    if updated == 0 {
        return Err(SetNameError::NoSuch);
    }
    Ok(())
}

/// Why a name could not be stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetNameError {
    /// The connection failed or Postgres refused the statement.
    Driver(String),
    /// No row owned by this viewer carries that id.
    NoSuch,
}
