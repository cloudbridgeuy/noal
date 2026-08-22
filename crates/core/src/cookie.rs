//! Cookie header parsing and building.
//!
//! Two pure functions bracket the shell's cookie work: one reads a `Cookie`
//! request header, the other writes a `Set-Cookie` response header. Keeping the
//! attribute string in one place stops a handler from writing a session cookie
//! that forgets `HttpOnly` or `Secure`.

/// How long a sealed session cookie lives in the browser, in seconds.
///
/// The cookie carries its own expiry inside the seal, so this only controls
/// when the browser stops sending it. Fourteen days matches the WorkOS refresh
/// token lifetime.
pub const MAX_AGE_SECONDS: i64 = 60 * 60 * 24 * 14;

/// How long a short-lived cookie lives, in seconds.
///
/// Used for the OAuth state cookie, which only has to survive the round trip
/// through the WorkOS sign-in screen.
pub const BRIEF_MAX_AGE_SECONDS: i64 = 60 * 10;

/// Read one cookie's value out of a `Cookie` request header.
///
/// Returns `None` when the header holds no cookie by that name. The value is
/// returned as it appeared; this function does not URL-decode, because noal
/// only stores base64url text, which needs no decoding.
#[must_use]
pub fn read<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header.split(';').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key.trim() == name).then(|| value.trim())
    })
}

/// Build the `Set-Cookie` value that stores `value` under `name`.
///
/// The attributes are fixed and not optional. `__Host-` prefixed names require
/// `Secure`, `Path=/`, and no `Domain`, so a caller cannot weaken them without
/// the browser rejecting the cookie outright.
#[must_use]
pub fn write(name: &str, value: &str) -> String {
    write_for(name, value, MAX_AGE_SECONDS)
}

/// Build the `Set-Cookie` value that stores `value` for `max_age_seconds`.
///
/// Same fixed attributes as [`write()`]; only the lifetime differs.
#[must_use]
pub fn write_for(name: &str, value: &str, max_age_seconds: i64) -> String {
    format!("{name}={value}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age={max_age_seconds}")
}

/// Build the `Set-Cookie` value that removes `name` from the browser.
///
/// The attributes must match the ones used to write it, or the browser keeps
/// the original cookie and stores a second, empty one.
#[must_use]
pub fn clear(name: &str) -> String {
    format!("{name}=; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=0")
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{clear, read, write, write_for, BRIEF_MAX_AGE_SECONDS};

    #[test]
    fn reads_the_only_cookie() {
        assert_eq!(read("session=abc", "session"), Some("abc"));
    }

    #[test]
    fn reads_a_cookie_among_others() {
        let header = "theme=dark; session=abc; locale=en";
        assert_eq!(read(header, "session"), Some("abc"));
    }

    #[test]
    fn ignores_the_space_after_a_semicolon() {
        assert_eq!(read("a=1;   session=abc", "session"), Some("abc"));
    }

    #[test]
    fn returns_none_for_a_missing_cookie() {
        assert_eq!(read("theme=dark", "session"), None);
    }

    #[test]
    fn returns_none_for_an_empty_header() {
        assert_eq!(read("", "session"), None);
    }

    #[test]
    fn does_not_match_a_name_by_prefix() {
        assert_eq!(read("session_id=abc", "session"), None);
    }

    #[test]
    fn keeps_base64url_padding_characters_in_the_value() {
        assert_eq!(read("session=a-b_c", "session"), Some("a-b_c"));
    }

    #[test]
    fn written_cookie_carries_every_required_attribute() {
        let header = write("__Host-noal_session", "sealed");
        assert!(header.starts_with("__Host-noal_session=sealed;"));
        assert!(header.contains("Path=/"));
        assert!(header.contains("HttpOnly"));
        assert!(header.contains("Secure"));
        assert!(header.contains("SameSite=Lax"));
        assert!(!header.contains("Domain"));
    }

    #[test]
    fn cleared_cookie_expires_at_once() {
        let header = clear("__Host-noal_session");
        assert!(header.contains("Max-Age=0"));
        assert!(header.starts_with("__Host-noal_session=;"));
    }

    #[test]
    fn a_brief_cookie_carries_the_shorter_lifetime() {
        let header = write_for("__Host-noal_oauth_state", "nonce", BRIEF_MAX_AGE_SECONDS);
        assert!(header.contains("Max-Age=600"));
        assert!(header.contains("HttpOnly"));
    }

    #[test]
    fn a_written_cookie_reads_back() {
        let header = write("session", "sealed");
        let sent = header.split(';').next().unwrap();
        assert_eq!(read(sent, "session"), Some("sealed"));
    }
}
