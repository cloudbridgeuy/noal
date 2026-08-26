//! The saved-window tree shown in the palette's Windows tab.
//!
//! The tree is data until it renders. The shell gathers [`Entry`] values and
//! their nesting — both defined in `noal_core::window`, so the shell builds
//! exactly one kind of tree — and this module turns them into markup. Nothing
//! here touches a connection, so the whole module is tested by comparing
//! strings.

use std::borrow::Cow;

use maud::{html, Markup};
pub use noal_core::window::{normalize_name, Entry, Name as NormalizedName, Node, NAME_LIMIT};

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

impl Current {
    /// The id of the window being viewed, or `None` on Home.
    #[must_use]
    pub fn window_id(&self) -> Option<uuid::Uuid> {
        match self {
            Current::Home => None,
            Current::Window(id) => Some(*id),
        }
    }
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
///
/// The row holds two controls: the label link, and a rename form hidden
/// inside the same row. The form is per-row markup, not fetched — no route
/// serves it — so opening the editor is only unhiding what is already there.
/// The current-window marker input rides on the current row alone; scripts
/// read it to learn which window the viewer is looking at.
fn branch(node: &Node, current: &Current) -> Markup {
    let marked = matches!(current, Current::Window(id) if *id == node.entry.id);
    html! {
        li id=[marked.then_some("window-current")] {
            a .window-label href=(format!("/w/{}", node.entry.id)) title=(node.entry.request) {
                (label(&node.entry))
            }
            // The opener stays outside the form: it must be clickable while
            // the form it opens is still hidden.
            button .window-rename-open type="button" aria-haspopup="dialog"
                title="Rename" {
                "Rename"
            }
            (rename_form(node, marked))
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

/// The hidden rename form of one row.
///
/// It pre-fills from the stored name only — never the derived cut — because
/// an unchanged submit must not write an invention into the column. Empty
/// submit clears the name, which is why `required` stays off. The label and
/// the buttons inside carry the accessible names for the edit they perform.
///
/// The current row's form carries a `current-window` field; scripts and the
/// route read it to learn that the renamed window is the one being viewed.
fn rename_form(node: &Node, marked: bool) -> Markup {
    let id = node.entry.id;
    html! {
        form .window-rename hidden hx-post=(format!("/w/{id}/name"))
            hx-target="#window-tree" hx-swap="outerHTML"
            hx-sync="this:drop" hx-disabled-elt="find .window-rename-submit" {
            @if marked {
                input type="hidden" name="current-window" value="true" {}
            }
            label for=(rename_input_id(id)) { "Name this window" }
            input #(rename_input_id(id)) name="name" type="text"
                value=[node.entry.name.as_deref()];
            button .window-rename-submit type="submit" { "Save name" }
            button .window-rename-cancel type="button" { "Cancel" }
        }
    }
}

/// A stable DOM id for one row's rename input.
fn rename_input_id(id: uuid::Uuid) -> String {
    format!("window-name-{}", id.as_simple())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
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

    // The rename control ships inside every row. These string tests pin the
    // markup the overlay script depends on: a hidden form per row, opened by
    // its own button, with the accessible names a screen reader needs.

    #[test]
    fn every_row_ships_a_hidden_rename_form_that_posts_to_the_window_s_route() {
        let rendered = tree(
            &Windows::Tree(vec![Node {
                entry: entry("w-1", "first window"),
                children: Vec::new(),
            }]),
            &Current::Home,
        )
        .into_string();
        // The tree holds exactly one kind of form; inspect its opening tag.
        let tag = opening_form_tag(&rendered);
        assert!(tag.contains("hidden"));
        assert!(tag.contains("hx-post="));
        assert!(tag.contains("/name"));
        // The form targets the tree itself, so a saved rename swaps the fresh
        // tree in place and the palette never has to close.
        assert!(tag.contains(r##"hx-target="#window-tree""##));
    }

    #[test]
    fn the_rename_form_drops_a_second_submit_and_disables_only_the_save_button() {
        let rendered = tree(
            &Windows::Tree(vec![Node {
                entry: entry("w-1", "first window"),
                children: Vec::new(),
            }]),
            &Current::Home,
        )
        .into_string();
        let tag = opening_form_tag(&rendered);
        assert!(tag.contains(r#"hx-sync="this:drop""#));
        assert!(tag.contains(r#"hx-disabled-elt="find .window-rename-submit""#));

        // The cancel control must stay outside hx-disabled-elt's reach so a
        // slow rename can still be put away mid-flight.
        assert!(!tag.contains("window-rename-cancel"));
    }

    /// Pull the opening `<form ...>` tag of the rendered tree's one form.
    fn opening_form_tag(rendered: &str) -> &str {
        let start = rendered.find("<form").unwrap();
        let end = start + rendered[start..].find('>').unwrap();
        &rendered[start..=end]
    }

    #[test]
    fn the_rename_input_pre_fills_with_the_stored_name_never_the_cut() {
        let mut named = entry(
            "w-1",
            "open tasks under the Render MVP epic with many words",
        );
        named.name = Some("Weekly report".to_owned());
        let unnamed = entry("w-2", "second window");

        let rendered = tree(
            &Windows::Tree(vec![
                Node {
                    entry: named,
                    children: Vec::new(),
                },
                Node {
                    entry: unnamed,
                    children: Vec::new(),
                },
            ]),
            &Current::Home,
        )
        .into_string();

        assert!(rendered.contains(r#"value="Weekly report""#));
        // No stored name, no value attribute at all — not the cut request.
        assert!(!rendered.contains("value=\"second window\""));
        assert!(!rendered.contains("…</input>") && !rendered.contains("value=\"open tasks under"));
    }

    #[test]
    fn the_rename_controls_carry_accessible_names() {
        let rendered = tree(
            &Windows::Tree(vec![Node {
                entry: entry("w-1", "first window"),
                children: Vec::new(),
            }]),
            &Current::Home,
        )
        .into_string();

        // The opener announces what it opens; it sits beside the form, so a
        // click can reach it while the form is hidden.
        let open_tag = opening_button_tag(&rendered, "window-rename-open");
        assert!(open_tag.contains(r#"aria-haspopup="dialog""#));
        let open_at = rendered.find("window-rename-open").unwrap();
        let form_open = rendered[open_at..].find("<form").unwrap();
        assert!(form_open > 0, "the form follows its opener");
        // The input has a real label bound by matching for/id, and the
        // submit button is text a screen reader can name.
        assert!(rendered.contains(r#"<label for="window-name-"#));
        let label_at = rendered.find(r#"for="window-name-"#).unwrap();
        let id_at = rendered[label_at..].find("window-name-").unwrap() + label_at;
        let input_id = &rendered[id_at..rendered[id_at..].find('"').unwrap() + id_at];
        assert!(rendered.contains(&format!("id=\"{input_id}\"")));
        assert!(rendered.contains(">Save name</button>"));
        assert!(rendered.contains(">Cancel</button>"));
    }

    #[test]
    fn the_rename_opener_is_visible_outside_its_hidden_form() {
        // The opener must be clickable while the form is still hidden, so
        // it cannot live inside the element the button's own click reveals.
        let rendered = tree(
            &Windows::Tree(vec![Node {
                entry: entry("w-1", "first window"),
                children: Vec::new(),
            }]),
            &Current::Home,
        )
        .into_string();
        let open_at = rendered.find("window-rename-open").unwrap();
        let form_open = rendered.find("<form").unwrap();
        let form_close = rendered.find("</form>").unwrap();
        assert!(open_at < form_open || open_at > form_close);
    }

    /// Pull the opening `<button ...>` tag carrying `class_needle`.
    fn opening_button_tag<'a>(rendered: &'a str, class_needle: &'a str) -> &'a str {
        let at = rendered.find(class_needle).unwrap();
        let start = rendered[..at].rfind('<').unwrap();
        let end = at + rendered[at..].find('>').unwrap();
        &rendered[start..=end]
    }

    #[test]
    fn only_the_current_row_s_form_carries_the_current_window_marker() {
        let windows = Windows::Tree(vec![
            Node {
                entry: entry("w-1", "first window"),
                children: Vec::new(),
            },
            Node {
                entry: entry("w-2", "second window"),
                children: Vec::new(),
            },
        ]);

        let rendered = tree(&windows, &Current::Window(id("w-2"))).into_string();
        assert_eq!(
            rendered.matches("current-window").count(),
            1,
            "the marker rides on exactly one form"
        );
        // The marker rides on the form of the row that is current: the
        // nearest opening tag before it posts to that row's own route.
        let marker_at = rendered.find("current-window").unwrap();
        let owner = rendered[..marker_at].rfind("<form").unwrap();
        assert!(rendered[owner..marker_at].contains(&format!("/w/{}/name", id("w-2"))));

        // And on Home no form carries it.
        let home = tree(&windows, &Current::Home).into_string();
        assert!(!home.contains("current-window"));
    }

    #[test]
    fn the_rename_form_is_a_sibling_of_the_label_link_not_inside_it() {
        let rendered = tree(
            &Windows::Tree(vec![Node {
                entry: entry("w-1", "first window"),
                children: Vec::new(),
            }]),
            &Current::Home,
        )
        .into_string();
        let link_close = rendered.find("</a>").unwrap();
        let form_open = rendered.find("<form").unwrap();
        assert!(form_open > link_close);
    }
}
