//! What rendered output may not carry, checked on the exact bytes that ship.
//!
//! The check runs on the final HTML, after Tera has filled the template,
//! because `{{ row.body | safe }}` can carry a whole link out of the
//! database past any scan of the template source. It matches tokens, not a
//! parsed document: a query result whose own text contains `href=` can trip
//! it, and the cost of such a false positive is one retry, never a wrong
//! page.

/// Elements whose only effect is to fetch, frame, submit, or redirect.
const FORBIDDEN_ELEMENTS: [&str; 8] = [
    "form", "iframe", "object", "embed", "base", "link", "script", "style",
];

/// Attributes whose effect is to fetch a URL or navigate to one, including
/// `<video poster>`'s silent fetch and `<a ping>`'s click-time navigation.
const FORBIDDEN_ATTRIBUTES: [&str; 7] = [
    "href",
    "ping",
    "poster",
    "src",
    "srcset",
    "action",
    "formaction",
];

/// Report the first forbidden token in rendered output.
///
/// The returned string names the token, so the model can be told what to
/// fix; `None` means the HTML may ship. Scanning is deliberately a token
/// match rather than a parse: a false positive costs one retry, a false
/// negative would ship a link that leads nowhere.
///
/// The scan runs in three phases over the whole input — CSS tokens first,
/// then elements, then attributes — so when tokens of different kinds are
/// present, the earlier phase's kind is reported even if another kind
/// appears first in document order. Every outcome refuses regardless; only
/// the reported name varies.
#[must_use]
pub fn forbidden_token(html: &str) -> Option<&'static str> {
    let lower = html.to_ascii_lowercase();
    css_tokens(&lower)
        .or_else(|| element_token(&lower))
        .or_else(|| attribute_token(&lower))
}

/// `url(` and `@import` are banned anywhere, including inside inline styles.
fn css_tokens(lower: &str) -> Option<&'static str> {
    if lower.contains("url(") {
        Some("url(")
    } else if lower.contains("@import") {
        Some("@import")
    } else {
        None
    }
}

/// The first `<name` whose tag names a forbidden element, matched whole.
fn element_token(lower: &str) -> Option<&'static str> {
    let bytes = lower.as_bytes();
    let mut from = 0;
    while let Some(offset) = lower[from..].find('<') {
        let mut start = from + offset + 1;
        if start == bytes.len() {
            break; // a trailing '<' with nothing after it names no element
        }
        if bytes[start] == b'/' {
            start += 1; // closing tags count too
        }
        let mut end = start;
        while end < bytes.len() && bytes[end].is_ascii_alphabetic() {
            end += 1;
        }
        let name = &lower[start..end];
        if let Some(&token) = FORBIDDEN_ELEMENTS.iter().find(|&&known| known == name) {
            return Some(token);
        }
        if name == "meta" {
            if let Some(close) = lower[end..].find('>') {
                let tag = &lower[end..end + close];
                // Loose co-occurrence of both words anywhere inside the tag,
                // consistent with token scanning; a false positive costs one
                // retry even though the token name sounds more specific.
                if tag.contains("http-equiv") && tag.contains("refresh") {
                    return Some("<meta http-equiv=refresh>");
                }
            }
        }
        from = end;
    }
    None
}

/// The first `name=` whose name is a forbidden URL carrier, an `hx-*`
/// attribute, or an `on*` inline handler.
fn attribute_token(lower: &str) -> Option<&'static str> {
    let bytes = lower.as_bytes();
    for (equals, _) in lower.match_indices('=') {
        let mut start = equals;
        while start > 0 && (bytes[start - 1].is_ascii_alphabetic() || bytes[start - 1] == b'-') {
            start -= 1;
        }
        let name = &lower[start..equals];
        if let Some(&token) = FORBIDDEN_ATTRIBUTES.iter().find(|&&known| known == name) {
            return Some(token);
        }
        if name.len() > 3 && name.starts_with("hx-") {
            return Some("hx-*");
        }
        if name.len() > 2 && name.starts_with("on") {
            return Some("on*");
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::forbidden_token;

    #[test]
    fn plain_markup_and_inline_style_pass() {
        let html = "<section><h1>Tickets</h1>\
                    <ul><li style=\"color: red\">one</li></ul></section>";
        assert_eq!(forbidden_token(html), None);
    }

    #[test]
    fn a_charset_meta_passes() {
        // Only the refresh form of <meta> is forbidden; <meta charset> is not.
        assert_eq!(forbidden_token("<meta charset=\"utf-8\">"), None);
    }

    #[test]
    fn every_forbidden_element_trips() {
        for (snippet, name) in [
            ("<form>", "form"),
            ("<iframe src=\"https://x\">", "iframe"),
            ("<object data=\"x\">", "object"),
            ("<embed src=\"x\">", "embed"),
            ("<base href=\"/x\">", "base"),
            ("<link rel=\"stylesheet\">", "link"),
            ("<script>alert(1)</script>", "script"),
            ("<style>p {}</style>", "style"),
        ] {
            assert_eq!(forbidden_token(snippet), Some(name), "trip on {snippet}");
        }
    }

    #[test]
    fn element_names_are_matched_whole() {
        // "<form" must not match "<format>"; the tag name ends where the
        // letters end.
        assert_eq!(forbidden_token("<format>{{ x }}</format>"), None);
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert_eq!(forbidden_token("<SCRIPT>alert(1)</SCRIPT>"), Some("script"));
        assert_eq!(forbidden_token("<a HREF=\"/x\">go</a>"), Some("href"));
    }

    #[test]
    fn every_url_carrying_attribute_trips() {
        for (snippet, name) in [
            ("<a href=\"/x\">", "href"),
            ("<video poster=\"x.jpg\">", "poster"),
            ("<img src=\"x.png\">", "src"),
            ("<img srcset=\"x.png 1w\">", "srcset"),
            ("<form action=\"/x\">", "form"),
            ("<button formaction=\"/x\">", "formaction"),
            ("<a ping=\"/track\">", "ping"),
        ] {
            assert_eq!(forbidden_token(snippet), Some(name), "trip on {snippet}");
        }
    }

    #[test]
    fn every_htmx_attribute_trips_under_one_name() {
        for snippet in ["<div hx-get=\"/x\">", "<button hx-post=\"/y\">"] {
            assert_eq!(forbidden_token(snippet), Some("hx-*"), "trip on {snippet}");
        }
    }

    #[test]
    fn every_inline_handler_trips_under_one_name() {
        for snippet in ["<p onclick=\"f()\">", "<body onload=\"f()\">"] {
            assert_eq!(forbidden_token(snippet), Some("on*"), "trip on {snippet}");
        }
    }

    #[test]
    fn css_url_and_import_trip_anywhere() {
        assert_eq!(
            forbidden_token("<p style=\"background: url(x.png)\">"),
            Some("url(")
        );
        assert_eq!(
            forbidden_token("<p style=\"@import 'x.css'\">"),
            Some("@import")
        );
    }

    #[test]
    fn a_refresh_meta_trips_by_its_parts() {
        // Neither `href`, `src`, nor `action` appears; the ban is on the
        // combination, which is why it is named separately.
        assert_eq!(
            forbidden_token("<meta http-equiv=\"refresh\" content=\"0\">"),
            Some("<meta http-equiv=refresh>")
        );
    }

    #[test]
    fn a_closing_tag_alone_trips() {
        assert_eq!(forbidden_token("</iframe>"), Some("iframe"));
    }

    #[test]
    fn bounds_edges_are_safe() {
        // An empty input names no token.
        assert_eq!(forbidden_token(""), None);
        // "<3" is prose, not a tag: `3` is not alphabetic.
        assert_eq!(forbidden_token("<3 hearts"), None);
        // A non-ASCII byte right after `<` ends the tag name immediately.
        assert_eq!(forbidden_token("<é"), None);
    }

    #[test]
    fn phase_order_beats_document_order_across_kinds() {
        // Records the settled three-phase order (css -> element -> attr):
        // the CSS token is reported even though `href` appears first in
        // document order. Both refuse; only the reported name varies.
        assert_eq!(forbidden_token("<a href=\"/x\">url(y)</a>"), Some("url("));
    }

    #[test]
    fn a_data_attribute_is_not_a_link() {
        // `data-href` does nothing without script, and script is banned
        // separately; the whole-name match keeps `data-*` out of the scan.
        assert_eq!(forbidden_token("<li data-href=\"/x\">"), None);
    }

    #[test]
    fn prose_mentioning_a_token_trips_and_that_is_accepted() {
        // A row value whose own text trips the scan — either an exact
        // attribute name like `href=` or a bare prefix like the `on` of
        // `online=` — costs one retry, never a wrong page. Recorded so
        // nobody "fixes" it.
        assert_eq!(forbidden_token("<p>wrote href= today</p>"), Some("href"));
        assert_eq!(forbidden_token("<p>online=yes</p>"), Some("on*"));
    }
}
