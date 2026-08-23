//! What the planner returns: a query and the shape of its result.
//!
//! The model fills this in through structured output, so the JSON Schema
//! derived here is what the model is constrained to. The shape is deliberately
//! flat — one level of nesting, for comments — because the provider's
//! structured-output support rejects recursive schemas.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A query and a description of every column it returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Plan {
    /// A single read-only `SELECT` over the catalog tables, no trailing
    /// semicolon.
    pub sql: String,
    /// One entry per output column, in order, with the alias used in `sql`.
    pub shape: Vec<Column>,
}

/// One output column of a plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Column {
    /// The alias exactly as it appears in the SQL.
    pub name: String,
    /// The value type the template will see.
    pub kind: ColumnKind,
    /// What the column means, for the renderer.
    pub description: String,
    /// For `object_list` only: the fields of each object. Empty otherwise.
    pub fields: Vec<NestedField>,
}

/// A field inside an `object_list` column.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NestedField {
    /// The key inside each object.
    pub name: String,
    /// The value type. Never `object_list`; nesting stops here.
    pub kind: ColumnKind,
}

/// The value types a column can carry into the template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ColumnKind {
    /// A string.
    Text,
    /// A whole number.
    Integer,
    /// A number with a fractional part.
    Number,
    /// `true` or `false`.
    Boolean,
    /// An ISO-8601 timestamp string.
    Timestamp,
    /// A list of strings, such as `tags`.
    TextList,
    /// A list of objects, such as a ticket's comments.
    ObjectList,
}

impl Plan {
    /// True when both plans return the same columns, so a template written
    /// for one fits the other.
    ///
    /// Compares name, kind, and nested fields only. `description` is prose
    /// for the model that wrote the plan; a reworded description binds to
    /// the same template field, so it is not part of what "the same shape"
    /// means. Ignoring it lets a retry keep a good template more often, at
    /// no risk: a real shape change still differs by name, kind, or fields.
    #[must_use]
    pub fn same_shape(&self, other: &Self) -> bool {
        self.shape.len() == other.shape.len()
            && self
                .shape
                .iter()
                .zip(&other.shape)
                .all(|(a, b)| a.name == b.name && a.kind == b.kind && a.fields == b.fields)
    }
}

/// Wrap the model's `SELECT` so Postgres returns one JSON text value.
///
/// The outer query turns every row into a JSON object and the whole result
/// into one array, so the shell reads a single text column with
/// `simple_query` and never maps Postgres types by hand. `coalesce` makes an
/// empty result `[]` instead of SQL `NULL`.
#[must_use]
pub fn wrap_sql(sql: &str) -> String {
    let inner = sql.trim().trim_end_matches(';').trim();
    format!("select coalesce(json_agg(t), '[]')::text as rows from ({inner}) t")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{wrap_sql, Column, ColumnKind, NestedField, Plan};

    fn text(name: &str) -> Column {
        Column {
            name: name.to_owned(),
            kind: ColumnKind::Text,
            description: String::new(),
            fields: Vec::new(),
        }
    }

    #[test]
    fn wrap_sql_nests_the_query_and_strips_a_trailing_semicolon() {
        let wrapped = wrap_sql("select id from ticket;  ");
        assert_eq!(
            wrapped,
            "select coalesce(json_agg(t), '[]')::text as rows from (select id from ticket) t"
        );
    }

    #[test]
    fn plans_with_equal_columns_share_a_shape() {
        let a = Plan {
            sql: "select 1".into(),
            shape: vec![text("id")],
        };
        let b = Plan {
            sql: "select 2".into(),
            shape: vec![text("id")],
        };
        assert!(a.same_shape(&b));
    }

    #[test]
    fn plans_with_different_columns_do_not_share_a_shape() {
        let a = Plan {
            sql: "select 1".into(),
            shape: vec![text("id")],
        };
        let b = Plan {
            sql: "select 1".into(),
            shape: vec![text("title")],
        };
        assert!(!a.same_shape(&b));
    }

    #[test]
    fn a_reworded_description_does_not_change_the_shape() {
        let mut a = text("id");
        a.description = "the ticket's id".into();
        let mut b = text("id");
        b.description = "unique identifier of the ticket".into();
        let plan_a = Plan {
            sql: "select 1".into(),
            shape: vec![a],
        };
        let plan_b = Plan {
            sql: "select 2".into(),
            shape: vec![b],
        };
        assert!(plan_a.same_shape(&plan_b));
    }

    #[test]
    fn column_kind_serializes_in_snake_case() {
        let json = serde_json::to_string(&ColumnKind::ObjectList).unwrap();
        assert_eq!(json, "\"object_list\"");
    }

    #[test]
    fn a_plan_round_trips_through_json() {
        let plan = Plan {
            sql: "select id, comments from ticket".into(),
            shape: vec![
                text("id"),
                Column {
                    name: "comments".into(),
                    kind: ColumnKind::ObjectList,
                    description: "the ticket's comments".into(),
                    fields: vec![NestedField {
                        name: "body".into(),
                        kind: ColumnKind::Text,
                    }],
                },
            ],
        };
        let json = serde_json::to_string(&plan).unwrap();
        let back: Plan = serde_json::from_str(&json).unwrap();
        assert_eq!(back, plan);
    }

    #[test]
    fn the_schema_has_no_recursion_and_names_every_field() {
        let schema = serde_json::to_value(schemars::schema_for!(Plan)).unwrap();
        let text = schema.to_string();
        assert!(text.contains("\"sql\""));
        assert!(text.contains("\"shape\""));
        assert!(text.contains("object_list"));
        // NestedField must not reference Column, or the schema would recurse.
        assert!(!text.contains("\"fields\":{\"$ref\":\"#/$defs/Column\""));
    }
}
