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
#debug-panel { position: fixed; inset: 0 0 0 auto; width: min(40rem, 100%); background: #111; color: #eee;
  overflow: auto; padding: 1rem; font: 13px/1.4 ui-monospace, monospace; z-index: 9; }
#debug-panel[hidden] { display: none; }
#debug-panel pre { white-space: pre-wrap; background: #222; padding: .5rem; }
#debug-panel header { display: flex; align-items: baseline; gap: 1rem; }
#debug-copy { font: inherit; }
";

/// The script that toggles the panel and fills it from the last answer.
///
/// It reads `#ask-debug` after every htmx swap, so each answer carries its own
/// data and the chrome never needs to know what an ask is.
const OVERLAY_SCRIPT: &str = r#"
(function () {
  var panel = document.getElementById('debug-panel');
  var toggle = document.getElementById('debug-toggle');
  function show(on) { panel.hidden = !on; }
  toggle.addEventListener('click', function () { show(panel.hidden); });
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
  document.body.addEventListener('htmx:afterSwap', function () {
    var el = document.getElementById('ask-debug');
    if (!el) return;
    try { render(JSON.parse(el.textContent)); } catch (err) { console.error('debug payload', err); }
  });
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

/// The hidden panel and its toggle, present on every page.
fn debug_overlay() -> Markup {
    html! {
        button #debug-toggle type="button" title="Toggle debug panel" { "debug" }
        aside #debug-panel hidden {
            header {
                h2 { "debug" }
                button #debug-copy type="button" { "copy" }
            }
            div #debug-content { p { "Ask something; the plan, template, and timings appear here." } }
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

/// Whether a page carries a working command palette.
///
/// There is no closed-but-present state: a page either has the palette, open
/// and focused, with its `#ask-result` swap target alongside it, or it has
/// neither. A palette with nowhere to swap into would be decoration, so the
/// two always travel together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Palette {
    /// The palette is on the page, visible and holding focus.
    Open,
    /// The page has no palette at all.
    Closed,
}

/// Wrap body markup in the full document chrome.
///
/// Use this for a normal navigation. For an htmx swap, return the fragment on
/// its own instead; sending a whole document into a swap target nests one
/// `<html>` inside another.
#[must_use]
pub fn page(title: &str, viewer: &Viewer, palette: Palette, body: &Markup) -> Markup {
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
            body {
                header { (header(viewer)) }
                @if matches!(palette, Palette::Open) {
                    (render_palette())
                }
                main { (body) }
                (debug_overlay())
            }
        }
    }
}

/// The command palette itself: a toggle and the ask form, in its own chrome.
///
/// Rendered only from [`page`], and only for [`Palette::Open`]. The toggle
/// button has no click behaviour yet — wiring it is a later slice's job — so
/// this renders the affordance without pretending it does anything today.
fn render_palette() -> Markup {
    html! {
        div #palette {
            button #palette-toggle type="button" title="Command palette" { "Palette" }
            (crate::ask::form())
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
    use super::{header, page, Palette, Viewer};
    use maud::html;

    #[test]
    fn a_page_is_a_whole_document() {
        let markup = page(
            "Home",
            &Viewer::Anonymous,
            Palette::Closed,
            &html! { p { "hello" } },
        );
        let rendered = markup.into_string();
        assert!(rendered.starts_with("<!DOCTYPE html>"));
        assert!(rendered.contains("<title>Home · noal</title>"));
        assert!(rendered.contains("<p>hello</p>"));
    }

    #[test]
    fn every_page_carries_the_debug_overlay() {
        let rendered = page("Home", &Viewer::Anonymous, Palette::Closed, &html! {}).into_string();
        assert!(rendered.contains("id=\"debug-panel\""));
        assert!(rendered.contains("htmx:afterSwap"));
    }

    #[test]
    fn a_closed_palette_puts_no_palette_markup_on_the_page() {
        let rendered = page("Home", &Viewer::Anonymous, Palette::Closed, &html! {}).into_string();
        assert!(!rendered.contains("id=\"palette\""));
        assert!(!rendered.contains("id=\"ask-form\""));
    }

    #[test]
    fn an_open_palette_is_focused_and_unhidden() {
        let rendered = page(
            "Home",
            &Viewer::SignedIn {
                email: "someone@example.com".to_owned(),
            },
            Palette::Open,
            &html! {},
        )
        .into_string();
        assert!(rendered.contains("id=\"palette\""));
        assert!(!rendered.contains("id=\"palette\" hidden"));
        assert!(rendered.contains("autofocus"));
        assert!(rendered.contains("id=\"ask-input\""));
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
