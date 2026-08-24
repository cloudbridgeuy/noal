//! Reading and judging `.dev.vars`.
//!
//! `.dev.vars` is the one file where a local checkout keeps its configuration,
//! because it is the same file `wrangler dev` already loads. Every xtask
//! command that needs local settings comes here.
//!
//! The top half is a functional core: turning the file's text into key–value
//! pairs, deciding what is missing, judging the Hyperdrive URL's shape, and
//! merging the entries over a process environment are all pure functions over
//! injected values, so they are unit tested without touching a disk. The
//! bottom half is the shell: locating and reading the file.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The Hyperdrive connection-string variable wrangler 4 actually reads.
pub const HYPERDRIVE_NAME: &str = "CLOUDFLARE_HYPERDRIVE_LOCAL_CONNECTION_STRING_DB";

/// The spelling [`HYPERDRIVE_NAME`] had before wrangler 4, which wrangler 4
/// silently ignores.
pub const LEGACY_HYPERDRIVE_NAME: &str = "WRANGLER_HYPERDRIVE_LOCAL_CONNECTION_STRING_DB";

/// The variables a local run cannot serve traffic without.
///
/// The list is checked in order, so reported gaps appear in this order too.
const REQUIRED: [&str; 6] = [
    "SESSION_KEY",
    "WORKOS_CLIENT_ID",
    "WORKOS_API_KEY",
    "REDIRECT_URI",
    "ANTHROPIC_API_KEY",
    HYPERDRIVE_NAME,
];

// ---------------------------------------------------------------------------
// Functional core — pure types and logic, no input or output
// ---------------------------------------------------------------------------

/// Parse `KEY=VALUE` lines into pairs.
///
/// Blank lines and lines starting with `#` are comments. Surrounding
/// whitespace is trimmed from keys and values; whatever sits between them,
/// including spaces and `#`, belongs to the value. A line without an `=`
/// cannot hold a pair, so it is skipped rather than guessed at. When a key
/// appears twice, the later line wins, matching what re-reading the file top
/// to bottom would suggest.
#[must_use]
pub fn parse(content: &str) -> BTreeMap<String, String> {
    let mut vars = BTreeMap::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        vars.insert(key.to_owned(), value.trim().to_owned());
    }
    vars
}

/// One reason `.dev.vars` cannot support a local run yet.
///
/// Names are reported; values never are, because they are secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gap {
    /// The variable is absent from the file, or present but empty.
    Unset(String),
    /// Only the legacy Hyperdrive name holds a connection string, which means
    /// wrangler 4 would serve without a database.
    LegacyHyperdriveName,
}

impl std::fmt::Display for Gap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unset(name) => write!(f, "`{name}` is missing or empty"),
            Self::LegacyHyperdriveName => write!(
                f,
                "`{LEGACY_HYPERDRIVE_NAME}` is the legacy name and wrangler 4 \
                 ignores it; rename it to `{HYPERDRIVE_NAME}`"
            ),
        }
    }
}

/// Everything wrong with the variables for a local dev run.
///
/// All problems are reported in one pass, so fixing the file takes one round
/// trip instead of one per gap. An empty value counts as unset, because an
/// empty secret serves no one.
#[must_use]
pub fn gaps(vars: &BTreeMap<String, String>) -> Vec<Gap> {
    let mut problems = Vec::new();

    for name in REQUIRED {
        if !filled(vars, name) {
            problems.push(Gap::Unset(name.to_owned()));
        }
    }

    // Wrangler reads only the new name, so a legacy-only file would start a
    // server with no database behind it — worth stopping for.
    if filled(vars, LEGACY_HYPERDRIVE_NAME) && !filled(vars, HYPERDRIVE_NAME) {
        problems.push(Gap::LegacyHyperdriveName);
    }

    problems
}

/// Where a database URL can be found among the variables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DatabaseUrl {
    /// Under the current [`HYPERDRIVE_NAME`].
    Current(String),
    /// Only under [`LEGACY_HYPERDRIVE_NAME`]; usable, but the caller should
    /// say so, since the name is on its way out.
    Legacy(String),
    /// Nowhere.
    Absent,
}

/// Pick the database URL out of parsed variables.
///
/// The current name wins over the legacy one. A present-but-empty value does
/// not count as found.
#[must_use]
pub fn database_url(vars: &BTreeMap<String, String>) -> DatabaseUrl {
    if let Some(url) = filled_value(vars, HYPERDRIVE_NAME) {
        return DatabaseUrl::Current(url);
    }
    match filled_value(vars, LEGACY_HYPERDRIVE_NAME) {
        Some(url) => DatabaseUrl::Legacy(url),
        None => DatabaseUrl::Absent,
    }
}

/// The value of `name`, when present and non-empty.
fn filled_value(vars: &BTreeMap<String, String>, name: &str) -> Option<String> {
    vars.get(name).filter(|value| !value.is_empty()).cloned()
}

/// Whether `name` holds a non-empty value.
fn filled(vars: &BTreeMap<String, String>, name: &str) -> bool {
    filled_value(vars, name).is_some()
}

/// Merge the process environment with `.dev.vars` defaults.
///
/// The result is the process environment with one entry added per parsed
/// file variable — except where the process already exports a variable of
/// the same name, because an explicit shell export should beat a file
/// default, matching standard dotenv precedence.
#[must_use]
pub fn overlay(
    process: &BTreeMap<String, String>,
    file_vars: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut merged = process.clone();
    for (name, value) in file_vars {
        merged.entry(name.clone()).or_insert_with(|| value.clone());
    }
    merged
}

/// Why a Hyperdrive connection string cannot serve a local run.
///
/// Each variant can say plainly what to type instead. No variant ever
/// carries the offending value, because it may hold secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlProblem {
    /// The value has no `scheme://…` shape at all.
    NotAUrl,
    /// The scheme names something other than Postgres.
    WrongScheme(String),
    /// Nothing sits between the credentials and the path.
    MissingHost,
    /// The password component is missing or empty.
    MissingPassword,
}

/// The shape wrangler 4 insists on, which trust-auth Postgres does not.
const URL_SHAPE: &str = "postgres://user:anything@host:port/db";

impl std::fmt::Display for UrlProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAUrl => write!(
                f,
                "the connection string is not a URL; use the shape `{URL_SHAPE}`"
            ),
            Self::WrongScheme(scheme) => write!(
                f,
                "`{scheme}` is not a Postgres scheme; use the shape `{URL_SHAPE}`"
            ),
            Self::MissingHost => write!(
                f,
                "the connection string has no host; use the shape `{URL_SHAPE}`"
            ),
            Self::MissingPassword => write!(
                f,
                "the connection string has no password. Local Postgres under \
                 trust auth ignores it, but wrangler 4 requires the format, \
                 so use the shape `{URL_SHAPE}`"
            ),
        }
    }
}

/// Judge whether a connection string can serve as the local Hyperdrive URL.
///
/// Wrangler 4 reads this string from its environment and refuses to start
/// unless it parses as a Postgres URL with a host and a non-empty password —
/// even though local Postgres under trust auth never checks the password.
/// `Some` means preflight must stop; `None` means go.
#[must_use]
pub fn hyperdrive_url_problem(url: &str) -> Option<UrlProblem> {
    let Some((scheme, rest)) = url.split_once("://") else {
        return Some(UrlProblem::NotAUrl);
    };
    if !matches!(scheme, "postgres" | "postgresql") {
        return Some(UrlProblem::WrongScheme(scheme.to_owned()));
    }

    // The authority ends where the path, query, or fragment begins.
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // A raw `@` may sit inside a password, so the last one separates
    // credentials from host.
    let (userinfo, host) = authority.rsplit_once('@').unwrap_or(("", authority));

    if host.is_empty() || host.starts_with(':') {
        return Some(UrlProblem::MissingHost);
    }

    let holds_a_password =
        matches!(userinfo.split_once(':'), Some((_, password)) if !password.is_empty());
    if holds_a_password {
        None
    } else {
        Some(UrlProblem::MissingPassword)
    }
}

// ---------------------------------------------------------------------------
// Imperative shell — filesystem effects
// ---------------------------------------------------------------------------

/// The path of the file at the checkout root.
#[must_use]
pub fn path(root: &Path) -> PathBuf {
    root.join(".dev.vars")
}

/// Read the file's contents from the checkout root.
///
/// # Errors
///
/// Returns the underlying error, including "not found"; callers decide how a
/// missing file should be explained, because migrate and dev word it
/// differently.
pub fn read(root: &Path) -> std::io::Result<String> {
    fs::read_to_string(path(root))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{
        database_url, gaps, hyperdrive_url_problem, overlay, parse, DatabaseUrl, Gap,
        HYPERDRIVE_NAME, LEGACY_HYPERDRIVE_NAME, REQUIRED,
    };
    use std::collections::BTreeMap;

    fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn all_set() -> BTreeMap<String, String> {
        vars(
            REQUIRED
                .iter()
                .map(|name| (*name, "x"))
                .collect::<Vec<_>>()
                .as_slice(),
        )
    }

    #[test]
    fn parses_a_well_formed_line() {
        let parsed = parse("KEY=value\n");
        assert_eq!(parsed.get("KEY").unwrap(), "value");
    }

    #[test]
    fn blank_lines_are_ignored() {
        assert!(parse("\n   \n\t\n").is_empty());
    }

    #[test]
    fn comment_lines_are_ignored() {
        assert!(parse("# a comment\n  # indented comment\n").is_empty());
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        let parsed = parse("  KEY  =  value  \n");
        assert_eq!(parsed.get("KEY").unwrap(), "value");
    }

    #[test]
    fn inner_spaces_stay_in_the_value() {
        let parsed = parse("URL=postgres://user:pass word@localhost/db\n");
        assert_eq!(
            parsed.get("URL").unwrap(),
            "postgres://user:pass word@localhost/db"
        );
    }

    #[test]
    fn hash_inside_a_value_is_kept() {
        let parsed = parse("SECRET=abc#not-a-comment\n");
        assert_eq!(parsed.get("SECRET").unwrap(), "abc#not-a-comment");
    }

    #[test]
    fn a_line_without_a_separator_is_skipped() {
        assert!(parse("NOT_A_PAIR\n").is_empty());
    }

    #[test]
    fn a_key_without_a_name_is_skipped() {
        assert!(parse("=value\n").is_empty());
    }

    #[test]
    fn an_empty_value_is_parsed_as_empty() {
        let parsed = parse("EMPTY=\n");
        assert_eq!(parsed.get("EMPTY").unwrap(), "");
    }

    #[test]
    fn the_last_occurrence_of_a_key_wins() {
        let parsed = parse("KEY=first\nKEY=second\n");
        assert_eq!(parsed.get("KEY").unwrap(), "second");
    }

    #[test]
    fn a_complete_file_has_no_gaps() {
        assert!(gaps(&all_set()).is_empty());
    }

    #[test]
    fn each_missing_variable_is_reported_by_name() {
        let mut incomplete = all_set();
        incomplete.remove("SESSION_KEY");
        incomplete.remove("ANTHROPIC_API_KEY");
        assert_eq!(
            gaps(&incomplete),
            vec![
                Gap::Unset("SESSION_KEY".to_owned()),
                Gap::Unset("ANTHROPIC_API_KEY".to_owned()),
            ]
        );
    }

    #[test]
    fn every_gap_is_reported_at_once_not_first_failure_only() {
        assert_eq!(
            gaps(&BTreeMap::new()),
            REQUIRED
                .iter()
                .map(|name| Gap::Unset((*name).to_owned()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn an_empty_value_counts_as_unset() {
        let mut hollow = all_set();
        hollow.insert("WORKOS_API_KEY".to_owned(), String::new());
        assert_eq!(gaps(&hollow), vec![Gap::Unset("WORKOS_API_KEY".to_owned())]);
    }

    #[test]
    fn only_the_legacy_hyperdrive_name_is_a_gap() {
        let mut outdated = all_set();
        outdated.remove(HYPERDRIVE_NAME);
        outdated.insert(LEGACY_HYPERDRIVE_NAME.to_owned(), "postgres://x".to_owned());
        assert_eq!(
            gaps(&outdated),
            vec![
                Gap::Unset(HYPERDRIVE_NAME.to_owned()),
                Gap::LegacyHyperdriveName,
            ]
        );
    }

    #[test]
    fn the_new_hyperdrive_name_satisfies_the_check() {
        let mut current = all_set();
        current.insert(LEGACY_HYPERDRIVE_NAME.to_owned(), "leftover".to_owned());
        assert!(gaps(&current).is_empty());
    }

    #[test]
    fn gap_messages_never_contain_values() {
        let secret = "sk-ant-super-secret-value";
        let mut leaky = all_set();
        leaky.insert("SESSION_KEY".to_owned(), secret.to_owned());
        leaky.remove("ANTHROPIC_API_KEY");
        leaky.remove(HYPERDRIVE_NAME);
        leaky.insert(
            LEGACY_HYPERDRIVE_NAME.to_owned(),
            format!("postgres://user:{secret}@localhost/db"),
        );

        let report = gaps(&leaky)
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(!report.contains(secret), "a value leaked: {report}");
        assert!(report.contains("ANTHROPIC_API_KEY"));
        assert!(report.contains(LEGACY_HYPERDRIVE_NAME));
    }

    #[test]
    fn the_current_name_beats_the_legacy_one() {
        let both = vars(&[
            (HYPERDRIVE_NAME, "postgres://current"),
            (LEGACY_HYPERDRIVE_NAME, "postgres://legacy"),
        ]);
        assert_eq!(
            database_url(&both),
            DatabaseUrl::Current("postgres://current".to_owned())
        );
    }

    #[test]
    fn the_legacy_name_alone_is_reported_as_legacy() {
        let outdated = vars(&[(LEGACY_HYPERDRIVE_NAME, "postgres://legacy")]);
        assert_eq!(
            database_url(&outdated),
            DatabaseUrl::Legacy("postgres://legacy".to_owned())
        );
    }

    #[test]
    fn nothing_set_means_absent() {
        assert_eq!(database_url(&vars(&[])), DatabaseUrl::Absent);
    }

    #[test]
    fn an_empty_connection_string_counts_as_absent() {
        let hollow = vars(&[(HYPERDRIVE_NAME, "")]);
        assert_eq!(database_url(&hollow), DatabaseUrl::Absent);
    }

    #[test]
    fn the_file_fills_names_the_process_does_not_export() {
        let process = vars(&[("PATH", "/bin")]);
        let file = vars(&[("SESSION_KEY", "from-file")]);
        let merged = overlay(&process, &file);
        assert_eq!(merged.get("SESSION_KEY").unwrap(), "from-file");
        assert_eq!(merged.get("PATH").unwrap(), "/bin");
    }

    #[test]
    fn an_explicit_shell_export_beats_the_file_default() {
        let process = vars(&[("SESSION_KEY", "from-shell")]);
        let file = vars(&[("SESSION_KEY", "from-file")]);
        assert_eq!(
            overlay(&process, &file).get("SESSION_KEY").unwrap(),
            "from-shell"
        );
    }

    #[test]
    fn an_empty_file_leaves_the_process_environment_alone() {
        let process = vars(&[("PATH", "/bin"), ("HOME", "/home/u")]);
        assert_eq!(overlay(&process, &vars(&[])), process);
    }

    #[test]
    fn every_file_entry_lands_when_nothing_is_exported() {
        let file = vars(&[("A", "1"), ("B", "2")]);
        let merged = overlay(&vars(&[]), &file);
        assert_eq!(merged, file);
    }

    #[test]
    fn a_credentialed_postgres_url_is_accepted() {
        assert_eq!(
            hyperdrive_url_problem("postgres://user:anything@localhost:5432/noal"),
            None
        );
        assert_eq!(
            hyperdrive_url_problem(
                "postgresql://user:secret@db.internal:5432/noal?sslmode=disable"
            ),
            None
        );
    }

    #[test]
    fn a_missing_password_component_is_rejected() {
        assert_eq!(
            hyperdrive_url_problem("postgres://user@localhost:5432/noal"),
            Some(super::UrlProblem::MissingPassword)
        );
    }

    #[test]
    fn a_user_with_no_password_at_all_is_rejected() {
        assert_eq!(
            hyperdrive_url_problem("postgres://localhost:5432/noal"),
            Some(super::UrlProblem::MissingPassword)
        );
    }

    #[test]
    fn an_empty_password_component_is_rejected() {
        assert_eq!(
            hyperdrive_url_problem("postgres://user:@localhost:5432/noal"),
            Some(super::UrlProblem::MissingPassword)
        );
    }

    #[test]
    fn a_wrong_scheme_is_rejected_and_named() {
        assert_eq!(
            hyperdrive_url_problem("mysql://user:pass@localhost/db"),
            Some(super::UrlProblem::WrongScheme("mysql".to_owned()))
        );
    }

    #[test]
    fn a_value_without_a_scheme_separator_is_rejected() {
        assert_eq!(
            hyperdrive_url_problem("localhost:5432/noal"),
            Some(super::UrlProblem::NotAUrl)
        );
    }

    #[test]
    fn a_missing_host_is_rejected() {
        assert_eq!(
            hyperdrive_url_problem("postgres://user:pass@/noal"),
            Some(super::UrlProblem::MissingHost)
        );
        assert_eq!(
            hyperdrive_url_problem("postgres://user:pass@:5433/noal"),
            Some(super::UrlProblem::MissingHost)
        );
    }

    #[test]
    fn an_at_sign_inside_the_password_still_finds_the_host() {
        assert_eq!(
            hyperdrive_url_problem("postgres://user:p@ssword@localhost:5432/noal"),
            None
        );
    }

    #[test]
    fn url_problem_messages_suggest_the_shape_but_never_echo_the_value() {
        let secret = "super-secret-password";
        // Missing host, so this produces a real problem carrying the secret.
        let leaky = format!("postgres://admin:{secret}@");
        let judged = hyperdrive_url_problem(&leaky);
        assert_ne!(judged, None);
        for problem in [
            super::UrlProblem::NotAUrl,
            super::UrlProblem::WrongScheme("mysql".to_owned()),
            super::UrlProblem::MissingHost,
            super::UrlProblem::MissingPassword,
        ]
        .into_iter()
        .chain(judged)
        {
            let message = problem.to_string();
            assert!(!message.contains(leaky.as_str()), "value leaked: {message}");
            assert!(!message.contains(secret), "password leaked: {message}");
            assert!(
                message.contains("postgres://user:anything@host:port/db"),
                "message lacks the suggested shape: {message}"
            );
        }
    }
}
