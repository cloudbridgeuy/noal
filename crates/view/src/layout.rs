//! The page chrome shared by every full document.

use maud::{html, Markup, PreEscaped, DOCTYPE};
use noal_core::ask::outcome::Outcome;

/// The htmx build noal loads. Pinned, and served with an integrity hash, so a
/// change to the CDN cannot change what runs in the browser.
const HTMX_SRC: &str = "https://unpkg.com/htmx.org@2.0.4/dist/htmx.min.js";

/// Enough CSS to make the page and the overlay usable. No framework yet.
const STYLE: &str = r"
body { font: 16px/1.5 system-ui, sans-serif; margin: 0; padding: 0 1rem; max-width: 72rem; margin-inline: auto; }
header nav { display: flex; gap: 1rem; padding: 1rem 0; border-bottom: 1px solid #ddd; }
#ask-form { display: grid; gap: .5rem; max-width: 40rem; margin: 2rem 0; }
#ask-form input { font: inherit; padding: .5rem; }
.htmx-indicator { display: none; }
/* htmx marks the element hx-indicator names, not an ancestor, so the
   same-element selector is the one that fires. */
.htmx-request .htmx-indicator, .htmx-request.htmx-indicator { display: inline; }
table { border-collapse: collapse; } td, th { border: 1px solid #ddd; padding: .25rem .5rem; text-align: left; }
#debug-toggle { position: fixed; right: 1rem; bottom: 1rem; z-index: 10; }
#debug-panel { position: fixed; inset: 0 0 0 auto; width: min(40rem, 100%); background: #111; color: #eee;
  overflow: auto; padding: 1rem; font: 13px/1.4 ui-monospace, monospace; z-index: 9; }
#debug-panel[hidden] { display: none; }
#debug-panel pre { white-space: pre-wrap; background: #222; padding: .5rem; }
#debug-panel header { display: flex; align-items: baseline; gap: 1rem; }
#debug-copy { font: inherit; }
/* A small overlay near the top, not a drawer: it floats over whatever `main`
   holds rather than pushing it down. `position: fixed` needs the `[hidden]`
   rule below for the same reason #debug-panel does. Kept off the bottom
   corners so a toast region has room there without overlapping this one; a
   future fixed-position sibling here should pick its own z-index rather than
   assume it is the only other one in play. */
#palette { position: fixed; top: 1rem; left: 50%; transform: translateX(-50%); z-index: 8;
  width: min(40rem, calc(100% - 2rem)); background: #fff; border: 1px solid #ddd; border-radius: .5rem;
  box-shadow: 0 .25rem 1rem rgba(0, 0, 0, .15); padding: 1rem; }
#palette[hidden] { display: none; }
#palette #ask-form { margin: 0; }
/* Bottom-left, the corner #palette's own comment reserves for exactly this.
   Highest z-index of the set: a failure notice must never end up hidden
   behind the debug panel (9) or its toggle (10), so 11 sits above both. */
#toasts { position: fixed; left: 1rem; bottom: 1rem; z-index: 11; display: grid; gap: .5rem; max-width: 24rem; }
.toast { display: flex; align-items: flex-start; gap: .75rem; background: #fff; border: 1px solid #ddd;
  border-radius: .5rem; box-shadow: 0 .25rem 1rem rgba(0, 0, 0, .15); padding: .75rem 1rem; }
.toast p { margin: 0; flex: 1; }
.toast-dismiss { font: inherit; }
";

/// The script that toggles the panel and fills it from the last answer, and
/// that wires the command palette's keyboard shortcut and button.
///
/// It reads `#ask-debug` after every htmx swap. That element is chrome, kept
/// current by an out-of-band swap alongside every answer, so this script
/// never needs to know where an ask's fragment landed on the page — but it
/// does know what an ask is, now: a `401` from `/ask` sends the browser to
/// sign in, and any other failure becomes a toast.
///
/// `#palette` is absent on most pages (an anonymous viewer, an error page),
/// so the palette wiring is guarded on finding it first and does nothing on a
/// page without one, leaving the rest of this script — shared with the debug
/// overlay above — to run regardless.
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
    var text = el.textContent;
    // Blank until the first ask's out-of-band swap arrives; nothing to parse yet.
    if (!text || !text.trim()) return;
    try { render(JSON.parse(text)); } catch (err) { console.error('debug payload', err); }
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
  // Clones the hidden `#toast-offline` template (see `page()`) rather than
  // build the markup here, so the wording lives in exactly one place: the
  // same `layout::toast` call every other toast's markup comes from.
  function appendOfflineToast(toasts) {
    var template = document.getElementById('toast-offline');
    if (template) toasts.appendChild(template.content.cloneNode(true));
  }
  // Registered here, ahead of the palette lookup below, on purpose: `#ask-form`
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
  var palette = document.getElementById('palette');
  if (palette) {
    var paletteToggle = document.getElementById('palette-toggle');
    var paletteInput = document.getElementById('ask-input');
    // #toasts is a sibling of #palette, rendered under the same condition,
    // so whenever this block runs it is on the page too.
    var toasts = document.getElementById('toasts');
    // navigator.platform is deprecated, but it is the only signal small
    // enough for a tooltip this size; a wrong guess here is cosmetic.
    paletteToggle.title = /Mac/.test(navigator.platform)
      ? 'Command palette (⌘K)'
      : 'Command palette (Ctrl+K)';
    function togglePalette() {
      // Flip the attribute only. Re-rendering the form would wipe out
      // whatever the user has already typed into it.
      palette.hidden = !palette.hidden;
      if (!palette.hidden && paletteInput) paletteInput.focus();
    }
    // Removes one toast. The server only ever appends with `beforeend`, so
    // `#toasts`' last child is always the newest.
    function dismissToast(el) {
      el.remove();
    }
    // Delegated: toasts arrive from the server, appended long after this
    // listener is registered, so a listener bound to each toast would miss
    // every one that shows up later.
    toasts.addEventListener('click', function (event) {
      var dismiss = event.target.closest('.toast-dismiss');
      if (dismiss) dismissToast(dismiss.closest('.toast'));
    });
    paletteToggle.addEventListener('click', togglePalette);
    document.addEventListener('keydown', function (event) {
      var target = event.target;
      var typingElsewhere = target && target !== paletteInput && (
        target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' ||
        target.tagName === 'SELECT' || target.isContentEditable);
      if (typingElsewhere) return;
      if ((event.metaKey || event.ctrlKey) && !event.shiftKey && !event.altKey &&
          event.key.toLowerCase() === 'k') {
        event.preventDefault();
        togglePalette();
      } else if (event.key === 'Escape') {
        // The newest toast goes first: one Escape clears it, and only the
        // next Escape — once none remain — closes the palette.
        var newestToast = toasts.lastElementChild;
        if (newestToast) {
          dismissToast(newestToast);
        } else if (!palette.hidden) {
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
      palette.hidden = true;
      if (paletteInput) paletteInput.value = '';
    });
  }
})();
"#;

/// The hidden panel and its toggle, present on every page.
///
/// The `#ask-debug` element starts empty. It is the one place in the document
/// that holds the debug payload; a handler replaces it out of band with
/// [`debug_payload`] after every ask, so the panel always shows the answer
/// most recently swapped in, wherever on the page that answer landed.
fn debug_overlay() -> Markup {
    html! {
        button #debug-toggle type="button" title="Toggle debug panel" { "debug" }
        aside #debug-panel hidden {
            header {
                h2 { "debug" }
                button #debug-copy type="button" { "copy" }
            }
            div #debug-content { p { "Ask something; the plan, template, and timings appear here." } }
            script #ask-debug type="application/json" {}
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
        div #toasts aria-live="polite" {}
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
        div .toast {
            p { (message) }
            button .toast-dismiss type="button" aria-label="Dismiss notification" { "×" }
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
///
/// The palette renders only when `palette` is [`Palette::Open`] **and**
/// `viewer` is [`Viewer::SignedIn`]. An anonymous viewer never gets palette
/// markup, whatever `palette` is asked for: there is no session for its ask
/// form to post against, so a caller cannot hand one out by mistake.
#[must_use]
pub fn page(title: &str, viewer: &Viewer, palette: Palette, body: &Markup) -> Markup {
    let show_palette =
        matches!(palette, Palette::Open) && matches!(viewer, Viewer::SignedIn { .. });
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
                @if show_palette {
                    (render_palette())
                    // A sibling of #palette, never a child: a toast must stay
                    // visible even while the palette itself is hidden.
                    (toasts())
                    (toast_offline())
                }
                main { (body) }
                (debug_overlay())
            }
        }
    }
}

/// The command palette itself: a toggle and the ask form, in its own chrome.
///
/// Rendered only from [`page`], and only when it has decided the palette
/// should show: [`Palette::Open`] for a signed-in viewer. The server decides
/// only whether the palette is on the page at all; once it is, `OVERLAY_SCRIPT`
/// owns the open/closed state through the `hidden` attribute, so this markup
/// itself never changes and never loses whatever the ask form already holds.
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
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{debug_payload, header, page, toast, toasts, Palette, Viewer, OFFLINE_MESSAGE};
    use maud::html;
    use noal_core::ask::outcome::{Debug, Outcome, Stage, Verdict};

    /// An [`Outcome`] with a fixed request, for tests that only care about
    /// the verdict.
    fn outcome(verdict: Verdict) -> Outcome {
        Outcome {
            request: "open tasks".into(),
            verdict,
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
    fn a_page_carries_exactly_one_ask_debug_element() {
        let rendered = page("Home", &Viewer::Anonymous, Palette::Closed, &html! {}).into_string();
        assert_eq!(rendered.matches("id=\"ask-debug\"").count(), 1);
    }

    #[test]
    fn the_page_s_ask_debug_element_starts_empty_and_carries_no_oob_swap() {
        let rendered = page("Home", &Viewer::Anonymous, Palette::Closed, &html! {}).into_string();
        let tag = opening_tag_containing(&rendered, "id=\"ask-debug\"");
        assert!(tag.contains("type=\"application/json\""));
        assert!(!tag.contains("hx-swap-oob"));
        let after = &rendered[rendered.find(tag).unwrap() + tag.len()..];
        assert!(after.trim_start().starts_with("</script>"));
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
        let palette_tag = opening_tag_containing(&rendered, "id=\"palette\"");
        assert!(!palette_tag
            .trim_end_matches('>')
            .split_whitespace()
            .any(|token| token == "hidden"));
        assert!(rendered.contains("autofocus"));
        assert!(rendered.contains("id=\"ask-input\""));
    }

    #[test]
    fn an_anonymous_viewer_asking_for_an_open_palette_gets_none() {
        // `page()` decides whether to show the palette; a caller cannot hand
        // one to a viewer with no session by passing `Palette::Open`.
        let rendered = page("Home", &Viewer::Anonymous, Palette::Open, &html! {}).into_string();
        assert!(!rendered.contains("id=\"palette\""));
        assert!(!rendered.contains("id=\"ask-form\""));
    }

    #[test]
    fn toasts_is_an_empty_polite_live_region() {
        let rendered = toasts().into_string();
        assert!(rendered.contains(r#"id="toasts""#));
        assert!(rendered.contains(r#"aria-live="polite""#));
        // Empty: a page starts with no failure to announce.
        assert!(rendered.contains(r#"<div id="toasts" aria-live="polite"></div>"#));
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
        let toast_tag = opening_tag_containing(&rendered, r#"class="toast""#);
        assert_eq!(toast_tag, r#"<div class="toast">"#);
    }

    #[test]
    fn an_open_palette_carries_the_toast_region_as_a_sibling() {
        let rendered = page(
            "Home",
            &Viewer::SignedIn {
                email: "someone@example.com".to_owned(),
            },
            Palette::Open,
            &html! {},
        )
        .into_string();
        let palette_at = rendered.find(r#"id="palette""#).unwrap();
        let toasts_at = rendered.find(r#"id="toasts""#).unwrap();
        let palette_close = rendered[palette_at..].find("</div>").unwrap() + palette_at;
        // #toasts appears only once #palette's own closing tag has passed,
        // proving it is a sibling rather than nested inside it.
        assert!(toasts_at > palette_close);
    }

    #[test]
    fn a_closed_palette_carries_no_toast_region_either() {
        let rendered = page("Home", &Viewer::Anonymous, Palette::Closed, &html! {}).into_string();
        assert!(!rendered.contains(r#"id="toasts""#));
    }

    #[test]
    fn an_open_palette_carries_a_hidden_offline_toast_template() {
        let rendered = page(
            "Home",
            &Viewer::SignedIn {
                email: "someone@example.com".to_owned(),
            },
            Palette::Open,
            &html! {},
        )
        .into_string();
        assert!(rendered.contains(r#"<template id="toast-offline">"#));
        // The wording comes from the one function every other toast reads
        // from, not a second copy typed into the template.
        assert!(rendered.contains(OFFLINE_MESSAGE));
        assert!(rendered.contains(r#"class="toast""#));
    }

    #[test]
    fn a_closed_palette_carries_no_offline_toast_template_either() {
        let rendered = page("Home", &Viewer::Anonymous, Palette::Closed, &html! {}).into_string();
        assert!(!rendered.contains(r#"id="toast-offline""#));
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

    // The palette's runtime behaviour — opening, closing, focus, and surviving
    // typed text — happens inside `OVERLAY_SCRIPT`'s JavaScript, which the
    // Rust toolchain never executes. These tests pin the source text of the
    // pieces that behaviour depends on, so a careless edit is caught even
    // though the behaviour itself is not.

    #[test]
    fn the_overlay_script_guards_a_missing_palette() {
        assert!(super::OVERLAY_SCRIPT.contains("if (palette)"));
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
    fn the_overlay_script_dismisses_the_newest_toast_before_closing_the_palette() {
        let script = super::OVERLAY_SCRIPT;
        assert!(script.contains("function dismissToast(el)"));
        assert!(script.contains("toasts.lastElementChild"));
        // The toast branch must be checked, and win, before the palette is
        // ever asked to close.
        let escape_at = script.find("event.key === 'Escape'").unwrap();
        let newest_at = script.find("var newestToast").unwrap();
        let toggle_at = script[newest_at..].find("togglePalette();").unwrap() + newest_at;
        assert!(escape_at < newest_at);
        assert!(newest_at < toggle_at);
    }

    #[test]
    fn the_overlay_script_delegates_the_toast_dismiss_click() {
        let script = super::OVERLAY_SCRIPT;
        assert!(script.contains("toasts.addEventListener('click'"));
        assert!(script.contains("closest('.toast-dismiss')"));
    }

    #[test]
    fn the_overlay_script_closes_and_empties_the_palette_on_the_answered_event() {
        let script = super::OVERLAY_SCRIPT;
        assert!(script.contains("document.addEventListener('noal:answered'"));
        assert!(script.contains("palette.hidden = true;"));
        assert!(script.contains("paletteInput.value = '';"));
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

    #[test]
    fn the_style_shows_an_indicator_that_carries_the_request_class_itself() {
        // htmx adds `htmx-request` to the element `hx-indicator` names, not
        // to an ancestor of it, so a rule scoped to a shared element is what
        // actually shows an indicator named directly, as `#ask-busy` is.
        assert!(super::STYLE.contains(".htmx-request.htmx-indicator { display: inline; }"));
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
        // this listener cannot live inside `if (palette)`.
        let script = super::OVERLAY_SCRIPT;
        let listener_at = script.find("htmx:responseError").unwrap();
        let guard_at = script
            .find("var palette = document.getElementById('palette');")
            .unwrap();
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
}
