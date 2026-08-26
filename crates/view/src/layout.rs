//! The page chrome shared by every full document.

use maud::{html, Markup, PreEscaped, DOCTYPE};
use noal_core::ask::outcome::Outcome;

/// The htmx build noal loads. Pinned, and served with an integrity hash, so a
/// change to the CDN cannot change what runs in the browser.
const HTMX_SRC: &str = "https://unpkg.com/htmx.org@2.0.4/dist/htmx.min.js";

/// The Inter typeface, loaded from its CDN. `--font` in [`STYLE`] names
/// system-ui first as the stack to fall back on whenever this URL cannot be
/// reached, so a page never blocks rendering on the download.
const INTER_STYLESHEET_HREF: &str =
    "https://fonts.googleapis.com/css2?family=Inter:wght@400;600&display=swap";

/// The styles every page carries: Zinc theme variables for content pages,
/// fixed dark colors for the side drawer, component classes shared by all
/// views, and a small set of utilities.
const STYLE: &str = r#"
*, ::before, ::after { box-sizing: border-box; }

:root {
  --bg: #fafafa;
  --fg: #09090b;
  --border: #e4e4e7;
  --primary: #18181b;
  --muted: #71717a;
  --radius: 0.5rem;
  --font: 'Inter', system-ui, sans-serif;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #09090b;
    --fg: #fafafa;
    --border: #27272a;
    --primary: #fafafa;
    --muted: #a1a1aa;
  }
}

body {
  max-width: 72rem; margin: 0 auto; padding: 0 1rem;
  font-family: var(--font); font-size: 16px; line-height: 1.5;
  background: var(--bg); color: var(--fg);
}
a { color: inherit; }
table { border-collapse: collapse; }
td, th { border: 1px solid var(--border); padding: .25rem .5rem; text-align: left; }
header nav { padding: 1rem 0; }
.viewer-email { color: var(--muted); }

.btn {
  display: inline-flex; align-items: center; justify-content: center;
  font: inherit; padding: .375rem .75rem;
  border: 1px solid var(--border); border-radius: var(--radius);
  background: none; color: inherit; cursor: pointer;
}
.btn:hover { opacity: .8; }
.btn-primary { background: var(--primary); border-color: var(--primary); color: var(--bg); }
.btn-ghost { border-color: transparent; background: none; color: var(--muted); }
.input {
  font: inherit; width: 100%; padding: .5rem .625rem;
  border: 1px solid var(--border); border-radius: var(--radius);
  background: none; color: inherit;
}
.card { border: 1px solid var(--border); border-radius: var(--radius); padding: 1rem; }

.flex { display: flex; }
.gap-sm { gap: .5rem; }
.gap-md { gap: 1rem; }
.mt-1 { margin-top: .25rem; }
.border-b { border-bottom: 1px solid var(--border); }
.sr-only { position: absolute; width: 1px; height: 1px; margin: -1px; padding: 0; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; border-width: 0; }

#debug-toggle {
  position: fixed; right: 1rem; bottom: 1rem; z-index: 10;
  font: inherit; padding: .375rem .625rem;
  border: 1px solid var(--border); border-radius: var(--radius);
  background: var(--bg); color: var(--fg); cursor: pointer;
}

/* htmx marks the element hx-indicator names, not an ancestor, so the
   same-element selector is the one that fires. */
.htmx-indicator { display: none; }
.htmx-request .htmx-indicator, .htmx-request.htmx-indicator { display: inline; }

#ask-form { display: grid; gap: .5rem; }
#ask-form input { font: inherit; padding: .5rem; }
.sign-out { display: contents; }
.sign-out button { font: inherit; color: inherit; background: none; border: none; padding: 0; cursor: pointer; }

.toast {
  align-items: flex-start; gap: .75rem; padding: .75rem 1rem;
  background: var(--bg); box-shadow: 0 .25rem 1rem rgba(9, 9, 11, .2);
}
.toast p { margin: 0; flex: 1; }

.toast-stack { display: grid; gap: .5rem; max-width: 24rem; }

/* Bottom-left, the corner the drawer's own geometry leaves free.
   Highest z-index of the set: a failure notice must never end up hidden
   behind the drawer (9) or its toggle (10), so 11 sits above both. */
#toasts { position: fixed; left: 1rem; bottom: 1rem; z-index: 11; }
#ask-toast { color: #f87171; }

/* The drawer is dark no matter which theme the viewer's system prefers:
   its colors are hardcoded rather than read from :root. Declaring the
   accent variables here puts them in scope for everything drawn inside. */
#palette {
  position: fixed; inset: 0 0 0 auto; width: min(40rem, 100%);
  overflow: auto; padding: 1rem; font: 13px/1.4 ui-monospace, monospace; z-index: 9;
  background: #09090b; color: #fafafa;
  --accent: #7dd3fc; --accent-hover: #bae6fd;
}
#palette[hidden] { display: none; }
#palette .tabs { display: flex; gap: 1.5rem; border-bottom: 1px solid #27272a; margin-bottom: .75rem; padding-bottom: .5rem; }
#palette .tabs button { font: inherit; font-weight: 600; letter-spacing: .04em; text-transform: uppercase;
  background: none; border: none; color: inherit; padding: 0 0 .25rem; cursor: pointer;
  border-bottom: 2px solid transparent; }
#palette .tabs button.active { color: #fff; border-bottom-color: #9cf; }
.tab-active { color: #7dd3fc; border-bottom-color: #7dd3fc; }
#palette ul { list-style: none; margin: 0; padding: 0; }
#palette ul ul { padding-left: 1rem; }
#palette li#window-current > a { color: var(--accent-hover); }
#palette a { color: var(--accent); }
#palette pre { white-space: pre-wrap; background: #18181b; padding: .5rem; }
.tree-row {
  display: inline-flex; align-items: center; gap: .375rem;
  padding: .25rem .375rem; border-radius: .375rem; text-decoration: none;
}
.tree-row:hover { background: #18181b; }
.windows-unavailable { color: #f87171; }
/* The rename editor hides its row's label link while open. The opener
   button sits beside the form, so it needs its own sizing rule. */
.window-rename-open { font: inherit; color: inherit; background: none; border: none;
  padding: 0; cursor: pointer; }
.window-rename input[type="text"] { font: inherit; width: 100%; box-sizing: border-box; }
.window-rename button { font: inherit; color: inherit; background: none; border: none;
  padding: 0 .25rem; cursor: pointer; }
"#;

/// The script that toggles the drawer, switches the palette tabs, fills the
/// debug tab from the last answer, and wires the keyboard shortcut.
///
/// It reads `#ask-debug` once when the page loads and again after every htmx
/// swap, so each answer carries its own data and the chrome never needs to know
/// what an ask is. Reading at load time matters for any page that arrives
/// already carrying an answer — but it does know what an ask is, now: a `401`
/// sends the browser to sign in, and any other failure becomes a toast.
///
/// Everything below the palette guard needs `#palette` or its children, which
/// exist only for a signed-in viewer; the error listeners above are registered
/// before that guard on purpose, so they run on any page that ever makes an
/// htmx request.
const OVERLAY_SCRIPT: &str = r#"
(function () {
  var panel = document.getElementById('palette');
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
    var text = el.textContent;
    // Blank until the page's own payload or the first ask's out-of-band
    // swap arrives; nothing to parse yet.
    if (!text || !text.trim()) return;
    try { render(JSON.parse(text)); } catch (err) { console.error('debug payload', err); }
  }
  // Runs once at load, for pages that arrive already carrying an answer
  // (a reopened window), and again after every htmx swap.
  update();
  document.body.addEventListener('htmx:afterSwap', update);
  // Clones the hidden `#toast-offline` template (see `page()`) rather than
  // build the markup here, so the wording lives in exactly one place: the
  // same `layout::toast` call every other toast's markup comes from.
  function appendOfflineToast(toasts) {
    var template = document.getElementById('toast-offline');
    if (template) toasts.appendChild(template.content.cloneNode(true));
  }
  // Registered here, ahead of the palette guard below, on purpose: `#ask-form`
  // is the only element that posts an htmx request today, but gating a `401`
  // redirect on the palette being present would mean the redirect silently
  // did nothing on a palette-less page that made one, and the redirect needs
  // no element from the page to run.
  document.body.addEventListener('htmx:responseError', function (event) {
    var xhr = event.detail.xhr;
    if (xhr.status === 401) {
      // A missing or expired session cookie: send the browser to sign in and
      // back to the page it was asking from, rather than leaving a stale
      // toast or an empty swap where the answer should be.
      var next = encodeURIComponent(location.pathname + location.search);
      location.href = '/auth/login?next=' + next;
      return;
    }
    var toasts = document.getElementById('toasts');
    if (!toasts) return;
    // A rendered failure carries its own toast markup as the response body
    // (see `Failure::toast`). An empty body on a non-200 is not expected
    // from noal's own routes, but falls back to the same offline wording a
    // dropped connection shows rather than appending nothing.
    if (xhr.responseText) {
      toasts.insertAdjacentHTML('beforeend', xhr.responseText);
    } else {
      appendOfflineToast(toasts);
    }
  });
  // Fired when the request never reached a response at all -- the Worker is
  // unreachable, not merely answering with an error.
  document.body.addEventListener('htmx:sendError', function () {
    var toasts = document.getElementById('toasts');
    if (toasts) appendOfflineToast(toasts);
  });
  if (panel) {
    var toggle = document.getElementById('debug-toggle');
    var input = document.getElementById('ask-input');
    // The toast region is a sibling of #palette but sits *after* this
    // script in the document, so it does not exist yet while the script
    // runs. Every touch of it looks it up at event time instead.
    // navigator.platform is deprecated, but it is the only signal small
    // enough for a tooltip this size; a wrong guess here is cosmetic.
    toggle.title = /Mac/.test(navigator.platform)
      ? 'Command palette (⌘K)'
      : 'Command palette (Ctrl+K)';
    function togglePalette() {
      // Flip the attribute only. Re-rendering the form would wipe out
      // whatever the user has already typed into it.
      panel.hidden = !panel.hidden;
      if (!panel.hidden) {
        if (input) input.focus();
        // Opening the drawer is how a viewer reaches their saved windows,
        // so land them on the row they are already looking at.
        var current = document.getElementById('window-current');
        if (current) current.scrollIntoView({ block: 'center' });
      }
    }
    toggle.addEventListener('click', togglePalette);
    // Removes one toast. The server only ever appends with `beforeend`, so
    // `#toasts`' last child is always the newest.
    function dismissToast(el) {
      el.remove();
    }
    // Delegated: toasts arrive from the server, appended long after this
    // listener is registered, so a listener bound to each toast would miss
    // every one that shows up later.
    panel.addEventListener('click', function (event) {
      var dismiss = event.target.closest('.toast-dismiss');
      if (!dismiss) return;
      var toasts = document.getElementById('toasts');
      if (toasts) dismissToast(dismiss.closest('.toast'));
    });
    // Rename editors live inside every tree row, swapped in and out with
    // the tree itself, so listeners bind to the palette once and delegate,
    // the same way toast dismissal does.
    //
    // Only one edit runs at a time: opening a second rename puts away the
    // first. Putting away hides the form and shows the label back, and the
    // input's live value is reset to its stored default, so reopening an
    // edit — by button, Cancel, or Escape — always starts from the stored
    // name rather than whatever was typed into the closed editor.
    var openRename = null;
    function putAwayRename() {
      if (!openRename) return;
      var field = openRename.querySelector('input[name="name"]');
      if (field) field.value = field.defaultValue;
      showLabel(openRename);
      openRename.hidden = true;
      openRename = null;
    }
    function showLabel(form) {
      var row = form.closest('li');
      var label = row ? row.querySelector('.window-label') : null;
      if (label) label.hidden = false;
    }
    panel.addEventListener('click', function (event) {
      var opener = event.target.closest('.window-rename-open');
      if (opener) {
        putAwayRename();
        // The opener is the row's visible control; its form is the hidden
        // sibling beside it.
        var form = opener.parentElement.querySelector('form.window-rename');
        if (!form) return;
        openRename = form;
        form.hidden = false;
        var label = form.closest('li').querySelector('.window-label');
        if (label) label.hidden = true;
        var field = form.querySelector('input[name="name"]');
        if (field) { field.focus(); field.select(); }
        return;
      }
      if (event.target.closest('.window-rename-cancel')) putAwayRename();
    });
    // A saved rename answers with the fresh tree, whose swap takes the
    // open editor's markup with it; forget it so Escape never reaches for
    // a detached form.
    document.body.addEventListener('htmx:afterSwap', function () {
      openRename = null;
    });
    var tabs = document.querySelectorAll('#palette .tabs button');
    tabs.forEach(function (tab) {
      tab.addEventListener('click', function () {
        var windows = tab.id === 'tab-windows';
        tabs.forEach(function (t) { t.classList.toggle('active', t === tab); });
        document.getElementById('windows-tab').hidden = !windows;
        document.getElementById('debug-tab').hidden = windows;
      });
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
    document.addEventListener('keydown', function (event) {
      var target = event.target;
      var typingElsewhere = target && target !== input && (
        target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' ||
        target.tagName === 'SELECT' || target.isContentEditable);
      // Escape typed inside the open rename input must still cancel the
      // edit, so the guard steps aside for that one key.
      if (typingElsewhere && !(event.key === 'Escape' && openRename)) return;
      if ((event.metaKey || event.ctrlKey) && !event.shiftKey && !event.altKey &&
          event.key.toLowerCase() === 'k') {
        event.preventDefault();
        togglePalette();
      } else if (event.key === 'Escape') {
        // An open rename edit goes first: Escape inside the editor cancels
        // it, keeping the stored name, and only the next Escape — with no
        // editor open — reaches the toast below.
        if (openRename) { putAwayRename(); return; }
        // The newest toast goes next: one Escape clears it, and only the
        // following Escape — once none remain — closes the palette. The
        // region is looked up now, not at load, because it sits after
        // this script in the document.
        var toasts = document.getElementById('toasts');
        var newestToast = toasts ? toasts.lastElementChild : null;
        if (newestToast) {
          dismissToast(newestToast);
        } else if (!panel.hidden) {
          togglePalette();
        }
      }
    });
    // `HX-Trigger: noal:answered` rides only an answered response; a
    // refused stage sends no such header, so this fires only once the
    // pipeline actually produced an answer. htmx dispatches the named
    // event on the element that made the request (`#ask-form`, which
    // lives inside `#palette`) and it bubbles to `document`, so listening
    // here catches it without binding anything to the form itself.
    //
    // Set `hidden` directly rather than reuse the toggle helper above:
    // that helper flips whatever state the palette is already in, so
    // calling it here would reopen a palette a viewer had already closed
    // before their answer came back.
    document.addEventListener('noal:answered', function () {
      panel.hidden = true;
      if (input) input.value = '';
    });
  }
})();
"#;

/// The side drawer a signed-in viewer gets: the ask form over the
/// saved-window tree and the debug panel, behind two tabs — plus, beside it,
/// the toast region and the offline template.
fn palette(chrome: &Chrome) -> Markup {
    use crate::windows::tree;

    html! {
        button #debug-toggle .btn type="button" title="Toggle the side panel" { "menu" }
        aside #palette hidden {
            div .tabs {
                button #tab-windows type="button" .active.tab-active { "Windows" }
                button #tab-debug type="button" { "Debug" }
            }
            section #windows-tab {
                (tree(&chrome.windows, &chrome.current))
            }
            section #debug-tab hidden {
                p { button #debug-copy .btn.btn-ghost type="button" { "copy" } }
                div #debug-content .card {
                    p { "Ask something; the plan, template, and timings appear here." }
                }
            }
            (crate::ask::form())
            script #ask-debug type="application/json" {
                @if let Some(json) = &chrome.debug_json {
                    (PreEscaped(json))
                }
            }
        }
        script { (PreEscaped(OVERLAY_SCRIPT)) }
    }
}

/// The out-of-band replacement for `#ask-debug`, carrying one ask's payload.
///
/// `hx-swap-oob="outerHTML"` tells htmx to swap this element in wherever the
/// existing `#ask-debug` sits in the document, id and all, rather than where
/// this element appears in the response body. A handler appends the markup
/// this returns beside whatever fragment it renders for the ask itself, for
/// every [`Outcome`], whichever verdict it reached — a refused stage still
/// has attempts and timings worth showing.
#[must_use]
pub fn debug_payload(outcome: &Outcome) -> Markup {
    html! {
        script #ask-debug type="application/json" hx-swap-oob="outerHTML" {
            (PreEscaped(outcome.debug_json()))
        }
    }
}

/// The empty toast region, present whenever the palette is.
///
/// `aria-live="polite"` is why a screen reader announces a toast appended
/// here without anything else on the page changing: a refusal can land
/// while the user is still looking at, or typing into, the palette.
#[must_use]
pub fn toasts() -> Markup {
    html! {
        div #toasts .toast-stack aria-live="polite" {}
    }
}

/// One toast: wording the user can read and select, plus a control to
/// dismiss it.
///
/// The dismiss control is a button *inside* the toast, not the toast itself.
/// A toast carries wording a user may want to copy, so making the whole
/// toast clickable would dismiss it on a text selection, and a screen
/// reader would announce the whole message as a button rather than as text.
#[must_use]
pub fn toast(message: &str) -> Markup {
    html! {
        div .toast.card.flex {
            p { (message) }
            button .toast-dismiss.btn.btn-ghost type="button" aria-label="Dismiss notification" { "×" }
        }
    }
}

/// What a viewer is told when a request never reached noal at all.
const OFFLINE_MESSAGE: &str = "noal could not be reached. Check your connection and try again.";

/// The hidden markup a dropped connection clones into `#toasts`.
///
/// A `<template>`'s content is inert — the browser parses it but never
/// renders or runs it — so it can sit on every page, present but invisible,
/// until `OVERLAY_SCRIPT`'s `htmx:sendError` listener clones it. The toast
/// inside is built by [`toast`], the same call every other toast's markup
/// comes from, so the offline wording is never a second string to keep in
/// step with it.
#[must_use]
fn toast_offline() -> Markup {
    html! {
        template #toast-offline {
            (toast(OFFLINE_MESSAGE))
        }
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
/// saved windows look like, where among them they are, and what the Debug
/// tab opens showing.
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
    /// The debug payload the palette opens with, as JSON, when the page
    /// arrives already carrying an answer — a reopened window. Empty
    /// elsewhere; an ask replaces the element's contents out of band with
    /// [`debug_payload`].
    pub debug_json: Option<String>,
}

impl Chrome {
    /// Chrome for a page no signed-in viewer ever reaches, such as the failure
    /// page.
    ///
    /// Such a page renders no palette, so neither the window state nor a
    /// debug payload ever shows; empty values keep construction total.
    #[must_use]
    pub fn anonymous() -> Self {
        Self {
            viewer: Viewer::Anonymous,
            windows: crate::windows::Windows::Tree(Vec::new()),
            current: crate::windows::Current::Home,
            debug_json: None,
        }
    }
}

/// Wrap body markup in the full document chrome.
///
/// Use this for a normal navigation. For an htmx swap, return the fragment on
/// its own instead; sending a whole document into a swap target nests one
/// `<html>` inside another.
///
/// The palette renders only for a [`Viewer::SignedIn`] viewer: an anonymous
/// viewer gets no palette markup, because there is no session for its ask
/// form to post against, and no toast region either — `#toasts` travels with
/// the palette. The body also tells htmx never to snapshot the page into its
/// history cache: a page always re-runs on the server when the browser
/// returns to it.
#[must_use]
pub fn page(title: &str, chrome: &Chrome, body: &Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) " · noal" }
                link rel="preconnect" href="https://fonts.googleapis.com";
                link rel="preconnect" href="https://fonts.gstatic.com" crossorigin;
                link rel="stylesheet" href=(INTER_STYLESHEET_HREF);
                script src=(HTMX_SRC) defer {}
                style { (PreEscaped(STYLE)) }
            }
            body hx-history="false" {
                header { (header(&chrome.viewer)) }
                main { (body) }
                @if let Viewer::SignedIn { .. } = &chrome.viewer {
                    (palette(chrome))
                    // A sibling of #palette, never a child: a toast must stay
                    // visible even while the palette itself is hidden.
                    (toasts())
                    (toast_offline())
                }
            }
        }
    }
}

/// The masthead, with the sign-in or sign-out control.
#[must_use]
pub fn header(viewer: &Viewer) -> Markup {
    html! {
        nav .flex .gap-md .border-b {
            a href="/" { "noal" }
            @match viewer {
                Viewer::Anonymous => {
                    a href="/auth/login" { "Sign in" }
                }
                Viewer::SignedIn { email } => {
                    span .viewer-email { (email) }
                    // Sign-out revokes the session, so it must be a POST
                    // that a stray link or prefetch can never trigger; a
                    // form is the markup that issues one from a click.
                    form .sign-out action="/auth/logout" method="post" {
                        button type="submit" { "Sign out" }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{debug_payload, header, page, toast, toasts, Chrome, Viewer, OFFLINE_MESSAGE};
    use crate::windows::{Current, Windows};
    use maud::html;
    use noal_core::ask::outcome::{Debug, Origin, Outcome, Stage, Verdict};

    fn signed_in(email: &str) -> Chrome {
        Chrome {
            viewer: Viewer::SignedIn {
                email: email.to_owned(),
            },
            windows: Windows::Tree(Vec::new()),
            current: Current::Home,
            debug_json: None,
        }
    }

    /// An [`Outcome`] with a fixed request, for tests that only care about
    /// the verdict.
    fn outcome(verdict: Verdict) -> Outcome {
        Outcome {
            request: "open tasks".into(),
            verdict,
            origin: Origin::Asked,
            debug: Debug::default(),
        }
    }

    /// Pull the opening tag containing `needle` out of rendered markup.
    ///
    /// Asserting on a fixed attribute order (`"id=\"palette\" hidden"`) would
    /// silently stop meaning anything if maud ever wrote the same attributes
    /// in a different order; this isolates the one tag and inspects its
    /// tokens instead.
    fn opening_tag_containing<'a>(rendered: &'a str, needle: &str) -> &'a str {
        let at = rendered.find(needle).unwrap();
        let start = rendered[..at].rfind('<').unwrap();
        let end = at + rendered[at..].find('>').unwrap();
        &rendered[start..=end]
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
    fn a_page_loads_inter_from_a_cdn_stylesheet_link_in_its_head() {
        let rendered = page("Home", &Chrome::anonymous(), &html! {}).into_string();
        let head_close_at = rendered.find("</head>").unwrap();
        let link = opening_tag_containing(&rendered, "family=Inter");
        assert!(link.starts_with("<link"));
        assert!(link.contains(r#"rel="stylesheet""#));
        // The link sits in the head, where font loading starts before any
        // body markup is drawn.
        assert!(rendered.find(link).unwrap() < head_close_at);
        // The preconnects open the connections the stylesheet and the font
        // files travel over, ahead of the fetch itself.
        assert!(rendered.matches(r#"rel="preconnect""#).count() == 2);
        assert!(rendered.contains("fonts.gstatic.com"));
        // Font loading starts before the style sheet or any script.
        let link_at = rendered.find(link).unwrap();
        assert!(link_at < rendered.find("<style>").unwrap());
        assert!(link_at < rendered.find("unpkg.com/htmx.org").unwrap());
    }

    #[test]
    fn a_signed_in_page_carries_the_palette_with_both_tabs_and_the_ask_form() {
        let rendered = page("Home", &signed_in("someone@example.com"), &html! {}).into_string();
        assert!(rendered.contains("id=\"palette\""));
        assert!(rendered.contains("id=\"window-tree\""));
        assert!(rendered.contains(">Home</a>"));
        assert!(rendered.contains("id=\"debug-content\""));
        assert!(rendered.contains("id=\"ask-form\""));
        assert!(rendered.contains("htmx:afterSwap"));
        // The debug renderer also runs at load time, for pages that arrive
        // already carrying an answer.
        assert!(rendered.contains("\n  update();"));
    }

    #[test]
    fn the_palette_tabs_are_buttons_with_windows_active_by_default() {
        let rendered = page("Home", &signed_in("someone@example.com"), &html! {}).into_string();
        // The tabs are real buttons the script can click.
        assert!(rendered.contains(
            "<button class=\"active tab-active\" id=\"tab-windows\" type=\"button\">Windows</button>"
        ));
        assert!(rendered.contains("<button id=\"tab-debug\" type=\"button\">Debug</button>"));
        // Each section carries the id the script targets, and Debug starts
        // hidden while Windows shows.
        assert!(rendered.contains("<section id=\"windows-tab\">"));
        assert!(rendered.contains("<section id=\"debug-tab\" hidden>"));
    }

    #[test]
    fn the_debug_toggle_takes_the_shared_button_class() {
        let rendered = page("Home", &signed_in("someone@example.com"), &html! {}).into_string();
        let tag = opening_tag_containing(&rendered, "id=\"debug-toggle\"");
        assert!(tag.starts_with("<button"));
        assert!(tag.contains(r#"class="btn""#));
    }

    #[test]
    fn the_debug_copy_button_takes_the_ghost_button_pair() {
        let rendered = page("Home", &signed_in("someone@example.com"), &html! {}).into_string();
        let tag = opening_tag_containing(&rendered, "id=\"debug-copy\"");
        assert!(tag.starts_with("<button"));
        assert!(tag.contains(r#"class="btn btn-ghost""#));
    }

    #[test]
    fn the_debug_content_panel_takes_the_card_class() {
        let rendered = page("Home", &signed_in("someone@example.com"), &html! {}).into_string();
        let tag = opening_tag_containing(&rendered, "id=\"debug-content\"");
        assert!(tag.starts_with("<div"));
        assert!(tag.contains(r#"class="card""#));
    }

    #[test]
    fn the_overlay_script_switches_the_palette_tabs() {
        let rendered = page("Home", &signed_in("someone@example.com"), &html! {}).into_string();
        // Clicking a tab moves the active style and swaps the two sections
        // through `hidden`, the same mechanism as the drawer itself.
        assert!(rendered.contains(".tabs button"));
        assert!(rendered.contains("classList.toggle('active', t === tab)"));
        assert!(rendered.contains("'windows-tab').hidden = !windows;"));
        assert!(rendered.contains("'debug-tab').hidden = windows;"));
        // The active style is real CSS, not just a class name.
        assert!(rendered.contains("#palette .tabs button.active"));
    }

    #[test]
    fn an_anonymous_page_carries_no_palette_toasts_or_overlay() {
        let rendered = page("Home", &Chrome::anonymous(), &html! {}).into_string();
        assert!(!rendered.contains("id=\"palette\""));
        assert!(!rendered.contains("id=\"debug-toggle\""));
        assert!(!rendered.contains("id=\"debug-content\""));
        assert!(!rendered.contains("id=\"toasts\""));
        assert!(!rendered.contains("id=\"toast-offline\""));
    }

    #[test]
    fn a_signed_in_viewer_signs_out_with_a_post_not_a_link() {
        // An anchor only ever issues a GET; sign-out revokes the session and
        // must not be reachable that way, so the control has to be a form.
        let rendered = header(&Viewer::SignedIn {
            email: "someone@example.com".to_owned(),
        })
        .into_string();
        let tag = opening_tag_containing(&rendered, "/auth/logout");
        assert!(tag.starts_with("<form"));
        assert!(tag.contains(r#"method="post""#));
        assert!(tag.contains(r#"action="/auth/logout""#));
    }

    #[test]
    fn an_anonymous_viewer_is_offered_sign_in() {
        let rendered = header(&Viewer::Anonymous).into_string();
        assert!(rendered.contains("/auth/login"));
        assert!(!rendered.contains("/auth/logout"));
    }

    #[test]
    fn a_signed_in_viewer_is_offered_sign_out() {
        let rendered = header(&Viewer::SignedIn {
            email: "someone@example.com".to_owned(),
        })
        .into_string();
        assert!(rendered.contains("someone@example.com"));
        assert!(rendered.contains("/auth/logout"));
        assert!(!rendered.contains("/auth/login"));
    }

    #[test]
    fn the_header_nav_lays_out_with_the_shared_layout_classes() {
        // The nav opens every header, so its tag is the whole output's prefix.
        let anonymous = header(&Viewer::Anonymous).into_string();
        assert!(anonymous.starts_with(r#"<nav class="flex gap-md border-b">"#));
    }

    #[test]
    fn header_identity_and_sign_out_carry_their_visual_classes() {
        let rendered = header(&Viewer::SignedIn {
            email: "someone@example.com".to_owned(),
        })
        .into_string();
        assert_eq!(
            opening_tag_containing(&rendered, "viewer-email"),
            r#"<span class="viewer-email">"#
        );
        let form_tag = opening_tag_containing(&rendered, "/auth/logout");
        assert!(form_tag.contains(r#"class="sign-out""#));
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

    #[test]
    fn a_page_carries_exactly_one_ask_debug_element() {
        let rendered = page("Home", &signed_in("someone@example.com"), &html! {}).into_string();
        assert_eq!(rendered.matches("id=\"ask-debug\"").count(), 1);
    }

    #[test]
    fn the_page_s_ask_debug_element_starts_empty_and_carries_no_oob_swap() {
        let rendered = page("Home", &signed_in("someone@example.com"), &html! {}).into_string();
        let tag = opening_tag_containing(&rendered, "id=\"ask-debug\"");
        assert!(tag.contains("type=\"application/json\""));
        assert!(!tag.contains("hx-swap-oob"));
        let after = &rendered[rendered.find(tag).unwrap() + tag.len()..];
        assert!(after.trim_start().starts_with("</script>"));
    }

    #[test]
    fn a_chrome_carrying_debug_json_fills_the_ask_debug_element() {
        // A reopened window arrives already carrying its answer's payload,
        // so the Debug tab has something to show before any ask runs.
        let mut chrome = signed_in("someone@example.com");
        chrome.debug_json = Some("{\"request\":\"open tasks\"}".to_owned());
        let rendered = page("Home", &chrome, &html! {}).into_string();
        assert!(rendered.contains("{\"request\":\"open tasks\"}"));
    }

    #[test]
    fn debug_payload_replaces_ask_debug_out_of_band() {
        let rendered = debug_payload(&outcome(Verdict::Answered {
            html: "<ul></ul>".into(),
        }))
        .into_string();
        let tag = opening_tag_containing(&rendered, "id=\"ask-debug\"");
        assert!(tag.contains("type=\"application/json\""));
        assert!(tag.contains("hx-swap-oob=\"outerHTML\""));
    }

    #[test]
    fn debug_payload_carries_valid_json_for_an_answered_outcome() {
        let rendered = debug_payload(&outcome(Verdict::Answered {
            html: "<ul></ul>".into(),
        }))
        .into_string();
        let start = rendered.find('>').unwrap() + 1;
        let end = rendered.rfind("</script>").unwrap();
        let value: serde_json::Value = serde_json::from_str(&rendered[start..end]).unwrap();
        assert_eq!(value["request"], "open tasks");
        assert!(value["failed_stage"].is_null());
    }

    #[test]
    fn debug_payload_carries_valid_json_for_a_refused_outcome() {
        let rendered = debug_payload(&outcome(Verdict::Failed {
            stage: Stage::Query,
        }))
        .into_string();
        let start = rendered.find('>').unwrap() + 1;
        let end = rendered.rfind("</script>").unwrap();
        let value: serde_json::Value = serde_json::from_str(&rendered[start..end]).unwrap();
        assert_eq!(value["request"], "open tasks");
        assert_eq!(value["failed_stage"], "query");
    }

    #[test]
    fn toasts_is_an_empty_polite_live_region() {
        let rendered = toasts().into_string();
        assert!(rendered.contains(r#"id="toasts""#));
        assert!(rendered.contains(r#"aria-live="polite""#));
        // Empty: a page starts with no failure to announce.
        assert!(
            rendered.contains(r#"<div class="toast-stack" id="toasts" aria-live="polite"></div>"#)
        );
    }

    #[test]
    fn a_toast_carries_its_message_and_a_labelled_dismiss_button() {
        let rendered = toast("noal could not run the query it wrote.").into_string();
        assert!(rendered.contains("noal could not run the query it wrote."));
        let tag = opening_tag_containing(&rendered, "toast-dismiss");
        assert!(tag.starts_with("<button"));
        assert!(tag.contains(r#"type="button""#));
        assert!(tag.contains("aria-label="));
    }

    #[test]
    fn a_toast_s_dismiss_button_is_not_the_whole_toast() {
        // The dismiss control must sit inside the toast, not be the toast
        // itself — otherwise selecting the message text would dismiss it,
        // and a screen reader would read the whole message as a button.
        let rendered = toast("noal could not run the query it wrote.").into_string();
        let toast_tag = opening_tag_containing(&rendered, r#"class="toast card flex""#);
        assert_eq!(toast_tag, r#"<div class="toast card flex">"#);
    }

    #[test]
    fn a_toast_s_dismiss_button_takes_the_ghost_button_pair() {
        let rendered = toast("noal could not run the query it wrote.").into_string();
        let tag = opening_tag_containing(&rendered, "toast-dismiss");
        assert!(tag.contains(r#"class="toast-dismiss btn btn-ghost""#));
    }

    #[test]
    fn the_toasts_region_carries_its_stack_layout_in_a_class() {
        assert!(
            super::STYLE.contains(".toast-stack { display: grid; gap: .5rem; max-width: 24rem; }")
        );
    }

    #[test]
    fn a_signed_in_page_carries_the_toast_region_as_a_palette_sibling() {
        let rendered = page("Home", &signed_in("someone@example.com"), &html! {}).into_string();
        let palette_at = rendered.find(r#"id="palette""#).unwrap();
        let toasts_at = rendered.find(r#"id="toasts""#).unwrap();
        let palette_close = rendered[palette_at..].find("</aside>").unwrap() + palette_at;
        // #toasts appears only once #palette's own closing tag has passed,
        // proving it is a sibling rather than nested inside it.
        assert!(toasts_at > palette_close);
    }

    #[test]
    fn a_signed_in_page_carries_a_hidden_offline_toast_template() {
        let rendered = page("Home", &signed_in("someone@example.com"), &html! {}).into_string();
        assert!(rendered.contains(r#"<template id="toast-offline">"#));
        // The wording comes from the one function every other toast reads
        // from, not a second copy typed into the template.
        assert!(rendered.contains(OFFLINE_MESSAGE));
        assert!(rendered.contains(r#"class="toast card flex""#));
    }

    // The palette's runtime behaviour — opening, closing, focus, tab
    // switching, and surviving typed text — happens inside
    // `OVERLAY_SCRIPT`'s JavaScript, which the Rust toolchain never executes.
    // These tests pin the source text of the pieces that behaviour depends
    // on, so a careless edit is caught even though the behaviour itself is
    // not.

    #[test]
    fn the_overlay_script_guards_a_missing_palette() {
        assert!(super::OVERLAY_SCRIPT.contains("if (panel)"));
    }

    #[test]
    fn the_overlay_script_reads_the_platform_for_the_toggle_title() {
        assert!(super::OVERLAY_SCRIPT.contains("navigator.platform"));
    }

    #[test]
    fn the_overlay_script_matches_the_command_or_control_k_chord_only() {
        let script = super::OVERLAY_SCRIPT;
        assert!(script.contains("event.metaKey || event.ctrlKey"));
        assert!(script.contains("!event.shiftKey"));
        assert!(script.contains("!event.altKey"));
        assert!(script.contains("event.key.toLowerCase() === 'k'"));
        assert!(script.contains("event.preventDefault()"));
    }

    #[test]
    fn the_overlay_script_closes_on_escape() {
        assert!(super::OVERLAY_SCRIPT.contains("event.key === 'Escape'"));
    }

    #[test]
    fn the_overlay_script_looks_the_toast_region_up_at_event_time() {
        // The region sits after this script in the document, so binding to
        // it at load would run on null and kill every later listener. The
        // Escape branch must look it up when the key lands.
        let script = super::OVERLAY_SCRIPT;
        assert!(!script.contains("var toasts = document.getElementById('toasts');\n    var toggle"));
        let escape_at = script.find("event.key === 'Escape'").unwrap();
        let after_escape = &script[escape_at..];
        let lookup_at = after_escape.find("getElementById('toasts')").unwrap();
        let use_at = after_escape.find("lastElementChild").unwrap();
        assert!(lookup_at < use_at);
    }

    #[test]
    fn the_overlay_script_delegates_the_toast_dismiss_click() {
        let script = super::OVERLAY_SCRIPT;
        // The dismiss click is caught on the palette, which exists when the
        // script runs; the toast region itself sits after the script in the
        // document and is looked up at event time.
        assert!(script.contains("panel.addEventListener('click'"));
        assert!(script.contains("closest('.toast-dismiss')"));
    }

    #[test]
    fn the_overlay_script_opens_the_drawer_focused_and_lands_on_the_current_row() {
        let script = super::OVERLAY_SCRIPT;
        // Opening focuses the ask input, so ⌘K is ask-ready...
        assert!(script.contains("if (input) input.focus();"));
        // ...and scrolls the viewer's current window row into view.
        assert!(script.contains("getElementById('window-current')"));
        assert!(script.contains("scrollIntoView({ block: 'center' })"));
    }

    #[test]
    fn the_overlay_script_never_reopens_the_palette_from_the_answered_event() {
        // Setting `hidden` directly, rather than calling `togglePalette()`,
        // is what stops this listener from reopening a palette the viewer
        // had already closed themselves before their answer came back.
        let script = super::OVERLAY_SCRIPT;
        let answered_at = script.find("noal:answered").unwrap();
        let listener_body = &script[answered_at..];
        assert!(!listener_body.contains("togglePalette()"));
    }

    #[test]
    fn the_style_still_hides_a_hidden_palette() {
        assert!(super::STYLE.contains("#palette[hidden] { display: none; }"));
    }

    /// Pull a CSS block out of [`STYLE`], from its opener through its
    /// closing brace.
    ///
    /// The stylesheet nests blocks (`@media` wraps an inner `:root`), so a
    /// bare substring search cannot tell where one block ends; this walks
    /// the braces instead. The first matching opener wins, and the themes
    /// are ordered deliberately — see the ordering assertions below.
    fn css_block<'a>(source: &'a str, opener: &str) -> Option<&'a str> {
        let start = source.find(opener)?;
        let open = source[start..].find('{')? + start;
        let mut depth = 1usize;
        for (offset, byte) in source[open + 1..].bytes().enumerate() {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&source[start..open + 2 + offset]);
                    }
                }
                _ => {}
            }
        }
        None
    }

    #[test]
    fn the_style_defines_the_light_theme_variables_on_root() {
        let root = css_block(super::STYLE, ":root").unwrap();
        assert!(root.contains("--bg: #fafafa;"));
        assert!(root.contains("--fg: #09090b;"));
        assert!(root.contains("--border: #e4e4e7;"));
        assert!(root.contains("--primary: #18181b;"));
        assert!(root.contains("--muted: #71717a;"));
        assert!(root.contains("--radius: 0.5rem;"));
        assert!(root.contains("--font: 'Inter', system-ui, sans-serif;"));
    }

    #[test]
    fn the_style_overrides_the_theme_variables_for_a_dark_system_preference() {
        let dark = css_block(super::STYLE, "@media (prefers-color-scheme: dark)").unwrap();
        assert!(dark.contains("--bg: #09090b;"));
        assert!(dark.contains("--fg: #fafafa;"));
        assert!(dark.contains("--border: #27272a;"));
        assert!(dark.contains("--primary: #fafafa;"));
        assert!(dark.contains("--muted: #a1a1aa;"));
    }

    #[test]
    fn the_light_theme_precedes_its_dark_override_in_source_order() {
        // Both theme rules target `:root` at equal specificity, so when the
        // viewer's system prefers dark, whichever declaration comes last
        // wins. The override must therefore follow the light variables.
        let light_at = super::STYLE.find(":root").unwrap();
        let dark_at = super::STYLE.find("(prefers-color-scheme: dark)").unwrap();
        assert!(light_at < dark_at);
    }

    #[test]
    fn the_palette_carries_its_own_dark_zone_off_the_theme_variables() {
        let palette = css_block(super::STYLE, "#palette {").unwrap();
        assert!(palette.contains("background: #09090b;"));
        assert!(palette.contains("color: #fafafa;"));
        assert!(palette.contains("--accent: #7dd3fc;"));
        assert!(palette.contains("--accent-hover: #bae6fd;"));
    }

    #[test]
    fn the_style_defines_component_classes_for_shared_views() {
        assert!(super::STYLE.contains(".btn {"));
        assert!(super::STYLE.contains(".btn-primary { background: var(--primary);"));
        assert!(super::STYLE.contains(".btn-ghost {"));
        assert!(super::STYLE.contains(".input {"));
        assert!(super::STYLE.contains(".card { border: 1px solid var(--border);"));
        assert!(super::STYLE.contains(".toast {"));
        assert!(super::STYLE.contains(".tab-active { color: #7dd3fc; border-bottom-color: #7dd3fc; }"));
        assert!(super::STYLE.contains(".tree-row:hover { background: #18181b; }"));
    }

    #[test]
    fn the_style_defines_layout_utilities() {
        assert!(super::STYLE.contains(".flex { display: flex; }"));
        assert!(super::STYLE.contains(".gap-sm { gap: .5rem; }"));
        assert!(super::STYLE.contains(".gap-md { gap: 1rem; }"));
        assert!(super::STYLE.contains(".mt-1 { margin-top: .25rem; }"));
        assert!(super::STYLE.contains(".sr-only { position: absolute; width: 1px; height: 1px;"));
    }

    #[test]
    fn the_style_shows_an_indicator_that_carries_the_request_class_itself() {
        // htmx adds `htmx-request` to the element `hx-indicator` names, not
        // to an ancestor of it, so a rule scoped to a shared element is what
        // actually shows an indicator named directly, as `#ask-busy` is.
        assert!(super::STYLE.contains(".htmx-request.htmx-indicator { display: inline; }"));
    }

    #[test]
    fn the_toast_rule_follows_the_card_rule_in_source_order() {
        // Both selectors land on a rendered toast at equal specificity, so
        // the toast's tighter padding wins only because its block comes
        // after .card's; reordering them silently widens every toast.
        let card_at = super::STYLE.find(".card {").unwrap();
        let toast_at = super::STYLE.find(".toast {").unwrap();
        assert!(card_at < toast_at);
    }

    #[test]
    fn the_toast_region_has_an_explicit_z_index_above_the_debug_chrome() {
        // #debug-toggle is the highest z-index otherwise in play, at 10.
        assert!(super::STYLE.contains("#toasts { position: fixed;"));
        assert!(super::STYLE.contains("z-index: 11;"));
    }

    #[test]
    fn the_overlay_script_registers_the_response_error_listener_before_the_palette_guard() {
        // A 401 must reach sign-in even on a page with no palette at all, so
        // this listener cannot live inside `if (panel)`.
        let script = super::OVERLAY_SCRIPT;
        let listener_at = script.find("htmx:responseError").unwrap();
        let guard_at = script.find("if (panel)").unwrap();
        assert!(listener_at < guard_at);
    }

    #[test]
    fn the_overlay_script_sends_a_401_to_sign_in_with_the_current_page_as_next() {
        let script = super::OVERLAY_SCRIPT;
        assert!(script.contains("xhr.status === 401"));
        assert!(script.contains("/auth/login?next="));
        assert!(script.contains("encodeURIComponent(location.pathname + location.search)"));
    }

    #[test]
    fn the_overlay_script_checks_401_before_appending_any_other_response_error() {
        let script = super::OVERLAY_SCRIPT;
        let status_at = script.find("xhr.status === 401").unwrap();
        let append_at = script.find("insertAdjacentHTML").unwrap();
        assert!(status_at < append_at);
    }

    #[test]
    fn the_overlay_script_appends_a_non_401_error_body_to_toasts() {
        let script = super::OVERLAY_SCRIPT;
        assert!(script.contains("document.getElementById('toasts')"));
        assert!(script.contains("toasts.insertAdjacentHTML('beforeend', xhr.responseText)"));
    }

    #[test]
    fn the_overlay_script_clones_the_offline_template_on_a_send_error() {
        let script = super::OVERLAY_SCRIPT;
        assert!(script.contains("htmx:sendError"));
        assert!(script.contains("getElementById('toast-offline')"));
        assert!(script.contains("template.content.cloneNode(true)"));
    }

    #[test]
    fn the_overlay_script_also_falls_back_to_the_offline_toast_on_an_empty_error_body() {
        // "any non-200 with an empty body" from the response-error listener
        // takes the same fallback path as a send error.
        let script = super::OVERLAY_SCRIPT;
        let response_error_at = script.find("htmx:responseError").unwrap();
        let send_error_at = script.find("htmx:sendError").unwrap();
        let between = &script[response_error_at..send_error_at];
        assert!(between.contains("appendOfflineToast(toasts)"));
    }

    // The rename editor's runtime behaviour is JavaScript the toolchain
    // never executes; these pin the source text it depends on.

    #[test]
    fn the_overlay_script_opens_one_rename_at_a_time_by_unhiding_its_form() {
        let script = super::OVERLAY_SCRIPT;
        // The opener unhides the form beside it and hides the row's label
        // link, rather than fetching anything — no GET route serves the
        // form.
        assert!(script.contains(".window-rename-open"));
        assert!(script.contains("form.hidden = false;"));
        assert!(script.contains(".window-label"));
        assert!(script.contains("label.hidden = true;"));
        assert!(script.contains("field.focus(); field.select();"));
        // Opening a second editor puts away the first.
        let open_at = script.find("function (event) {").unwrap();
        let put_at = script[open_at..].find("putAwayRename();").unwrap();
        assert!(put_at > 0);
    }

    #[test]
    fn the_overlay_script_cancels_a_rename_back_to_the_stored_name() {
        // Putting an edit away hides the form and restores the label, and
        // resets the field to its stored default — so reopening after
        // Cancel or Escape starts from the stored name, never the text the
        // viewer typed into the closed editor.
        let script = super::OVERLAY_SCRIPT;
        let start = script.find("function putAwayRename()").unwrap();
        let body = &script[start..start + 400];
        assert!(body.contains("hidden = true;"));
        assert!(body.contains("field.value = field.defaultValue;"));
    }

    #[test]
    fn the_escape_order_is_rename_then_newest_toast_then_palette() {
        let script = super::OVERLAY_SCRIPT;
        let escape_at = script.find("event.key === 'Escape'").unwrap();
        let rename_at = script[escape_at..].find("if (openRename)").unwrap() + escape_at;
        let toast_at = script[escape_at..].find("var newestToast").unwrap() + escape_at;
        let palette_at = script[toast_at..].find("togglePalette();").unwrap() + toast_at;
        assert!(rename_at < toast_at);
        assert!(toast_at < palette_at);
    }

    #[test]
    fn the_rename_guard_yields_escape_only_while_an_edit_is_open() {
        // The typing-elsewhere guard would swallow keys typed into the
        // rename input; Escape must still cancel through it, but only when
        // an edit is actually open.
        let script = super::OVERLAY_SCRIPT;
        let guard_at = script.find("if (typingElsewhere").unwrap();
        let after_guard = &script[guard_at..];
        let line_end = after_guard.find('\n').unwrap();
        let condition = &after_guard[..line_end];
        assert!(condition.contains("typingElsewhere"));
        assert!(condition.contains("event.key === 'Escape'"));
        assert!(condition.contains("openRename"));
    }

    #[test]
    fn a_tree_swap_forgets_any_open_rename_editor() {
        // The fresh tree replaces the form being edited, so the tracked
        // reference must be dropped or Escape would reach for a detached
        // node while the palette stays open.
        let script = super::OVERLAY_SCRIPT;
        let reset_at = script.rfind("htmx:afterSwap").unwrap();
        let listener = &script[reset_at..reset_at + 400];
        assert!(listener.contains("openRename = null;"));
    }
}
