//! Saved windows: what one successful ask keeps, and how they nest.
//!
//! A window is the stored twin of an ask — the request, the query, and the
//! template that together recreate the rendered view. This module owns the
//! record type and the one rule about shape: which rows hang under which in
//! the palette's tree. Both are pure; reading rows from Postgres and writing
//! them back are the shell's job.

use std::collections::HashMap;

use uuid::Uuid;

use crate::ask::plan::Plan;

/// One saved window: everything that recreates its view.
///
/// `created_at` is assigned by the database on insert and carried by
/// [`Window`] once read back: the window page shows the window's age, so
/// the value has a reader and must travel with the row. Nothing in noal
/// writes it.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    /// The window's identity. Drawn by the shell, so a failed insert wastes
    /// nothing but randomness and a row never waits on the database for it.
    pub id: Uuid,
    /// The WorkOS user the window belongs to. Every read is scoped by this.
    pub user_id: String,
    /// The window this one hangs under, when it has one.
    pub parent_id: Option<Uuid>,
    /// What the user typed to produce the view.
    pub request: String,
    /// The read-only `SELECT`, as the planner wrote it (unwrapped).
    pub sql: String,
    /// The plan's description of the query's output columns.
    pub shape: serde_json::Value,
    /// The Tera template the model wrote for this shape.
    pub template: String,
    /// The name the viewer gave the window. Nothing writes one yet.
    pub name: Option<String>,
    /// When the ask that produced this window succeeded.
    ///
    /// Populated only by the shell's read, never by [`Self::answered`]:
    /// the database assigns it, and a window that has not been written
    /// has no age yet.
    pub created_at: crate::clock::Timestamp,
}

impl Window {
    /// Build the window one answered ask produces.
    ///
    /// An answer is only savable when the pipeline actually holds a plan and
    /// a template; anything else returns `None`, and the caller must treat
    /// that exactly like a failed save rather than inventing a half row.
    /// `parent_id` starts empty — windows attach to nothing until something
    /// decides they belong somewhere. `created_at` comes from the caller
    /// because only the moment of the insert knows it; the shell reads the
    /// clock once, at the edge, like every other time-dependent input.
    #[must_use]
    pub fn answered(
        id: Uuid,
        user_id: &str,
        request: &str,
        plan: Option<&Plan>,
        template: Option<&str>,
        created_at: crate::clock::Timestamp,
    ) -> Option<Self> {
        let plan = plan?;
        let template = template?;

        Some(Self {
            id,
            user_id: user_id.to_owned(),
            parent_id: None,
            request: request.to_owned(),
            sql: plan.sql.clone(),
            shape: serde_json::to_value(plan.shape.clone()).ok()?,
            template: template.to_owned(),
            name: None,
            created_at,
        })
    }
}

/// One row of the tree as the palette shows it: the label data plus where the
/// row hangs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The window's id.
    pub id: Uuid,
    /// The window this one hangs under, when it names one.
    pub parent_id: Option<Uuid>,
    /// The request that produced the window.
    pub request: String,
    /// The viewer-given name, when there is one.
    pub name: Option<String>,
}

/// One node of the nested tree: an entry and everything under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// The window this node shows.
    pub entry: Entry,
    /// The windows whose parent is this one, in creation order.
    pub children: Vec<Node>,
}

/// Nest flat entries into a tree.
///
/// The result is the root's children — there is no synthetic root; Home is
/// the renderer's own row. Siblings appear in creation order, which is the
/// order of the slice. The function is total: a row whose parent is absent
/// from the input, or appears after it, or is the row itself, hangs at the
/// root rather than being dropped or refused. Because a row only ever
/// attaches to a parent *earlier* in the slice, the nesting cannot cycle.
#[must_use]
pub fn tree(entries: &[Entry]) -> Vec<Node> {
    // Where each id sits in the slice. A duplicate id keeps the first slot;
    // later duplicates then find their parent "already used" and sit at the
    // root, which keeps this total even over nonsense input.
    let mut slots: HashMap<Uuid, usize> = HashMap::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        slots.entry(entry.id).or_insert(index);
    }

    let mut children: Vec<Vec<usize>> = vec![Vec::new(); entries.len()];
    let mut roots: Vec<usize> = Vec::new();

    for (index, entry) in entries.iter().enumerate() {
        match entry
            .parent_id
            .and_then(|parent| slots.get(&parent).copied())
        {
            // Forward pass: only an earlier slot can hold this row's parent.
            Some(parent) if parent < index => children[parent].push(index),
            _ => roots.push(index),
        }
    }

    roots
        .into_iter()
        .map(|root| assemble(root, entries, &children))
        .collect()
}

/// Build the node at `index` with everything nested under it.
fn assemble(index: usize, entries: &[Entry], children: &[Vec<usize>]) -> Node {
    Node {
        entry: entries[index].clone(),
        children: children[index]
            .iter()
            .map(|&child| assemble(child, entries, children))
            .collect(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{tree, Entry, Window};
    use crate::ask::plan::{Column, ColumnKind, Plan};
    use uuid::Uuid;

    fn id(n: u8) -> Uuid {
        Uuid::from_bytes([0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, n])
    }

    fn entry(n: u8, parent: Option<Uuid>) -> Entry {
        Entry {
            id: id(n),
            parent_id: parent,
            request: format!("window {n}"),
            name: None,
        }
    }

    #[test]
    fn empty_input_yields_no_tree() {
        assert_eq!(tree(&[]), Vec::new());
    }

    #[test]
    fn siblings_hold_creation_order_at_the_root() {
        let roots = tree(&[entry(1, None), entry(2, None), entry(3, None)]);
        let order: Vec<Uuid> = roots.iter().map(|node| node.entry.id).collect();
        assert_eq!(order, vec![id(1), id(2), id(3)]);
    }

    #[test]
    fn children_nest_to_any_depth() {
        // 1 ← 2 ← 3, each hanging under the previous.
        let roots = tree(&[entry(1, None), entry(2, Some(id(1))), entry(3, Some(id(2)))]);

        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].entry.id, id(1));
        assert_eq!(roots[0].children.len(), 1);
        assert_eq!(roots[0].children[0].entry.id, id(2));
        assert_eq!(roots[0].children[0].children[0].entry.id, id(3));
    }

    #[test]
    fn siblings_under_one_parent_keep_their_creation_order() {
        let roots = tree(&[
            entry(1, None),
            entry(4, Some(id(1))),
            entry(2, Some(id(1))),
            entry(3, Some(id(1))),
        ]);

        let order: Vec<Uuid> = roots[0].children.iter().map(|node| node.entry.id).collect();
        assert_eq!(order, vec![id(4), id(2), id(3)]);
    }

    #[test]
    fn a_row_whose_parent_is_absent_hangs_at_the_root() {
        let roots = tree(&[entry(1, Some(id(99))), entry(2, None)]);
        let order: Vec<Uuid> = roots.iter().map(|node| node.entry.id).collect();
        assert_eq!(order, vec![id(1), id(2)]);
    }

    #[test]
    fn a_row_whose_parent_appears_later_hangs_at_the_root() {
        // The forward pass only looks backwards, so a parent the row was
        // written before cannot catch it.
        let roots = tree(&[entry(1, Some(id(2))), entry(2, None)]);
        let order: Vec<Uuid> = roots.iter().map(|node| node.entry.id).collect();
        assert_eq!(order, vec![id(1), id(2)]);
    }

    #[test]
    fn a_row_that_names_itself_as_parent_hangs_at_the_root() {
        let roots = tree(&[entry(1, Some(id(1)))]);
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].entry.id, id(1));
        assert!(roots[0].children.is_empty());
    }

    #[test]
    fn two_parents_interleave_without_losing_a_row() {
        let roots = tree(&[
            entry(1, None),
            entry(2, None),
            entry(3, Some(id(1))),
            entry(4, Some(id(2))),
            entry(5, Some(id(1))),
        ]);

        assert_eq!(
            roots.iter().map(|n| n.entry.id).collect::<Vec<_>>(),
            vec![id(1), id(2)]
        );
        assert_eq!(
            roots[0]
                .children
                .iter()
                .map(|n| n.entry.id)
                .collect::<Vec<_>>(),
            vec![id(3), id(5)]
        );
        assert_eq!(
            roots[1]
                .children
                .iter()
                .map(|n| n.entry.id)
                .collect::<Vec<_>>(),
            vec![id(4)]
        );
    }

    fn plan() -> Plan {
        Plan {
            sql: "select id from ticket".into(),
            shape: vec![Column {
                name: "id".into(),
                kind: ColumnKind::Integer,
                description: String::new(),
                fields: Vec::new(),
            }],
        }
    }

    #[test]
    fn an_answered_ask_becomes_a_savable_window() {
        let window = Window::answered(
            id(9),
            "user_01",
            "open tasks",
            Some(&plan()),
            Some("<p>{{ rows | length }}</p>"),
            crate::clock::Timestamp::from_unix_seconds(0),
        )
        .unwrap();

        assert_eq!(window.id, id(9));
        assert_eq!(window.user_id, "user_01");
        assert_eq!(window.parent_id, None);
        assert_eq!(window.request, "open tasks");
        assert_eq!(window.sql, "select id from ticket");
        assert_eq!(window.name, None);
        // The shape survives as the JSON the jsonb column stores.
        let shape = serde_json::to_value(&window.shape).unwrap();
        assert_eq!(shape[0]["name"], "id");
    }

    #[test]
    fn an_answer_without_its_artifacts_cannot_be_saved() {
        let plan = plan();
        let at = crate::clock::Timestamp::from_unix_seconds(0);
        assert!(Window::answered(id(9), "user_01", "ask", Some(&plan), None, at).is_none());
        assert!(Window::answered(id(9), "user_01", "ask", None, Some("<p></p>"), at).is_none());
        assert!(Window::answered(id(9), "user_01", "ask", None, None, at).is_none());
    }
}
