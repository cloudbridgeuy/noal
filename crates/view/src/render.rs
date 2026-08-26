//! Filling a model-written Tera template with rows.
//!
//! This is the only place noal runs a template it did not write. Tera is
//! strict — an undefined variable is an error, not an empty string — and HTML
//! autoescape is on, so a value containing `<` is shown, not interpreted.

use noal_core::ask::prompt::RENDER_PREAMBLE;
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

/// Every CSS class the stylesheet offers a filled answer, as text for a
/// template-writing model call.
///
/// The list is derived from the stylesheet in `layout::STYLE`; a class
/// outside it carries no styling. Machinery-only names with no use outside
/// the page chrome (`htmx-request`, `htmx-indicator`, `sign-out`, the
/// palette's `tabs` and `active` markers, `window-rename-open`) are
/// deliberately absent, as are every id.
pub const CSS_CLASS_GUIDE: &str = "# Stylesheet classes\n\
\n\
Present your answer using these class names and nothing else: a class name \
not on this list has no styles behind it. Your output is placed inside a \
section already framed by `.card`, under a heading repeating the user's \
request.\n\
\n\
Buttons: `.btn` is an outlined inline button; appending `.btn-primary` makes \
it solid and inverted against the theme; appending `.btn-ghost` removes its \
border and mutes its label.\n\
\n\
Field: `.input` is a full-width bordered input.\n\
\n\
Frame: `.card` draws a bordered, rounded container — the section your output \
sits in wears one, so reach for it again only around inner parts worth \
boxing.\n\
\n\
Toast: `.toast` frames a short notice and pairs with `.card` and `.flex`; the \
notice stack itself is laid out by `.toast-stack`, and a dismiss control is a \
button wearing `.toast-dismiss`, `.btn`, and `.btn-ghost`.\n\
\n\
Chrome pieces: `.tree-row` is a compact row link that darkens on hover; \
`.tab-active` marks the active tab in pale blue; `.windows-unavailable` is \
error-red text; `.viewer-email` prints an address in the muted color.\n\
\n\
Layout: `.flex` lays children out in a row; `.gap-sm` and `.gap-md` space them \
half a rem apart and one rem apart; `.mt-1` adds a quarter-rem margin above; \
`.border-b` draws a bottom hairline.\n\
\n\
Text: `.muted` colors text gray; `.text-sm` sets it smaller; `.saved-date` \
combines both for timestamp lines; `.sr-only` hides content visually while \
screen readers still announce it.\n\
\n\
No class needed: tables collapse their borders and give every td and th cell \
a hairline border and padding, and links take on the color of the text around \
them.";

/// The system preamble for a call asking the model for a template: the core
/// rendering rules from `noal_core`, then [`CSS_CLASS_GUIDE`], so the answer
/// can draw on the stylesheet instead of inline styles alone.
///
/// Every template-writing call must pass this, or model output silently loses
/// access to the shared design system.
#[must_use]
pub fn template_preamble() -> String {
    format!("{RENDER_PREAMBLE}\n\n{CSS_CLASS_GUIDE}")
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
    use super::{fill, template_preamble, RenderError, CSS_CLASS_GUIDE};
    use noal_core::ask::prompt::RENDER_PREAMBLE;
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

    // The class guide must stay in step with the stylesheet it describes, so
    // its contents are checked against `layout::STYLE` mechanically rather
    // than trusted.

    /// Every `.name` token in a stylesheet or the guide, deduplicated.
    ///
    /// A token counts as a class name only when the dot is followed by a
    /// lowercase letter, which skips decimal values such as `.5rem`.
    fn class_tokens(text: &str) -> Vec<String> {
        let bytes = text.as_bytes();
        let mut tokens = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            if bytes[at] == b'.' && bytes.get(at + 1).is_some_and(u8::is_ascii_lowercase) {
                let start = at + 1;
                let mut end = start;
                while end < bytes.len()
                    && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'-')
                {
                    end += 1;
                }
                tokens.push(String::from_utf8_lossy(&bytes[start..end]).into_owned());
                at = end;
            } else {
                at += 1;
            }
        }
        tokens.sort_unstable();
        tokens.dedup();
        tokens
    }

    #[test]
    fn the_class_guide_names_every_styled_class_the_stylesheet_defines() {
        // Machinery-only classes whose rules exist for the page chrome or
        // htmx itself are the only names a template writer never benefits
        // from; every other class in the sheet must be offered by the guide.
        let chrome_only = [
            "htmx-indicator",     // request-state toggling machinery
            "htmx-request",       // added and removed by htmx during a request
            "sign-out",           // display:contents wrapper of the header form
            "tabs",               // styled only inside #palette's tab row
            "active",             // styled only under #palette .tabs button
            "window-rename-open", // reset style for the palette rename opener
        ];
        for name in class_tokens(crate::layout::STYLE) {
            if chrome_only.contains(&name.as_str()) {
                continue;
            }
            assert!(
                class_tokens(CSS_CLASS_GUIDE).contains(&name),
                "the stylesheet defines .{name}, but the guide does not offer it"
            );
        }
    }

    #[test]
    fn the_class_guide_offers_nothing_the_stylesheet_lacks() {
        // `toast-dismiss` styles nothing: it is the hook the overlay script
        // binds toast dismissal to, documented because a dismiss control
        // needs it to work, so it is named alongside the classes that do.
        let hooks = ["toast-dismiss"];
        for name in class_tokens(CSS_CLASS_GUIDE) {
            assert!(
                class_tokens(crate::layout::STYLE).contains(&name)
                    || hooks.contains(&name.as_str()),
                "the guide offers .{name}, but the stylesheet has no such class"
            );
        }
    }

    #[test]
    fn the_class_guide_keeps_chrome_machinery_out_of_model_reach() {
        for name in ["htmx-indicator", "sign-out", "window-rename-open"] {
            assert!(
                !class_tokens(CSS_CLASS_GUIDE).contains(&name.to_owned()),
                "the guide offers .{name}, though templates have no use for it"
            );
        }
    }

    #[test]
    fn the_class_guide_pins_the_key_names() {
        let key = [
            ".btn",
            ".btn-primary",
            ".btn-ghost",
            ".input",
            ".card",
            ".toast",
            ".toast-dismiss",
            ".tab-active",
            ".tree-row",
            ".flex",
            ".gap-sm",
            ".gap-md",
            ".mt-1",
            ".border-b",
            ".muted",
            ".text-sm",
            ".sr-only",
            ".saved-date",
        ];
        for name in key {
            assert!(CSS_CLASS_GUIDE.contains(name), "the guide omits {name}");
        }
    }

    #[test]
    fn the_template_preamble_carries_the_render_rules_then_the_styles() {
        let preamble = template_preamble();
        // The core rules lead so the security constraints keep their weight,
        // with the vocabulary appended whole.
        assert!(preamble.starts_with(RENDER_PREAMBLE));
        let rules_at = preamble.find(RENDER_PREAMBLE).unwrap();
        let styles_at = preamble.find("# Stylesheet classes").unwrap();
        assert!(rules_at < styles_at);
        // The guide is included complete, sentence and all.
        assert!(preamble.ends_with("the text around them."));
        assert!(preamble.contains("using these class names"));
    }

    #[test]
    fn filling_still_runs_a_template_that_uses_a_guide_class() {
        // The vocabulary reaches the renderer untouched: no preprocessing
        // stands between fill() and a template that names `.muted`.
        let html = super::fill(
            "<p class=\"muted\">{{ rows[0].n }}</p>",
            &serde_json::json!([{ "n": 1 }]),
        )
        .unwrap();
        assert_eq!(html, "<p class=\"muted\">1</p>");
    }
}
