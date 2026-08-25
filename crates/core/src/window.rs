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
/// `created_at` is deliberately absent: it is assigned by the database on
/// insert and only ever used for ordering there, so no code needs to carry a
/// value it cannot set.
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
}

impl Window {
    /// Build the window one answered ask produces.
    ///
    /// An answer is only savable when the pipeline actually holds a plan and
    /// a template; anything else returns `None`, and the caller must treat
    /// that exactly like a failed save rather than inventing a half row.
    /// `parent_id` starts empty — windows attach to nothing until something
    /// decides they belong somewhere.
    #[must_use]
    pub fn answered(
        id: Uuid,
        user_id: &str,
        request: &str,
        plan: Option<&Plan>,
        template: Option<&str>,
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

/// The most characters a stored name may carry.
///
/// A longer offering is refused whole. Nothing ever truncates: a silent cut
/// would store a name the viewer did not choose.
pub const NAME_LIMIT: usize = 200;

/// A viewer-given window name, as it may be stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Name {
    /// The viewer cleared the name; the row falls back to the cut request.
    Clear,
    /// A trimmed name of at most [`NAME_LIMIT`] characters.
    Set(String),
}

/// Normalization refused the offering outright.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooLong;

/// Turn what the viewer typed into what may be stored as a name.
///
/// Surrounding whitespace is trimmed away. What remains empty or blank
/// clears the name. Anything longer than [`NAME_LIMIT`] characters is
/// refused whole, never truncated. These are facts about what the column
/// may hold; how the value reaches here and what a refusal shows are the
/// shell's concerns.
pub fn normalize_name(input: &str) -> Result<Name, TooLong> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(Name::Clear);
    }
    if trimmed.chars().count() > NAME_LIMIT {
        return Err(TooLong);
    }
    Ok(Name::Set(trimmed.to_owned()))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{normalize_name, tree, Entry, Name, TooLong, Window, NAME_LIMIT};
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
        assert!(Window::answered(id(9), "user_01", "ask", Some(&plan), None).is_none());
        assert!(Window::answered(id(9), "user_01", "ask", None, Some("<p></p>")).is_none());
        assert!(Window::answered(id(9), "user_01", "ask", None, None).is_none());
    }

    #[test]
    fn a_name_is_trimmed_of_surrounding_whitespace() {
        assert_eq!(
            normalize_name("  Weekly report\t\n"),
            Ok(Name::Set("Weekly report".to_owned()))
        );
    }

    #[test]
    fn an_empty_or_blank_name_clears_the_stored_one() {
        assert_eq!(normalize_name(""), Ok(Name::Clear));
        assert_eq!(normalize_name("   "), Ok(Name::Clear));
        // Interior whitespace stays; only the edges are the viewer's accident.
        assert_eq!(normalize_name(" a b "), Ok(Name::Set("a b".to_owned())));
    }

    #[test]
    fn a_name_of_exactly_two_hundred_characters_is_kept() {
        let name = "x".repeat(NAME_LIMIT);
        assert_eq!(normalize_name(&name), Ok(Name::Set(name)));
    }

    #[test]
    fn a_name_longer_than_two_hundred_characters_is_refused_whole() {
        let name = "x".repeat(NAME_LIMIT + 1);
        assert_eq!(normalize_name(&name), Err(TooLong));
        // Multi-byte characters count per character, not per byte.
        let wide = "é".repeat(NAME_LIMIT + 1);
        assert_eq!(normalize_name(&wide), Err(TooLong));
    }

    #[test]
    fn a_normalized_name_round_trips_through_storage() {
        let Ok(Name::Set(stored)) = normalize_name("  Weekly report ") else {
            panic!("a fitting name is kept");
        };
        // What comes back from the column is what normalization produced,
        // and feeding it through again changes nothing.
        assert_eq!(normalize_name(&stored), Ok(Name::Set(stored.clone())));
        assert_eq!(stored, "Weekly report");
    }
}
