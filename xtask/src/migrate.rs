//! The migration runner.
//!
//! The top half is a functional core: it turns file names into migrations,
//! rejects malformed or duplicated ones, and decides which are pending. Those
//! decisions are pure, so they are unit tested without a database or a disk.
//!
//! The bottom half is the shell: it reads the directory, connects to Postgres,
//! and applies each pending migration inside a transaction together with the
//! row that records it. A migration and its bookkeeping commit or fail as one,
//! so a crash halfway cannot leave the ledger lying.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use clap::Args;
use color_eyre::eyre::{eyre, Result, WrapErr};

use crate::dev_vars;

/// Where migration files live, relative to the workspace root.
const MIGRATIONS_DIR: &str = "migrations";

/// The table that records which migrations have run.
const LEDGER_TABLE: &str = "schema_migrations";

// ---------------------------------------------------------------------------
// Functional core — pure types and logic, no input or output
// ---------------------------------------------------------------------------

/// One migration file, named `<version>_<description>.sql`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Migration {
    /// The numeric prefix. Ordering is by this, not by file name.
    pub version: i64,
    /// The human-readable remainder of the file name.
    pub description: String,
}

/// Why a set of migration files was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrateError {
    /// The file name had no `_` separating the version from the description.
    MissingSeparator(String),
    /// The prefix before the first `_` was not a number.
    UnparsableVersion(String),
    /// The description after the version was empty.
    EmptyDescription(String),
    /// Two files claimed the same version.
    DuplicateVersion(i64),
}

impl std::fmt::Display for MigrateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingSeparator(name) => {
                write!(f, "`{name}` must be named <version>_<description>.sql")
            }
            Self::UnparsableVersion(name) => {
                write!(f, "`{name}` does not start with a numeric version")
            }
            Self::EmptyDescription(name) => write!(f, "`{name}` has an empty description"),
            Self::DuplicateVersion(version) => {
                write!(f, "two migrations both claim version {version}")
            }
        }
    }
}

impl std::error::Error for MigrateError {}

/// Turn one file stem into a migration.
///
/// # Errors
///
/// Returns a [`MigrateError`] when the stem does not match
/// `<version>_<description>`.
pub fn parse_migration(stem: &str) -> Result<Migration, MigrateError> {
    let (version_text, description) = stem
        .split_once('_')
        .ok_or_else(|| MigrateError::MissingSeparator(stem.to_owned()))?;

    let version = version_text
        .parse::<i64>()
        .map_err(|_| MigrateError::UnparsableVersion(stem.to_owned()))?;

    if description.is_empty() {
        return Err(MigrateError::EmptyDescription(stem.to_owned()));
    }

    Ok(Migration {
        version,
        description: description.to_owned(),
    })
}

/// Parse every stem and sort the result by version.
///
/// # Errors
///
/// Returns the first parse failure, or [`MigrateError::DuplicateVersion`] when
/// two files claim one version.
pub fn parse_all(stems: &[String]) -> Result<Vec<Migration>, MigrateError> {
    let mut migrations = stems
        .iter()
        .map(|stem| parse_migration(stem))
        .collect::<Result<Vec<_>, _>>()?;

    migrations.sort();

    if let Some(duplicate) = first_duplicate_version(&migrations) {
        return Err(MigrateError::DuplicateVersion(duplicate));
    }

    Ok(migrations)
}

/// The first version claimed by more than one migration, if any.
///
/// Expects `migrations` sorted by version, which `parse_all` guarantees.
fn first_duplicate_version(migrations: &[Migration]) -> Option<i64> {
    migrations
        .windows(2)
        .find(|pair| pair[0].version == pair[1].version)
        .map(|pair| pair[0].version)
}

/// The migrations that have not run yet, in the order they must run.
#[must_use]
pub fn pending<'a>(all: &'a [Migration], applied: &BTreeSet<i64>) -> Vec<&'a Migration> {
    all.iter()
        .filter(|migration| !applied.contains(&migration.version))
        .collect()
}

/// A version recorded in the ledger with no matching file.
///
/// This means the database ran a migration this checkout does not have, so the
/// checkout is behind and applying more could corrupt the schema.
#[must_use]
pub fn orphaned(all: &[Migration], applied: &BTreeSet<i64>) -> Vec<i64> {
    let known: BTreeSet<i64> = all.iter().map(|migration| migration.version).collect();
    applied.difference(&known).copied().collect()
}

// ---------------------------------------------------------------------------
// Imperative shell — filesystem, network, and process effects
// ---------------------------------------------------------------------------

/// Command line for `cargo xtask migrate`.
#[derive(Args)]
pub struct MigrateArgs {
    /// Postgres connection string. When absent, the
    /// `CLOUDFLARE_HYPERDRIVE_LOCAL_CONNECTION_STRING_DB` variable from
    /// `.dev.vars` is used, so one file holds every local setting.
    #[arg(long)]
    pub database_url: Option<String>,

    /// Report what would run, then stop without changing anything
    #[arg(long)]
    pub dry_run: bool,
}

/// Apply every pending migration.
///
/// # Errors
///
/// Returns an error when the directory cannot be read, the file names are
/// malformed, the database refuses a connection, or a migration fails. A failed
/// migration rolls back with its ledger row.
pub fn run(args: &MigrateArgs) -> Result<()> {
    let database_url = resolve_database_url(&args.database_url)?;

    let directory = migrations_dir();
    let stems = read_migration_stems(&directory)?;
    let all = parse_all(&stems).map_err(|error| eyre!("{error}"))?;

    if all.is_empty() {
        println!("No migrations found in {}.", directory.display());
        return Ok(());
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .wrap_err("could not start the async runtime")?;

    runtime.block_on(apply(&database_url, &directory, &all, args.dry_run))
}

/// Decide where to connect.
///
/// An explicit flag wins over everything. Without one, the connection string
/// comes from `.dev.vars` — the same file wrangler reads — under its current
/// name, with the pre-wrangler-4 name accepted as a fallback so an unrenamed
/// file still works. Values are never printed; they are secrets.
///
/// # Errors
///
/// Returns an error when no flag was given and `.dev.vars` is missing,
/// unreadable, or holds no usable connection string; the message names the
/// file and the variables it looked for.
fn resolve_database_url(explicit: &Option<String>) -> Result<String> {
    if let Some(url) = explicit {
        return Ok(url.clone());
    }

    let root = std::env::var("CARGO_WORKSPACE_DIR").unwrap_or_else(|_| ".".to_owned());
    let file = dev_vars::path(Path::new(&root));

    let contents = match dev_vars::read(Path::new(&root)) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(eyre!(
                "no --database-url was given and {} does not exist. \
                 Either pass --database-url, or set {VAR} there \
                 (copy .dev.vars.example to create it)",
                file.display(),
                VAR = dev_vars::HYPERDRIVE_NAME
            ));
        }
        Err(error) => {
            return Err(error).wrap_err_with(|| format!("could not read {}", file.display()));
        }
    };

    match dev_vars::database_url(&dev_vars::parse(&contents)) {
        dev_vars::DatabaseUrl::Current(url) => Ok(url),
        dev_vars::DatabaseUrl::Legacy(url) => {
            eprintln!(
                "note: taking the connection string from {}, which is the legacy variable name; \
                 rename it to {}",
                dev_vars::LEGACY_HYPERDRIVE_NAME,
                dev_vars::HYPERDRIVE_NAME
            );
            Ok(url)
        }
        dev_vars::DatabaseUrl::Absent => Err(eyre!(
            "no --database-url was given and neither {current} nor its legacy spelling \
             {legacy} holds a value in {}. Either pass --database-url, or fill in {current}",
            file.display(),
            current = dev_vars::HYPERDRIVE_NAME,
            legacy = dev_vars::LEGACY_HYPERDRIVE_NAME
        )),
    }
}

/// The absolute path to the migrations directory.
fn migrations_dir() -> PathBuf {
    let root = std::env::var("CARGO_WORKSPACE_DIR").unwrap_or_else(|_| ".".to_owned());
    Path::new(&root).join(MIGRATIONS_DIR)
}

/// Every `.sql` file stem in `directory`.
fn read_migration_stems(directory: &Path) -> Result<Vec<String>> {
    if !directory.is_dir() {
        return Ok(Vec::new());
    }

    let mut stems = Vec::new();
    for entry in fs::read_dir(directory)
        .wrap_err_with(|| format!("could not read {}", directory.display()))?
    {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "sql") {
            if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
                stems.push(stem.to_owned());
            }
        }
    }
    Ok(stems)
}

/// Connect, compare the ledger with the files, and run what is pending.
async fn apply(
    database_url: &str,
    directory: &Path,
    all: &[Migration],
    dry_run: bool,
) -> Result<()> {
    let connector = postgres_native_tls::MakeTlsConnector::new(
        native_tls::TlsConnector::new().wrap_err("could not build a TLS connector")?,
    );

    let (mut client, connection) = tokio_postgres_native::connect(database_url, connector)
        .await
        .wrap_err("could not connect to the database")?;

    tokio::spawn(async move {
        if let Err(error) = connection.await {
            eprintln!("connection error: {error}");
        }
    });

    client
        .batch_execute(&format!(
            "CREATE TABLE IF NOT EXISTS {LEDGER_TABLE} (
                 version     BIGINT PRIMARY KEY,
                 description TEXT        NOT NULL,
                 applied_at  TIMESTAMPTZ NOT NULL DEFAULT now()
             )"
        ))
        .await
        .wrap_err("could not create the migration ledger")?;

    let applied = read_ledger(&client).await?;

    let stranded = orphaned(all, &applied);
    if !stranded.is_empty() {
        return Err(eyre!(
            "the database has run migration(s) {stranded:?} that this checkout does not contain; \
             update the checkout before migrating"
        ));
    }

    let queue = pending(all, &applied);
    if queue.is_empty() {
        println!(
            "Up to date. {} migration(s) already applied.",
            applied.len()
        );
        return Ok(());
    }

    for migration in &queue {
        println!(
            "pending  {:04}_{}",
            migration.version, migration.description
        );
    }

    if dry_run {
        println!("\nDry run. Nothing was applied.");
        return Ok(());
    }

    for migration in queue {
        apply_one(&mut client, directory, migration).await?;
        println!(
            "applied  {:04}_{}",
            migration.version, migration.description
        );
    }

    Ok(())
}

/// Every version already recorded in the ledger.
async fn read_ledger(client: &tokio_postgres_native::Client) -> Result<BTreeSet<i64>> {
    let rows = client
        .query(&format!("SELECT version FROM {LEDGER_TABLE}"), &[])
        .await
        .wrap_err("could not read the migration ledger")?;

    Ok(rows
        .iter()
        .map(|row| row.get::<_, i64>("version"))
        .collect())
}

/// Run one migration and record it, both inside one transaction.
async fn apply_one(
    client: &mut tokio_postgres_native::Client,
    directory: &Path,
    migration: &Migration,
) -> Result<()> {
    let path = directory.join(format!(
        "{:04}_{}.sql",
        migration.version, migration.description
    ));
    let sql =
        fs::read_to_string(&path).wrap_err_with(|| format!("could not read {}", path.display()))?;

    let transaction = client
        .transaction()
        .await
        .wrap_err("could not open a transaction")?;

    transaction
        .batch_execute(&sql)
        .await
        .wrap_err_with(|| format!("migration {} failed", path.display()))?;

    transaction
        .execute(
            &format!("INSERT INTO {LEDGER_TABLE} (version, description) VALUES ($1, $2)"),
            &[&migration.version, &migration.description],
        )
        .await
        .wrap_err("could not record the migration")?;

    transaction
        .commit()
        .await
        .wrap_err("could not commit the migration")?;

    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{orphaned, parse_all, parse_migration, pending, MigrateError, Migration};
    use std::collections::BTreeSet;

    fn migration(version: i64, description: &str) -> Migration {
        Migration {
            version,
            description: description.to_owned(),
        }
    }

    #[test]
    fn parses_a_well_formed_stem() {
        assert_eq!(parse_migration("0001_init").unwrap(), migration(1, "init"));
    }

    #[test]
    fn keeps_underscores_in_the_description() {
        assert_eq!(
            parse_migration("0012_add_user_table").unwrap(),
            migration(12, "add_user_table")
        );
    }

    #[test]
    fn rejects_a_stem_with_no_separator() {
        assert_eq!(
            parse_migration("0001init"),
            Err(MigrateError::MissingSeparator("0001init".to_owned()))
        );
    }

    #[test]
    fn rejects_a_non_numeric_version() {
        assert_eq!(
            parse_migration("first_init"),
            Err(MigrateError::UnparsableVersion("first_init".to_owned()))
        );
    }

    #[test]
    fn rejects_an_empty_description() {
        assert_eq!(
            parse_migration("0001_"),
            Err(MigrateError::EmptyDescription("0001_".to_owned()))
        );
    }

    #[test]
    fn sorts_by_version_not_by_name() {
        let stems = vec!["0010_ten".to_owned(), "0002_two".to_owned()];
        let parsed = parse_all(&stems).unwrap();
        assert_eq!(parsed, vec![migration(2, "two"), migration(10, "ten")]);
    }

    #[test]
    fn rejects_two_migrations_claiming_one_version() {
        let stems = vec!["0001_one".to_owned(), "0001_other".to_owned()];
        assert_eq!(parse_all(&stems), Err(MigrateError::DuplicateVersion(1)));
    }

    #[test]
    fn pending_skips_what_the_ledger_already_holds() {
        let all = vec![
            migration(1, "one"),
            migration(2, "two"),
            migration(3, "three"),
        ];
        let applied: BTreeSet<i64> = [1, 2].into_iter().collect();
        let queue = pending(&all, &applied);
        assert_eq!(queue, vec![&all[2]]);
    }

    #[test]
    fn pending_is_empty_when_everything_ran() {
        let all = vec![migration(1, "one")];
        let applied: BTreeSet<i64> = [1].into_iter().collect();
        assert!(pending(&all, &applied).is_empty());
    }

    #[test]
    fn pending_preserves_version_order() {
        let all = vec![migration(1, "one"), migration(2, "two")];
        let applied = BTreeSet::new();
        let queue = pending(&all, &applied);
        assert_eq!(
            queue.iter().map(|m| m.version).collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn orphaned_finds_a_version_the_checkout_lacks() {
        let all = vec![migration(1, "one")];
        let applied: BTreeSet<i64> = [1, 7].into_iter().collect();
        assert_eq!(orphaned(&all, &applied), vec![7]);
    }

    #[test]
    fn orphaned_is_empty_when_the_checkout_is_current() {
        let all = vec![migration(1, "one"), migration(2, "two")];
        let applied: BTreeSet<i64> = [1].into_iter().collect();
        assert!(orphaned(&all, &applied).is_empty());
    }
}
