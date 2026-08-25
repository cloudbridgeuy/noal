//! The saved-window tree shown in the palette's Windows tab.
//!
//! The tree is data until it renders. The shell gathers [`Entry`] values and
//! their nesting — both defined in `noal_core::window`, so the shell builds
//! exactly one kind of tree — and this module turns them into markup. Nothing
//! here touches a connection, so the whole module is tested by comparing
//! strings.

use std::borrow::Cow;

use maud::{html, Markup};
pub use noal_core::window::{Entry, Node};

/// The most characters a window label shows before it is cut.
///
/// This is a fact about how much text fits the drawer's width, not about
/// windows themselves, so the cut lives beside the rendering rather than
/// wherever windows are created.
const LABEL_LIMIT: usize = 60;

/// Cut a request down to `LABEL_LIMIT` characters on a word boundary.
///
/// The cut is marked with an ellipsis. When the request has no space to cut at,
/// it falls back to a hard cut at the limit rather than overflowing the drawer.
#[must_use]
pub fn cut(request: &str) -> Cow<'_, str> {
    if request.chars().count() <= LABEL_LIMIT {
        return Cow::Borrowed(request);
    }
    let head: String = request.chars().take(LABEL_LIMIT).collect();
    let kept = match head.rfind(char::is_whitespace) {
        Some(index) if index > 0 => &head[..index],
        _ => &head[..],
    };
    Cow::Owned(format!("{kept}…"))
}

/// What a tree row shows: the viewer's name when there is one, otherwise the
/// cut request.
///
/// A free function rather than an inherent method because `Entry` lives in
/// the core; the label is a fact about the drawer, not about windows.
fn label(entry: &Entry) -> Cow<'_, str> {
    match &entry.name {
        Some(name) => Cow::Borrowed(name.as_str()),
        None => cut(&entry.request),
    }
}

/// What the shell knows about the viewer's saved windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Windows {
    /// The windows themselves, already nested.
    Tree(Vec<Node>),
    /// The store could not be read. The page says so rather than showing an
    /// empty tree that claims there are no windows.
    Unavailable,
}

/// Where the viewer currently is, so the tree can mark that row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Current {
    /// The start page; its Home row is marked.
    Home,
    /// A saved window, by id; its row is marked when it appears in the tree.
    Window(uuid::Uuid),
}

/// Render the Windows tab: the Home row, then either the saved tree or the
/// line explaining that the tree could not be read.
#[must_use]
pub fn tree(windows: &Windows, current: &Current) -> Markup {
    html! {
        nav #window-tree { (listing(windows, current)) }
    }
}

/// The same tab as [`tree`], marked for htmx to swap in out of band.
///
/// An answer fragment carries this beside itself after a saved ask, so the
/// palette's tree grows without a page load. It must only be sent with an
/// answer that swapped into a full document — the swap target owns the
/// element this replaces.
#[must_use]
pub fn oob_tree(windows: &Windows, current: &Current) -> Markup {
    html! {
        nav #window-tree hx-swap-oob="outerHTML" { (listing(windows, current)) }
    }
}

/// Everything inside the `<nav>`: Home plus whatever state the windows are in.
fn listing(windows: &Windows, current: &Current) -> Markup {
    html! {
        ul {
            (home_row(current))
            @match windows {
                Windows::Tree(nodes) => {
                    @for node in nodes {
                        (branch(node, current))
                    }
                }
                Windows::Unavailable => {
                    li .windows-unavailable { "Saved windows could not be read." }
                }
            }
        }
    }
}

/// The Home row, which is current when the viewer is on the start page.
fn home_row(current: &Current) -> Markup {
    let marked = matches!(current, Current::Home);
    html! {
        li id=[marked.then_some("window-current")] {
            a href="/" title="Home" { "Home" }
        }
    }
}

/// One window row with its nested children, if it has any.
fn branch(node: &Node, current: &Current) -> Markup {
    let marked = matches!(current, Current::Window(id) if *id == node.entry.id);
    html! {
        li id=[marked.then_some("window-current")] {
            a href=(format!("/w/{}", node.entry.id)) title=(node.entry.request) {
                (label(&node.entry))
            }
            @if marked {
                // Refresh is arrival at the window you stand on: an ordinary
                // anchor, so the browser loads a fresh document and re-runs
                // the window with no noal script at all. It sits on the
                // marked row alone — re-running a window you are not standing
                // on is a visit, and the entry beside it does that.
                a href=(format!("/w/{}", node.entry.id))
                  class="refresh"
                  title="Run this window again" { "↻" }
            }
            @if !node.children.is_empty() {
                ul {
                    @for child in &node.children {
                        (branch(child, current))
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{cut, tree, Current, Entry, Node, Windows};

    /// One deterministic id per short name, so assertions can build the exact
    /// `/w/<id>` text a row links to.
    fn id(name: &str) -> uuid::Uuid {
        let mut bytes = [0_u8; 16];
        for (index, byte) in name.bytes().rev().enumerate().take(16) {
            bytes[15 - index] = byte;
        }
        uuid::Uuid::from_bytes(bytes)
    }

    fn entry(name: &str, request: &str) -> Entry {
        Entry {
            id: id(name),
            parent_id: None,
            request: request.to_owned(),
            name: None,
        }
    }

    #[test]
    fn a_request_within_the_limit_is_untouched() {
        let request = "open tasks under the Render MVP epic";
        assert_eq!(cut(request), request);
    }

    #[test]
    fn a_request_exactly_at_the_limit_is_untouched() {
        let request = "x".repeat(60);
        assert_eq!(cut(&request), request);
    }

    #[test]
    fn a_long_request_cuts_between_words_not_inside_one() {
        let request = format!("{} tail", "word ".repeat(14));
        // Sixty characters land exactly on a trailing space, so the cut keeps
        // twelve whole words and drops the partial thirteenth.
        let expected = format!("{}…", "word ".repeat(12).trim_end());
        assert_eq!(cut(&request), expected);
    }

    #[test]
    fn a_request_without_spaces_falls_back_to_a_hard_cut() {
        let request = "x".repeat(80);
        let expected = format!("{}…", "x".repeat(60));
        assert_eq!(cut(&request), expected);
    }

    #[test]
    fn the_tree_is_a_nav_named_window_tree_with_home_in_it() {
        let rendered = tree(&Windows::Tree(vec![]), &Current::Home).into_string();
        assert!(rendered.contains("<nav id=\"window-tree\">"));
        assert!(rendered.contains(">Home</a>"));
        assert!(rendered.contains("href=\"/\""));
    }

    #[test]
    fn home_is_listed_even_when_the_windows_cannot_be_read() {
        let rendered = tree(&Windows::Unavailable, &Current::Home).into_string();
        assert!(rendered.contains(">Home</a>"));
        assert!(rendered.contains("Saved windows could not be read."));
    }

    #[test]
    fn the_full_request_sits_in_the_title_even_when_the_label_is_cut() {
        let request = "open tasks under the Render MVP epic with comments older than last week";
        let node = Node {
            entry: entry("w-1", request),
            children: Vec::new(),
        };
        let rendered = tree(&Windows::Tree(vec![node]), &Current::Window(id("w-1"))).into_string();
        assert!(rendered.contains(&format!("title=\"{request}\"")));
        assert!(rendered.contains("…"));
        assert!(!rendered.contains(&format!(">{request}</a>")));
    }

    #[test]
    fn a_name_overrides_the_cut_request_as_the_visible_label() {
        let mut long = entry(
            "w-1",
            "open tasks under the Render MVP epic with many words attached",
        );
        long.name = Some("Weekly report".to_owned());
        let rendered = tree(
            &Windows::Tree(vec![Node {
                entry: long,
                children: Vec::new(),
            }]),
            &Current::Home,
        )
        .into_string();
        assert!(rendered.contains("Weekly report"));
        assert!(!rendered.contains("…"));
    }

    #[test]
    fn exactly_one_row_carries_the_current_marker() {
        let windows = Windows::Tree(vec![
            Node {
                entry: entry("w-1", "first window"),
                children: vec![Node {
                    entry: entry("w-2", "second window"),
                    children: Vec::new(),
                }],
            },
            Node {
                entry: entry("w-3", "third window"),
                children: Vec::new(),
            },
        ]);

        let rendered = tree(&windows, &Current::Window(id("w-2"))).into_string();
        assert_eq!(rendered.matches("id=\"window-current\"").count(), 1);

        let rendered = tree(&windows, &Current::Home).into_string();
        assert_eq!(rendered.matches("id=\"window-current\"").count(), 1);
        assert!(rendered.contains("<li id=\"window-current\"><a href=\"/\""));
    }

    #[test]
    fn a_row_links_to_its_own_url_and_nests_its_children() {
        let windows = Windows::Tree(vec![Node {
            entry: entry("w-1", "first window"),
            children: vec![Node {
                entry: entry("w-2", "second window"),
                children: Vec::new(),
            }],
        }]);
        let rendered = tree(&windows, &Current::Home).into_string();

        let parent = rendered
            .find(&format!("/w/{}", id("w-1")))
            .unwrap_or_else(|| panic!("the tree links to the parent window"));
        let child = rendered
            .find(&format!("/w/{}", id("w-2")))
            .unwrap_or_else(|| panic!("the tree links to the child window"));
        assert!(child > parent);
        // Nothing closes a row or a list between the two links, so the child
        // hangs inside its parent's row rather than beside it.
        let between = &rendered[parent..child];
        assert!(!between.contains("</li>"));
        assert!(!between.contains("</ul>"));
    }

    #[test]
    fn only_the_current_row_carries_a_refresh_control() {
        let windows = Windows::Tree(vec![
            Node {
                entry: entry("w-1", "first window"),
                children: vec![Node {
                    entry: entry("w-2", "second window"),
                    children: Vec::new(),
                }],
            },
            Node {
                entry: entry("w-3", "third window"),
                children: Vec::new(),
            },
        ]);
        let rendered = tree(&windows, &Current::Window(id("w-2"))).into_string();
        assert_eq!(rendered.matches("class=\"refresh\"").count(), 1);
        let url = format!("/w/{}", id("w-2"));
        assert!(rendered.contains(&format!("href=\"{url}\" class=\"refresh\"")));
        assert!(rendered.contains("title=\"Run this window again\""));
    }

    #[test]
    fn home_and_non_current_windows_have_no_refresh_control() {
        let windows = Windows::Tree(vec![Node {
            entry: entry("w-1", "first window"),
            children: Vec::new(),
        }]);
        let at_home = tree(&windows, &Current::Home).into_string();
        assert!(!at_home.contains("class=\"refresh\""));
        let elsewhere = tree(&windows, &Current::Window(id("w-9"))).into_string();
        assert!(!elsewhere.contains("class=\"refresh\""));
    }

    #[test]
    fn the_oob_tree_is_the_same_nav_marked_for_an_out_of_band_swap() {
        let windows = Windows::Tree(vec![Node {
            entry: entry("w-1", "first window"),
            children: Vec::new(),
        }]);

        let plain = tree(&windows, &Current::Home).into_string();
        let oob = super::oob_tree(&windows, &Current::Home).into_string();

        assert!(oob.starts_with("<nav id=\"window-tree\" hx-swap-oob=\"outerHTML\">"));
        assert!(plain.starts_with("<nav id=\"window-tree\">"));
        // Same content inside, including the honest line when unreadable.
        assert_eq!(
            plain.trim_start_matches("<nav id=\"window-tree\">"),
            oob.trim_start_matches("<nav id=\"window-tree\" hx-swap-oob=\"outerHTML\">")
        );

        let unavailable = super::oob_tree(&Windows::Unavailable, &Current::Home).into_string();
        assert!(unavailable.contains("Saved windows could not be read."));
    }
}
