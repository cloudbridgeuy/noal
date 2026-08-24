//! The saved-window tree a page's chrome shows.
//!
//! One read serves every page: the palette's Windows tab is the same on Home,
//! on an ask, and on a window page. A tree that cannot be read is not an
//! empty tree — an empty tree claims the viewer has no windows, which would
//! be a lie — so every failure here collapses into
//! [`noal_view::windows::Windows::Unavailable`] and the page says so.

use noal_core::window::{tree, Entry};
use noal_view::windows::Windows;

use crate::state::AppState;

/// The columns the tree needs, and no more: identity, parentage, the label's
/// two sources, and the creation time siblings are ordered by.
const TREE_SELECT: &str = "select id, parent_id, request, name, created_at \
                           from \"window\" where user_id = $1 order by created_at, id";

/// Read one viewer's window tree for the chrome.
///
/// This never fails upward: a database that cannot be reached or refuses the
/// read becomes [`Windows::Unavailable`], because a page that answered
/// everything else correctly should not be thrown away over its sidebar.
/// Where the viewer currently is does not belong to the tree data; callers
/// pair the result with a [`noal_view::windows::Current`] when rendering.
pub async fn build(state: &AppState, user_id: &str) -> Windows {
    let client = match state.database().await {
        Ok(client) => client,
        Err(failure) => {
            worker::console_error!("window tree unreadable: {}", failure.detail());
            return Windows::Unavailable;
        }
    };

    let rows = match client.query(TREE_SELECT, &[&user_id]).await {
        Ok(rows) => rows,
        Err(error) => {
            worker::console_error!("window tree unreadable: {error}");
            return Windows::Unavailable;
        }
    };

    // The name column is read through even though nothing writes it yet, so
    // naming the windows later changes nothing here. An unnamed window reads
    // back as exactly the None the entry wants.
    let entries: Vec<Entry> = rows
        .iter()
        .map(|row| Entry {
            id: row.get(0),
            parent_id: row.get(1),
            request: row.get(2),
            name: row.get(3),
        })
        .collect();

    Windows::Tree(tree(&entries))
}
