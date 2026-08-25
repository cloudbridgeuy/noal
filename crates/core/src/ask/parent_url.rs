//! The window a follow-up ask came from, read from one header.
//!
//! htmx sends `HX-Current-URL` on every request, and question 104 fixed the
//! address bar as the one copy of "which window the user is standing on".
//! This module is that decision's string half: a pure parser over the header
//! value, testable by comparison like every other prompt-side rule.

/// The path segment of an `/w/<segment>` address, when the value names one.
///
/// The rules, all decided by Q104:
///
/// - An optional scheme and authority are skipped, so both a bare
///   `/w/<segment>` and what a browser puts in the address bar parse.
/// - Everything from the first `?` or `#` on is cut: a query or fragment
///   does not change which window is on screen.
/// - The rest must be exactly `/w/<segment>` — non-empty segment, nothing
///   after it. Anything else is `None`, and `None` means a root ask.
///
/// Whether the segment names a real, owned window is the shell's concern;
/// this function only says whether the string has the shape of a window
/// address.
#[must_use]
pub fn window_segment(value: &str) -> Option<&str> {
    // Skip an optional scheme and authority. A scheme is `://`-terminated;
    // the authority ends at the first `/`.
    let path = match value.split_once("://") {
        Some((_, rest)) => match rest.find('/') {
            Some(slash) => &rest[slash..],
            // A bare origin carries no path at all.
            None => return None,
        },
        None => value,
    };

    // Cut at the first query or fragment marker, whichever comes first.
    let end = path.find(['?', '#']).unwrap_or(path.len());
    let path = &path[..end];

    let segment = path.strip_prefix("/w/")?;
    // Nothing may follow the segment: `/w/8f2c/more` is another address,
    // not this window's.
    if segment.is_empty() || segment.contains('/') {
        return None;
    }
    Some(segment)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::window_segment;

    #[test]
    fn a_bare_window_path_names_its_segment() {
        assert_eq!(
            window_segment("/w/8f2c1a2b-0000-0000-0000-000000000001"),
            Some("8f2c1a2b-0000-0000-0000-000000000001")
        );
    }

    #[test]
    fn a_full_address_is_reduced_to_the_segment() {
        assert_eq!(window_segment("https://noal.example/w/8f2c"), Some("8f2c"));
        assert_eq!(window_segment("http://localhost:8787/w/8f2c"), Some("8f2c"));
    }

    #[test]
    fn a_query_and_a_fragment_are_cut() {
        assert_eq!(window_segment("/w/8f2c?q=1"), Some("8f2c"));
        assert_eq!(window_segment("/w/8f2c#heading"), Some("8f2c"));
        assert_eq!(window_segment("https://x.dev/w/8f2c?a=b#c?d"), Some("8f2c"));
    }

    #[test]
    fn anything_that_is_not_exactly_a_window_path_is_none() {
        // The root and every non-window route mean a root ask.
        assert_eq!(window_segment("/"), None);
        assert_eq!(window_segment("/ask"), None);
        assert_eq!(window_segment("/auth/login"), None);
        // A prefix is not the route; a suffix is another address.
        assert_eq!(window_segment("/w"), None);
        assert_eq!(window_segment("/wx/8f2c"), None);
        assert_eq!(window_segment("/w/8f2c/more"), None);
        // An empty segment names nothing.
        assert_eq!(window_segment("/w/"), None);
        assert_eq!(window_segment("/w/?q=1"), None);
    }

    #[test]
    fn degenerate_values_are_none() {
        assert_eq!(window_segment(""), None);
        assert_eq!(window_segment("https://x.dev"), None);
        assert_eq!(window_segment("https://x.dev/"), None);
        assert_eq!(window_segment("not a url"), None);
    }
}
