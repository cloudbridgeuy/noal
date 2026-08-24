//! The local development launcher.
//!
//! The top half is a functional core: it turns facts about the machine into an
//! ordered plan of prerequisite fixes, and decides staleness, target
//! presence, and how `.dev.vars` entries merge over the process environment
//! from injected values. Those decisions are pure, so they are unit tested
//! without touching the filesystem or spawning a process.
//!
//! The bottom half is the shell: it reads `.dev.vars`, probes for tools,
//! reads modification times, runs each fix with its output inherited so
//! failures speak for themselves, and finally replaces this process with
//! wrangler.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use clap::Args;
use color_eyre::eyre::{eyre, Result, WrapErr};

use crate::dev_vars;

/// The compilation target the Worker crate needs.
const WASM_TARGET: &str = "wasm32-unknown-unknown";

// ---------------------------------------------------------------------------
// Functional core — pure types and logic, no input or output
// ---------------------------------------------------------------------------

/// One prerequisite fix the dev command may perform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Turn on pnpm through corepack, which ships with Node.
    EnablePnpm,
    /// Install or refresh the Node dependencies with pnpm.
    InstallNodeDeps,
    /// Add the Wasm compilation target through rustup.
    AddWasmTarget,
    /// Install worker-build through cargo.
    InstallWorkerBuild,
}

impl std::fmt::Display for Step {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EnablePnpm => write!(f, "enabling pnpm through corepack"),
            Self::InstallNodeDeps => write!(f, "installing Node dependencies"),
            Self::AddWasmTarget => write!(f, "adding the {WASM_TARGET} target"),
            Self::InstallWorkerBuild => write!(f, "installing worker-build"),
        }
    }
}

/// What the machine looked like when `dev` started.
///
/// Every field is an answer the shell gathered by probing; the plan is decided
/// from these answers alone, which keeps the decision testable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facts {
    /// Whether `corepack` can be spawned.
    pub corepack_present: bool,
    /// Whether `pnpm` can be spawned.
    pub pnpm_present: bool,
    /// Whether `node_modules` is missing or older than `package.json`.
    pub node_deps_stale: bool,
    /// Whether rustup lists [`WASM_TARGET`]-equivalent as installed.
    pub wasm_target_installed: bool,
    /// Whether `worker-build` can be spawned.
    pub worker_build_installed: bool,
}

/// Why the dev environment cannot be started at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevError {
    /// Neither pnpm nor corepack was found, so Node itself appears missing.
    NodeMissing,
}

impl std::fmt::Display for DevError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NodeMissing => write!(
                f,
                "neither pnpm nor corepack was found; \
                 install Node <https://nodejs.org> and try again"
            ),
        }
    }
}

impl std::error::Error for DevError {}

/// The fixes to apply, in the order they must run.
///
/// # Errors
///
/// Returns [`DevError::NodeMissing`] when pnpm is absent and corepack cannot
/// enable it.
pub fn plan(facts: &Facts) -> Result<Vec<Step>, DevError> {
    let mut steps = Vec::new();

    if !facts.pnpm_present {
        if !facts.corepack_present {
            return Err(DevError::NodeMissing);
        }
        steps.push(Step::EnablePnpm);
    }

    if facts.node_deps_stale {
        steps.push(Step::InstallNodeDeps);
    }

    if !facts.wasm_target_installed {
        steps.push(Step::AddWasmTarget);
    }

    if !facts.worker_build_installed {
        steps.push(Step::InstallWorkerBuild);
    }

    Ok(steps)
}

/// Are the Node dependencies missing or older than the manifest?
///
/// Missing dependencies always count as stale. An unreadable manifest also
/// counts as stale, because `pnpm install` will report exactly what is wrong
/// with it, which is more honest than guessing.
pub fn deps_stale(node_modules: Option<SystemTime>, package_json: Option<SystemTime>) -> bool {
    let Some(deps_modified) = node_modules else {
        return true;
    };
    match package_json {
        Some(manifest_modified) => manifest_modified > deps_modified,
        None => true,
    }
}

/// Does the output of `rustup target list --installed` name the Wasm target?
pub fn targets_include(listing: &str) -> bool {
    listing.lines().any(|line| line.trim() == WASM_TARGET)
}

/// The command that performs one step.
fn command_line(step: &Step) -> (&'static str, Vec<&'static str>) {
    match step {
        Step::EnablePnpm => ("corepack", vec!["enable", "pnpm"]),
        Step::InstallNodeDeps => ("pnpm", vec!["install"]),
        Step::AddWasmTarget => ("rustup", vec!["target", "add", WASM_TARGET]),
        Step::InstallWorkerBuild => ("cargo", vec!["install", "worker-build"]),
    }
}

// ---------------------------------------------------------------------------
// Imperative shell — processes, filesystem, orchestration
// ---------------------------------------------------------------------------

/// Command line for `cargo xtask dev`.
#[derive(Args)]
pub struct DevArgs {
    /// Extra arguments forwarded verbatim to `wrangler dev`
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub wrangler_args: Vec<String>,
}

/// Fill every gap between this machine and `wrangler dev`, then start it.
///
/// # Errors
///
/// Returns an error when Node is missing entirely or any fix command fails;
/// the failing tool's own output has already been printed by then.
pub fn run(args: &DevArgs) -> Result<()> {
    let file_vars = check_dev_vars()?;

    let facts = gather_facts();

    let steps = plan(&facts).map_err(|error| eyre!("{error}"))?;

    if steps.is_empty() {
        println!("Everything needed for dev is already in place.");
    }

    for step in steps {
        println!("{step}:");
        run_step(&step)?;
    }

    start_wrangler(&file_vars, &args.wrangler_args)
}

/// Judge `.dev.vars` before anything is installed or started.
///
/// This runs ahead of every prerequisite fix on purpose: wrangler 4 ignores
/// the legacy Hyperdrive variable name, so a stale file would otherwise start
/// a server with no database behind it and only fail at request time. All
/// gaps are named in one pass. Values are never printed — they are secrets.
///
/// # Errors
///
/// Returns an error when the file is missing, unreadable, has gaps, or its
/// Hyperdrive connection string cannot serve wrangler; the message names
/// exactly what is wrong and how to fix it. Otherwise the parsed variables
/// come back, ready to be handed to wrangler's environment.
fn check_dev_vars() -> Result<BTreeMap<String, String>> {
    let root = workspace_root();
    let file = dev_vars::path(&root);

    let contents = match dev_vars::read(&root) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(eyre!(
                "{} is missing. Copy the example into place first:\n  cp .dev.vars.example .dev.vars",
                file.display()
            ));
        }
        Err(error) => {
            return Err(error).wrap_err_with(|| format!("could not read {}", file.display()));
        }
    };

    let vars = dev_vars::parse(&contents);
    let problems = dev_vars::gaps(&vars);
    if !problems.is_empty() {
        let mut report = format!("{} needs attention before dev can start:", file.display());
        for problem in &problems {
            report.push_str(&format!("\n  - {problem}"));
        }
        return Err(eyre!(report));
    }

    // Wrangler reads this variable from its environment, not from
    // `.dev.vars`, and refuses to start over a passwordless URL even though
    // local Postgres under trust auth would accept one — better to say so
    // here than after wrangler has failed.
    if let dev_vars::DatabaseUrl::Current(url) = dev_vars::database_url(&vars) {
        if let Some(problem) = dev_vars::hyperdrive_url_problem(&url) {
            return Err(eyre!("{}: {problem}", file.display()));
        }
    }

    Ok(vars)
}

/// Probe the machine once, before anything changes.
fn gather_facts() -> Facts {
    let root = workspace_root();
    Facts {
        corepack_present: spawnable("corepack", &["--version"]),
        pnpm_present: spawnable("pnpm", &["--version"]),
        node_deps_stale: deps_stale(
            modified_at(&root.join("node_modules")),
            modified_at(&root.join("package.json")),
        ),
        wasm_target_installed: wasm_target_installed(),
        worker_build_installed: spawnable("worker-build", &["--version"]),
    }
}

/// Could a program on PATH be spawned?
fn spawnable(program: &str, args: &[&str]) -> bool {
    Command::new(program).args(args).output().is_ok()
}

/// Whether rustup already has the Wasm target installed.
fn wasm_target_installed() -> bool {
    let output = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output();
    match output {
        Ok(output) => targets_include(&String::from_utf8_lossy(&output.stdout)),
        Err(_) => false,
    }
}

/// When a path was last changed, or nothing if it cannot be read.
fn modified_at(path: &Path) -> Option<SystemTime> {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
}

/// The checkout root, matching how the migration runner finds its files.
fn workspace_root() -> PathBuf {
    let root = std::env::var("CARGO_WORKSPACE_DIR").unwrap_or_else(|_| ".".to_owned());
    PathBuf::from(root)
}

/// Run one fix with its output inherited, so failures speak for themselves.
fn run_step(step: &Step) -> Result<()> {
    let (program, args) = command_line(step);
    println!("$ {program} {}", args.join(" "));

    let status = Command::new(program)
        .args(args)
        .status()
        .wrap_err_with(|| format!("could not run `{program}`"))?;

    if !status.success() {
        return Err(eyre!("`{program}` failed while {step} (status {status})"));
    }

    Ok(())
}

/// Replace this process with `wrangler dev`, forwarding extra arguments.
///
/// Wrangler reads the local Hyperdrive connection string from its
/// environment, not from `.dev.vars`, so every parsed entry is placed into
/// the child's environment first. A variable the shell already exported
/// keeps its exported value, matching standard dotenv precedence.
fn start_wrangler(file_vars: &BTreeMap<String, String>, extra: &[String]) -> Result<()> {
    let forwarded = if extra.is_empty() {
        String::new()
    } else {
        format!(" {}", extra.join(" "))
    };
    println!("$ pnpm exec wrangler dev{forwarded}");

    let process_env: BTreeMap<String, String> = std::env::vars().collect();
    let injected = file_vars
        .keys()
        .filter(|name| !process_env.contains_key(*name))
        .cloned()
        .collect::<Vec<_>>();
    if !injected.is_empty() {
        println!("Loaded from .dev.vars: {}", injected.join(", "));
    }

    let mut command = Command::new("pnpm");
    command.args(["exec", "wrangler", "dev"]).args(extra);
    for (name, value) in dev_vars::overlay(&process_env, file_vars) {
        command.env(name, value);
    }

    // Replacing the process lets Ctrl-C reach wrangler directly instead of
    // passing through this intermediate.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let error = command.exec();
        Err(error).wrap_err("could not start wrangler")
    }

    #[cfg(not(unix))]
    {
        let status = command.status().wrap_err("could not start wrangler")?;
        std::process::exit(status.code().unwrap_or(1));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{deps_stale, plan, targets_include, DevError, Facts, Step};
    use std::time::{Duration, SystemTime};

    fn facts() -> Facts {
        Facts {
            corepack_present: true,
            pnpm_present: true,
            node_deps_stale: false,
            wasm_target_installed: true,
            worker_build_installed: true,
        }
    }

    fn settled(offset_secs: u64) -> Option<SystemTime> {
        Some(SystemTime::UNIX_EPOCH + Duration::new(offset_secs, 0))
    }

    #[test]
    fn a_complete_machine_needs_no_steps() {
        assert_eq!(plan(&facts()).unwrap(), Vec::<Step>::new());
    }

    #[test]
    fn missing_pnpm_is_enabled_through_corepack() {
        let mut current = facts();
        current.pnpm_present = false;
        assert_eq!(plan(&current).unwrap(), vec![Step::EnablePnpm]);
    }

    #[test]
    fn present_pnpm_does_not_need_corepack() {
        let mut current = facts();
        current.corepack_present = false;
        assert_eq!(plan(&current).unwrap(), Vec::<Step>::new());
    }

    #[test]
    fn no_pnpm_and_no_corepack_means_node_is_missing() {
        let mut current = facts();
        current.pnpm_present = false;
        current.corepack_present = false;
        assert_eq!(plan(&current), Err(DevError::NodeMissing));
    }

    #[test]
    fn stale_node_dependencies_are_reinstalled() {
        let mut current = facts();
        current.node_deps_stale = true;
        assert_eq!(plan(&current).unwrap(), vec![Step::InstallNodeDeps]);
    }

    #[test]
    fn a_missing_wasm_target_is_added() {
        let mut current = facts();
        current.wasm_target_installed = false;
        assert_eq!(plan(&current).unwrap(), vec![Step::AddWasmTarget]);
    }

    #[test]
    fn a_missing_worker_build_is_installed() {
        let mut current = facts();
        current.worker_build_installed = false;
        assert_eq!(plan(&current).unwrap(), vec![Step::InstallWorkerBuild]);
    }

    #[test]
    fn every_gap_produces_every_step_in_order() {
        let mut bare = facts();
        bare.pnpm_present = false;
        bare.node_deps_stale = true;
        bare.wasm_target_installed = false;
        bare.worker_build_installed = false;
        assert_eq!(
            plan(&bare).unwrap(),
            vec![
                Step::EnablePnpm,
                Step::InstallNodeDeps,
                Step::AddWasmTarget,
                Step::InstallWorkerBuild,
            ]
        );
    }

    #[test]
    fn missing_node_modules_are_stale() {
        assert!(deps_stale(None, settled(10)));
    }

    #[test]
    fn dependencies_older_than_the_manifest_are_stale() {
        assert!(deps_stale(settled(5), settled(10)));
    }

    #[test]
    fn dependencies_newer_than_the_manifest_are_fresh() {
        assert!(!deps_stale(settled(10), settled(5)));
    }

    #[test]
    fn an_unreadable_manifest_counts_as_stale() {
        assert!(deps_stale(settled(10), None));
    }

    #[test]
    fn the_installed_list_names_the_wasm_target() {
        let listing = "aarch64-apple-darwin\nwasm32-unknown-unknown\nx86_64-unknown-linux-gnu\n";
        assert!(targets_include(listing));
    }

    #[test]
    fn a_prefix_match_is_not_the_wasm_target() {
        assert!(!targets_include("wasm32-wasip1\n"));
    }

    #[test]
    fn an_empty_list_lacks_the_wasm_target() {
        assert!(!targets_include(""));
    }
}
