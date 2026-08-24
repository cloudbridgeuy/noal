//! The saved-window tree shown in the palette's Windows tab.
//!
//! The tree is data until it renders. The shell gathers [`Entry`] values and
//! their nesting; the pure [`tree`] function turns them into markup. Nothing
//! here touches a connection, so the whole module is tested by comparing
//! strings.

use std::borrow::Cow;

use maud::{html, Markup};

/// The most characters a window label shows before it is cut.
///
/// This is a fact about how much text fits the drawer's width, not about
/// windows themselves, so the cut lives beside the rendering rather than
/// wherever windows are created.
const LABEL_LIMIT: usize = 60;

/// Cut a request down to [`LABEL_LIMIT`] characters on a word boundary.
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

impl Entry {
    /// What the row shows: the viewer's name when there is one, otherwise the
    /// cut request.
    fn label(&self) -> Cow<'_, str> {
        match &self.name {
            Some(name) => Cow::Borrowed(name.as_str()),
            None => cut(&self.request),
        }
    }
}

/// One saved window as the tree shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The window's id; its row links to `/w/<id>`.
    pub id: String,
    /// The request that produced the window. Shown cut, and carried in full in
    /// the row's tooltip.
    pub request: String,
    /// The name the viewer gave the window. When present it replaces the cut
    /// request as the visible label.
    pub name: Option<String>,
}

/// One row of the tree: an entry and the entries nested under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// The window this row shows.
    pub entry: Entry,
    /// The windows whose parent is this one, in creation order.
    pub children: Vec<Node>,
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
    Window(String),
}

/// Render the Windows tab: the Home row, then either the saved tree or the
/// line explaining that the tree could not be read.
#[must_use]
pub fn tree(windows: &Windows, current: &Current) -> Markup {
    html! {
        nav #window-tree {
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
                (node.entry.label())
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

    fn entry(id: &str, request: &str) -> Entry {
        Entry {
            id: id.to_owned(),
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
        let rendered = tree(
            &Windows::Tree(vec![node]),
            &Current::Window("w-1".to_owned()),
        )
        .into_string();
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

        let rendered = tree(&windows, &Current::Window("w-2".to_owned())).into_string();
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
            .find("/w/w-1")
            .unwrap_or_else(|| panic!("the tree links to the parent window"));
        let child = rendered
            .find("/w/w-2")
            .unwrap_or_else(|| panic!("the tree links to the child window"));
        assert!(child > parent);
        // Nothing closes a row or a list between the two links, so the child
        // hangs inside its parent's row rather than beside it.
        let between = &rendered[parent..child];
        assert!(!between.contains("</li>"));
        assert!(!between.contains("</ul>"));
    }
}
