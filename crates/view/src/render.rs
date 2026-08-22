//! Filling a model-written Tera template with rows.
//!
//! This is the only place noal runs a template it did not write. Tera is
//! strict — an undefined variable is an error, not an empty string — and HTML
//! autoescape is on, so a value containing `<` is shown, not interpreted.

use serde_json::Value;
use tera::{Context, ErrorKind, Tera};

/// Why a template could not be filled.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RenderError {
    /// The template does not parse.
    #[error("template syntax: {0}")]
    Syntax(String),
    /// The template parsed but failed while rendering, such as an undefined
    /// variable or a filter applied to the wrong type.
    #[error("template rendering: {0}")]
    Rendering(String),
    /// The rows could not become a template context.
    #[error("template context: {0}")]
    Context(String),
}

/// Render `template` with `rows` bound to the variable `rows`.
///
/// `rows` must be a JSON array, which is what `noal_core::ask::plan::wrap_sql`
/// makes Postgres return.
///
/// # Errors
///
/// Returns [`RenderError`] with Tera's report, which names the line and the
/// variable, so the shell can hand it back to the model.
pub fn fill(template: &str, rows: &Value) -> Result<String, RenderError> {
    let context = Context::from_serialize(&serde_json::json!({ "rows": rows }))
        .map_err(|error| RenderError::Context(error.to_string()))?;

    Tera::one_off(template, &context, true).map_err(|error| match error.kind() {
        ErrorKind::SyntaxError(_) => RenderError::Syntax(error.to_string()),
        _ => RenderError::Rendering(error.to_string()),
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{fill, RenderError};
    use serde_json::json;

    #[test]
    fn fills_a_loop_from_json_rows() {
        let rows = json!([{ "title": "a" }, { "title": "b" }]);
        let html = fill(
            "<ul>{% for r in rows %}<li>{{ r.title }}</li>{% endfor %}</ul>",
            &rows,
        )
        .unwrap();
        assert_eq!(html, "<ul><li>a</li><li>b</li></ul>");
    }

    #[test]
    fn fills_a_conditional_and_a_length() {
        let rows = json!([]);
        let html = fill(
            "{% if rows | length == 0 %}none{% else %}some{% endif %}",
            &rows,
        )
        .unwrap();
        assert_eq!(html, "none");
    }

    #[test]
    fn escapes_html_in_values() {
        let rows = json!([{ "title": "<b>x</b>" }]);
        let html = fill("{{ rows[0].title }}", &rows).unwrap();
        assert_eq!(html, "&lt;b&gt;x&lt;/b&gt;");
    }

    #[test]
    fn an_undefined_variable_is_a_rendering_error_that_names_it() {
        let rows = json!([{ "title": "a" }]);
        let error = fill("{{ rows[0].missing }}", &rows).unwrap_err();
        assert!(matches!(error, RenderError::Rendering(_)));
        assert!(error.to_string().contains("missing"));
    }

    #[test]
    fn a_malformed_template_is_a_syntax_error() {
        let rows = json!([]);
        let error = fill("{% for r in rows %}", &rows).unwrap_err();
        assert!(matches!(error, RenderError::Syntax(_)));
    }
}
