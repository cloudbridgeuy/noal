//! Reading and judging `.dev.vars`.
//!
//! `.dev.vars` is the one file where a local checkout keeps its configuration,
//! because it is the same file `wrangler dev` already loads. Every xtask
//! command that needs local settings comes here.
//!
//! The top half is a functional core: turning the file's text into key–value
//! pairs, and deciding what is missing, are pure functions over string
//! contents, so they are unit tested without touching a disk. The bottom half
//! is the shell: locating and reading the file.

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
        database_url, gaps, parse, DatabaseUrl, Gap, HYPERDRIVE_NAME, LEGACY_HYPERDRIVE_NAME,
        REQUIRED,
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
}
