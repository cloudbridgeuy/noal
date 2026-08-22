//! Repository automation.
//!
//! See <https://github.com/matklad/cargo-xtask/>. This binary holds the
//! development commands that plain `cargo` cannot express.
//!
//! `lint` runs every quality check in order and manages the Git pre-commit
//! hook. `migrate` applies the SQL files under `migrations/` to a Postgres
//! database.
//!
//! This crate builds for the host, never for Wasm. It is the only place in the
//! repository that may talk to a database over a normal TCP socket, because
//! `noal_worker` reaches Neon through a Cloudflare Socket instead.
#![deny(clippy::unwrap_used, clippy::expect_used)]

mod lint;
mod migrate;

use clap::{Parser, Subcommand};
use color_eyre::eyre::Result;

#[derive(Parser)]
#[command(name = "xtask", about = "noal development tasks")]
struct App {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run every quality check: fmt, check, clippy, test, file length, and the
    /// `#[allow(clippy::too_many_arguments)]` ban
    Lint(lint::LintArgs),

    /// Apply pending SQL migrations to a Postgres database
    Migrate(migrate::MigrateArgs),
}

fn main() -> Result<()> {
    color_eyre::install()?;
    let app = App::parse();

    match app.command {
        Commands::Lint(args) => lint::run(&args),
        Commands::Migrate(args) => migrate::run(&args),
    }
}
