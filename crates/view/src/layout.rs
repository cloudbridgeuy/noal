//! The page chrome shared by every full document.

use maud::{html, Markup, PreEscaped, DOCTYPE};

/// The htmx build noal loads. Pinned, and served with an integrity hash, so a
/// change to the CDN cannot change what runs in the browser.
const HTMX_SRC: &str = "https://unpkg.com/htmx.org@2.0.4/dist/htmx.min.js";

/// Enough CSS to make the page and the overlay usable. No framework yet.
const STYLE: &str = r"
body { font: 16px/1.5 system-ui, sans-serif; margin: 0; padding: 0 1rem; max-width: 72rem; margin-inline: auto; }
header nav { display: flex; gap: 1rem; padding: 1rem 0; border-bottom: 1px solid #ddd; }
#ask-form { display: grid; gap: .5rem; max-width: 40rem; margin: 2rem 0; }
#ask-form input { font: inherit; padding: .5rem; }
.htmx-indicator { display: none; } .htmx-request .htmx-indicator { display: inline; }
table { border-collapse: collapse; } td, th { border: 1px solid #ddd; padding: .25rem .5rem; text-align: left; }
#debug-toggle { position: fixed; right: 1rem; bottom: 1rem; z-index: 10; }
#palette { position: fixed; inset: 0 0 0 auto; width: min(40rem, 100%); background: #111; color: #eee;
  overflow: auto; padding: 1rem; font: 13px/1.4 ui-monospace, monospace; z-index: 9; }
#palette[hidden] { display: none; }
#palette .tabs { display: flex; gap: 1.5rem; border-bottom: 1px solid #444; margin-bottom: .75rem; padding-bottom: .5rem; }
#palette .tabs span { font-weight: 600; letter-spacing: .04em; text-transform: uppercase; }
#palette ul { list-style: none; margin: 0; padding: 0; }
#palette ul ul { padding-left: 1rem; }
#palette a { color: #9cf; }
.windows-unavailable { color: #f99; }
#palette pre { white-space: pre-wrap; background: #222; padding: .5rem; }
#debug-copy { font: inherit; }
";

/// The script that toggles the drawer, marks the current window row, and fills
/// the debug tab from the last answer.
///
/// It reads `#ask-debug` once when the page loads and again after every htmx
/// swap, so each answer carries its own data and the chrome never needs to know
/// what an ask is. Reading at load time matters for any page that arrives
/// already carrying an answer.
const OVERLAY_SCRIPT: &str = r#"
(function () {
  var panel = document.getElementById('palette');
  var toggle = document.getElementById('debug-toggle');
  function show(on) { panel.hidden = !on; }
  toggle.addEventListener('click', function () {
    show(panel.hidden);
    var current = document.getElementById('window-current');
    if (current) current.scrollIntoView({ block: 'center' });
  });
  function render(data) {
    var out = document.getElementById('debug-content');
    function block(title, text) {
      return '<h3>' + title + '</h3><pre>' + String(text).replace(/[&<>]/g, function (c) {
        return { '&': '&amp;', '<': '&lt;', '>': '&gt;' }[c]; }) + '</pre>';
    }
    var html = block('request', data.request);
    if (data.failed_stage) html += block('failed stage', data.failed_stage);
    if (data.plan) { html += block('sql', data.plan.sql); html += block('shape', JSON.stringify(data.plan.shape, null, 2)); }
    if (data.template) html += block('template', data.template);
    (data.attempts || []).forEach(function (a, i) {
      html += block('attempt ' + (i + 1) + ' · ' + a.stage, a.artifact + '\n\n--- error ---\n' + a.error);
    });
    html += block('timings', (data.timings || []).map(function (t) { return t.stage + ': ' + t.millis + ' ms'; }).join('\n'));
    out.innerHTML = html;
  }
  function update() {
    var el = document.getElementById('ask-debug');
    if (!el) return;
    try { render(JSON.parse(el.textContent)); } catch (err) { console.error('debug payload', err); }
  }
  update();
  document.body.addEventListener('htmx:afterSwap', update);
  var copy = document.getElementById('debug-copy');
  copy.addEventListener('click', function () {
    var el = document.getElementById('ask-debug');
    var text = el ? el.textContent : '';
    try { text = JSON.stringify(JSON.parse(text), null, 2); } catch (err) { /* copy it raw */ }
    if (!text) { copy.textContent = 'nothing yet'; }
    else {
      // A textarea works without the clipboard permission, and on every
      // browser this runs in, so it is the only path rather than a fallback.
      var box = document.createElement('textarea');
      box.value = text;
      box.setAttribute('readonly', '');
      box.style.position = 'fixed';
      box.style.opacity = '0';
      document.body.appendChild(box);
      box.select();
      copy.textContent = document.execCommand('copy') ? 'copied' : 'copy failed';
      document.body.removeChild(box);
    }
    setTimeout(function () { copy.textContent = 'copy'; }, 1500);
  });
})();
"#;

/// The side drawer a signed-in viewer gets: the saved-window tree and the
/// debug panel behind two tabs.
fn palette(chrome: &Chrome) -> Markup {
    use crate::windows::tree;

    html! {
        button #debug-toggle type="button" title="Toggle the side panel" { "menu" }
        aside #palette hidden {
            div .tabs { span { "Windows" } span { "Debug" } }
            section #windows-tab {
                (tree(&chrome.windows, &chrome.current))
            }
            section #debug-tab {
                p { button #debug-copy type="button" { "copy" } }
                div #debug-content {
                    p { "Ask something; the plan, template, and timings appear here." }
                }
            }
        }
        script { (PreEscaped(OVERLAY_SCRIPT)) }
    }
}

/// Who is looking at the page. `view` renders differently for a signed-in user,
/// so the shell passes identity in rather than the template reaching for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Viewer {
    /// Nobody is signed in.
    Anonymous,
    /// A signed-in user, identified by the email noal shows in the header.
    SignedIn {
        /// The address to display.
        email: String,
    },
}

/// Everything a page needs to draw its chrome: who is looking, what their
/// saved windows look like, and where among them they are.
///
/// The shell builds one per request and hands it to [`page`]; the templates
/// never reach for session or storage state themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chrome {
    /// Who is looking at the page.
    pub viewer: Viewer,
    /// The viewer's saved windows as the palette should show them.
    pub windows: crate::windows::Windows,
    /// Where the viewer currently is, so the tree can mark that row.
    pub current: crate::windows::Current,
}

impl Chrome {
    /// Chrome for a page no signed-in viewer ever reaches, such as the failure
    /// page.
    ///
    /// Such a page renders no palette, so the window state never shows; an
    /// empty tree keeps construction total.
    #[must_use]
    pub fn anonymous() -> Self {
        Self {
            viewer: Viewer::Anonymous,
            windows: crate::windows::Windows::Tree(Vec::new()),
            current: crate::windows::Current::Home,
        }
    }
}

/// Wrap body markup in the full document chrome.
///
/// Use this for a normal navigation. For an htmx swap, return the fragment on
/// its own instead; sending a whole document into a swap target nests one
/// `<html>` inside another.
///
/// The body tells htmx never to snapshot the page into its history cache: a
/// page always re-runs on the server when the browser returns to it.
#[must_use]
pub fn page(title: &str, chrome: &Chrome, body: &Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) " · noal" }
                script src=(HTMX_SRC) defer {}
                style { (PreEscaped(STYLE)) }
            }
            body hx-history="false" {
                header { (header(&chrome.viewer)) }
                main { (body) }
                @if let Viewer::SignedIn { .. } = &chrome.viewer {
                    (palette(chrome))
                }
            }
        }
    }
}

/// The masthead, with the sign-in or sign-out control.
#[must_use]
pub fn header(viewer: &Viewer) -> Markup {
    html! {
        nav {
            a href="/" { "noal" }
            @match viewer {
                Viewer::Anonymous => {
                    a href="/auth/login" { "Sign in" }
                }
                Viewer::SignedIn { email } => {
                    span .viewer-email { (email) }
                    a href="/auth/logout" { "Sign out" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{header, page, Chrome, Viewer};
    use crate::windows::{Current, Windows};
    use maud::html;

    fn signed_in(email: &str) -> Chrome {
        Chrome {
            viewer: Viewer::SignedIn {
                email: email.to_owned(),
            },
            windows: Windows::Tree(Vec::new()),
            current: Current::Home,
        }
    }

    #[test]
    fn a_page_is_a_whole_document() {
        let markup = page("Home", &Chrome::anonymous(), &html! { p { "hello" } });
        let rendered = markup.into_string();
        assert!(rendered.starts_with("<!DOCTYPE html>"));
        assert!(rendered.contains("<title>Home · noal</title>"));
        assert!(rendered.contains("<p>hello</p>"));
        assert!(rendered.contains("hx-history=\"false\""));
    }

    #[test]
    fn a_signed_in_page_carries_the_palette_with_both_tabs() {
        let rendered = page("Home", &signed_in("someone@example.com"), &html! {}).into_string();
        assert!(rendered.contains("id=\"palette\""));
        assert!(rendered.contains("<span>Windows</span>"));
        assert!(rendered.contains("<span>Debug</span>"));
        assert!(rendered.contains("id=\"window-tree\""));
        assert!(rendered.contains(">Home</a>"));
        assert!(rendered.contains("id=\"debug-content\""));
        assert!(rendered.contains("htmx:afterSwap"));
        // The debug renderer also runs at load time, for pages that arrive
        // already carrying an answer.
        assert!(rendered.contains("\n  update();"));
    }

    #[test]
    fn an_anonymous_page_carries_no_palette() {
        let rendered = page("Home", &Chrome::anonymous(), &html! {}).into_string();
        assert!(!rendered.contains("id=\"palette\""));
        assert!(!rendered.contains("id=\"debug-toggle\""));
        assert!(!rendered.contains("id=\"debug-content\""));
    }

    #[test]
    fn an_anonymous_viewer_is_offered_sign_in() {
        let rendered = header(&Viewer::Anonymous).into_string();
        assert!(rendered.contains("/auth/login"));
        assert!(!rendered.contains("/auth/logout"));
    }

    #[test]
    fn a_signed_in_viewer_is_offered_sign_out() {
        let viewer = Viewer::SignedIn {
            email: "someone@example.com".to_owned(),
        };
        let rendered = header(&viewer).into_string();
        assert!(rendered.contains("someone@example.com"));
        assert!(rendered.contains("/auth/logout"));
        assert!(!rendered.contains("/auth/login"));
    }

    #[test]
    fn markup_escapes_viewer_supplied_text() {
        let viewer = Viewer::SignedIn {
            email: "<script>alert(1)</script>".to_owned(),
        };
        let rendered = header(&viewer).into_string();
        assert!(!rendered.contains("<script>alert(1)</script>"));
        assert!(rendered.contains("&lt;script&gt;"));
    }
}
