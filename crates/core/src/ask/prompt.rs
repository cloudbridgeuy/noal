//! The text the model is sent, built from values.
//!
//! Each builder takes everything it needs as arguments, including the
//! previous failed attempts, so the retry policy is a matter of what the
//! shell passes in and the prompt itself is testable by string comparison.

use super::plan::{Column, ColumnKind};
use super::CATALOG;

/// How many times a stage may run before noal gives up on the request.
///
/// One attempt: a refused stage ends the ask instead of trying again. The
/// builders already accept previous attempts so a later policy can raise this
/// without reshaping `plan_prompt` or `render_prompt`.
pub const MAX_ATTEMPTS: usize = 1;

/// A previous try at a stage: what the model produced and why it was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    /// The SQL or the template the model wrote.
    pub artifact: String,
    /// The error Postgres or Tera reported, verbatim.
    pub error: String,
}

/// The system preamble for the planning call.
pub const PLAN_PREAMBLE: &str = "\
You write one read-only PostgreSQL SELECT that answers a user's request over \
the schema you are given, and you describe the shape of its result. You return \
JSON matching the schema you are constrained to. Use only the tables and \
columns in the catalog. Alias every output column with a plain snake_case \
name. No semicolon, no comments, no DML.";

/// The system preamble for the rendering call.
pub const RENDER_PREAMBLE: &str = "\
You write a Tera template (Jinja2 syntax, Tera 2) that presents query results \
to a user. The rows are in a variable named `rows`, an array of objects with \
exactly the fields described in the shape. You never see the data; bind to \
the fields by name. Return only the template: plain HTML, no <script>, no \
<style> blocks longer than a few rules, no markdown fences, no explanation. \
Use semantic HTML. You may use htmx attributes but there are no endpoints to \
call yet. Choose the presentation that fits the request: a table for many \
uniform rows, cards or a list for few rich rows, headings and short summary \
text where they help. Never invent numbers in prose; compute them with Tera \
(`{{ rows | length }}`) or leave them out.";

/// Build the user message for the planning call.
#[must_use]
pub fn plan_prompt(request: &str, previous: &[Attempt]) -> String {
    let mut text = String::new();
    text.push_str("# Catalog\n\n");
    text.push_str(CATALOG);
    text.push_str("\n\n# Request\n\n");
    text.push_str(request.trim());
    text.push('\n');
    push_attempts(&mut text, "SQL", previous);
    text
}

/// Build the user message for the rendering call.
#[must_use]
pub fn render_prompt(request: &str, shape: &[Column], previous: &[Attempt]) -> String {
    let mut text = String::new();
    text.push_str("# Request\n\n");
    text.push_str(request.trim());
    text.push_str("\n\n# Shape of `rows`\n\n");
    for column in shape {
        text.push_str(&describe_column(column));
    }
    push_attempts(&mut text, "template", previous);
    text
}

/// One line per column, plus one indented line per nested field.
fn describe_column(column: &Column) -> String {
    let mut line = format!(
        "- `{}` ({}): {}\n",
        column.name,
        kind_name(column.kind),
        column.description.trim()
    );
    for field in &column.fields {
        line.push_str(&format!(
            "  - `{}` ({})\n",
            field.name,
            kind_name(field.kind)
        ));
    }
    line
}

/// The name the model sees for a kind.
const fn kind_name(kind: ColumnKind) -> &'static str {
    match kind {
        ColumnKind::Text => "text",
        ColumnKind::Integer => "integer",
        ColumnKind::Number => "number",
        ColumnKind::Boolean => "boolean",
        ColumnKind::Timestamp => "timestamp",
        ColumnKind::TextList => "list of text",
        ColumnKind::ObjectList => "list of objects",
    }
}

/// Append the failed attempts, if any, so the model can correct itself.
fn push_attempts(text: &mut String, what: &str, previous: &[Attempt]) {
    if previous.is_empty() {
        return;
    }
    text.push_str("\n# Previous attempts\n\n");
    text.push_str(&format!(
        "Each {what} below was rejected with the error shown. Fix the cause; do not repeat it.\n"
    ));
    for (index, attempt) in previous.iter().enumerate() {
        text.push_str(&format!(
            "\n## Attempt {}\n\n```\n{}\n```\n\nError:\n\n```\n{}\n```\n",
            index + 1,
            attempt.artifact.trim(),
            attempt.error.trim()
        ));
    }
}

/// Remove a surrounding markdown code fence, if the model added one.
///
/// Models wrap templates in a fenced code block against instructions often
/// enough that refusing the answer would waste a retry. Only an outermost
/// fence is removed; inner content is untouched.
#[must_use]
pub fn strip_fences(text: &str) -> String {
    let trimmed = text.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed.to_owned();
    };
    let Some(body) = rest.strip_suffix("```") else {
        return trimmed.to_owned();
    };
    // Drop the language tag on the opening fence line.
    let body = match body.split_once('\n') {
        Some((tag, after)) if !tag.contains(' ') && !tag.contains('<') => after,
        _ => body,
    };
    body.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::{plan_prompt, render_prompt, strip_fences, Attempt};
    use crate::ask::plan::{Column, ColumnKind, NestedField};

    #[test]
    fn plan_prompt_carries_the_catalog_and_the_request() {
        let text = plan_prompt("  open tasks ", &[]);
        assert!(text.contains("## ticket"));
        assert!(text.contains("# Request\n\nopen tasks\n"));
        assert!(!text.contains("Previous attempts"));
    }

    #[test]
    fn plan_prompt_appends_failed_attempts_with_their_errors() {
        let previous = [Attempt {
            artifact: "select nope from ticket".into(),
            error: "column \"nope\" does not exist".into(),
        }];
        let text = plan_prompt("open tasks", &previous);
        assert!(text.contains("# Previous attempts"));
        assert!(text.contains("## Attempt 1"));
        assert!(text.contains("select nope from ticket"));
        assert!(text.contains("column \"nope\" does not exist"));
    }

    #[test]
    fn render_prompt_describes_every_column_and_nested_field() {
        let shape = vec![
            Column {
                name: "title".into(),
                kind: ColumnKind::Text,
                description: "the ticket title".into(),
                fields: vec![],
            },
            Column {
                name: "comments".into(),
                kind: ColumnKind::ObjectList,
                description: "its comments".into(),
                fields: vec![NestedField {
                    name: "body".into(),
                    kind: ColumnKind::Text,
                }],
            },
        ];
        let text = render_prompt("show tickets", &shape, &[]);
        assert!(text.contains("- `title` (text): the ticket title"));
        assert!(text.contains("- `comments` (list of objects): its comments"));
        assert!(text.contains("  - `body` (text)"));
    }

    #[test]
    fn render_prompt_appends_failed_attempts_with_their_errors() {
        let previous = [Attempt {
            artifact: "{{ rows.0.missing }}".into(),
            error: "variable `missing` not found".into(),
        }];
        let text = render_prompt("show tickets", &[], &previous);
        assert!(text.contains("# Previous attempts"));
        assert!(text.contains("{{ rows.0.missing }}"));
        assert!(text.contains("variable `missing` not found"));
    }

    #[test]
    fn strip_fences_removes_an_outer_fence_with_a_language_tag() {
        let text = "```html\n<ul></ul>\n```";
        assert_eq!(strip_fences(text), "<ul></ul>");
    }

    #[test]
    fn strip_fences_leaves_unfenced_text_alone() {
        assert_eq!(strip_fences("  <p>hi</p>\n"), "<p>hi</p>");
    }

    #[test]
    fn strip_fences_keeps_a_fence_that_does_not_close() {
        assert_eq!(strip_fences("```html\n<p>"), "```html\n<p>");
    }
}
