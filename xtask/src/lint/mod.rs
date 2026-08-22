//! The lint pipeline.
//!
//! The functional-core/imperative-shell split applies here. The top half is a
//! functional core: it decides which checks to run, what to call them, how to
//! read an exit status, and whether a set of collected facts passes. The bottom
//! half is the shell: it spawns processes, walks directories, reads files, and
//! writes the log.
//!
//! Two checks have no external tool. File length and the ban on
//! `#[allow(clippy::too_many_arguments)]` are decided by pure functions over
//! collected facts, so they are unit tested without touching the filesystem.

pub mod hooks;

use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use clap::Args;
use color_eyre::eyre::{eyre, Result, WrapErr};

/// No source file may pass this many lines.
const MAX_FILE_LINES: usize = 1000;

/// Where the full transcript of a run is written.
const LOG_NAME: &str = "xtask-lint.log";

/// Git query for staged Rust files that a repair can restage.
const STAGED_RUST_DIFF_ARGS: &[&str] = &[
    "diff",
    "--cached",
    "--name-only",
    "--diff-filter=ACMR",
    "--",
];

/// Git query for any unstaged change to a staged Rust path.
const UNSTAGED_RUST_DIFF_ARGS: &[&str] = &["diff", "--name-only", "--"];

// ---------------------------------------------------------------------------
// Functional core — pure types and logic, no input or output
// ---------------------------------------------------------------------------

/// Names one check, so skip flags and fix-mode overrides can match on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckId {
    Fmt,
    Check,
    CheckWasm,
    Clippy,
    ClippyWasm,
    Test,
    FileLength,
    TooManyArgsAllow,
}

/// One check in the pipeline.
///
/// A check with the sentinel program `__builtin__` runs in this crate instead
/// of in a child process.
struct Check {
    id: CheckId,
    name: &'static str,
    program: &'static str,
    args: &'static [&'static str],
    optional: bool,
}

/// What a check decided.
enum CheckOutcome {
    Passed { output: String },
    Failed { output: String },
    Skipped,
}

/// A check and its outcome, ready to log.
struct CheckResult {
    name: String,
    outcome: CheckOutcome,
}

/// Command line for `cargo run -p xtask -- lint`.
#[derive(Args)]
pub struct LintArgs {
    /// Print all output, not only failures
    #[arg(long)]
    pub verbose: bool,

    /// Skip the format check
    #[arg(long)]
    pub no_fmt: bool,

    /// Skip the compile check
    #[arg(long)]
    pub no_check: bool,

    /// Skip Clippy
    #[arg(long)]
    pub no_clippy: bool,

    /// Skip the tests
    #[arg(long)]
    pub no_test: bool,

    /// Skip the file-length check
    #[arg(long)]
    pub no_file_length: bool,

    /// Skip the `#[allow(clippy::too_many_arguments)]` ban
    #[arg(long)]
    pub no_too_many_args: bool,

    /// Repair what can be repaired: apply formatting and Clippy fixes
    #[arg(long)]
    pub fix: bool,

    /// Pre-commit hook mode: implies `--fix` and stages the repaired files again
    #[arg(long, hide = true)]
    pub staged_only: bool,

    /// Install the git pre-commit hook
    #[arg(long, conflicts_with_all = ["uninstall_hooks", "hooks_status"])]
    pub install_hooks: bool,

    /// Remove the git pre-commit hook
    #[arg(long, conflicts_with_all = ["install_hooks", "hooks_status"])]
    pub uninstall_hooks: bool,

    /// Report whether the git pre-commit hook is installed
    #[arg(long, conflicts_with_all = ["install_hooks", "uninstall_hooks"])]
    pub hooks_status: bool,
}

/// The pipeline, in order. The cheapest check that fails most often comes first.
const CHECKS: &[Check] = &[
    Check {
        id: CheckId::Fmt,
        name: "cargo fmt -- --check",
        program: "cargo",
        args: &["fmt", "--", "--check"],
        optional: false,
    },
    Check {
        id: CheckId::Check,
        name: "cargo check --workspace --exclude noal_worker --all-targets",
        program: "cargo",
        args: &[
            "check",
            "--workspace",
            "--exclude",
            "noal_worker",
            "--all-targets",
        ],
        optional: false,
    },
    Check {
        id: CheckId::CheckWasm,
        name: "cargo check -p noal_worker --target wasm32-unknown-unknown",
        program: "cargo",
        args: &[
            "check",
            "-p",
            "noal_worker",
            "--target",
            "wasm32-unknown-unknown",
        ],
        optional: false,
    },
    Check {
        id: CheckId::Clippy,
        name: "cargo clippy --workspace --exclude noal_worker --all-targets -- -D warnings",
        program: "cargo",
        args: &[
            "clippy",
            "--workspace",
            "--exclude",
            "noal_worker",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
        optional: false,
    },
    Check {
        id: CheckId::ClippyWasm,
        name: "cargo clippy -p noal_worker --target wasm32-unknown-unknown -- -D warnings",
        program: "cargo",
        args: &[
            "clippy",
            "-p",
            "noal_worker",
            "--target",
            "wasm32-unknown-unknown",
            "--",
            "-D",
            "warnings",
        ],
        optional: false,
    },
    Check {
        id: CheckId::Test,
        name: "cargo test --workspace --exclude noal_worker --all-targets",
        program: "cargo",
        args: &[
            "test",
            "--workspace",
            "--exclude",
            "noal_worker",
            "--all-targets",
        ],
        optional: false,
    },
    Check {
        id: CheckId::FileLength,
        name: "file length (<= 1000 lines)",
        program: "__builtin__",
        args: &[],
        optional: false,
    },
    Check {
        id: CheckId::TooManyArgsAllow,
        name: "forbid #[allow(clippy::too_many_arguments)]",
        program: "__builtin__",
        args: &[],
        optional: false,
    },
];

/// Did the operator ask for this check to be left out?
fn should_skip(id: CheckId, args: &LintArgs) -> bool {
    match id {
        CheckId::Fmt => args.no_fmt,
        CheckId::Check | CheckId::CheckWasm => args.no_check,
        CheckId::Clippy | CheckId::ClippyWasm => args.no_clippy,
        CheckId::Test => args.no_test,
        CheckId::FileLength => args.no_file_length,
        CheckId::TooManyArgsAllow => args.no_too_many_args,
    }
}

/// The arguments a check takes in `--fix` mode, if it can repair anything.
///
/// - `fmt` drops `--check`, so the formatting is written.
/// - `clippy` gains `--fix --allow-dirty`, so the machine-applicable lints are
///   written.
/// - Every other check reports only, and returns `None`.
fn fix_args(id: CheckId) -> Option<Vec<&'static str>> {
    match id {
        CheckId::Fmt => Some(vec!["fmt"]),
        CheckId::Clippy => Some(vec![
            "clippy",
            "--workspace",
            "--exclude",
            "noal_worker",
            "--all-targets",
            "--fix",
            "--allow-dirty",
            "--",
            "-D",
            "warnings",
        ]),
        CheckId::ClippyWasm => Some(vec![
            "clippy",
            "-p",
            "noal_worker",
            "--target",
            "wasm32-unknown-unknown",
            "--fix",
            "--allow-dirty",
            "--",
            "-D",
            "warnings",
        ]),
        _ => None,
    }
}

/// The name to print for a check whose arguments were overridden.
fn check_display_name(program: &str, args: &[&str]) -> String {
    if args.is_empty() {
        program.to_string()
    } else {
        format!("{program} {}", args.join(" "))
    }
}

/// Does this output say the tool is absent rather than unhappy?
fn is_tool_not_found(output: &str) -> bool {
    let lower = output.to_lowercase();
    lower.contains("not found")
        || lower.contains("no such file or directory")
        || lower.contains("unrecognized subcommand")
        || lower.contains("no such command")
}

/// Read an exit status and its output as an outcome.
fn determine_outcome(success: bool, output: String, optional: bool) -> CheckOutcome {
    if optional && !success && is_tool_not_found(&output) {
        return CheckOutcome::Skipped;
    }
    if success {
        CheckOutcome::Passed { output }
    } else {
        CheckOutcome::Failed { output }
    }
}

/// Render one result as a log entry.
fn format_log_entry(result: &CheckResult) -> String {
    match &result.outcome {
        CheckOutcome::Skipped => {
            format!("=== {} ===\n[skipped — tool not installed]\n", result.name)
        }
        CheckOutcome::Passed { output } | CheckOutcome::Failed { output } => {
            format!("=== {} ===\n{}\n", result.name, output)
        }
    }
}

/// Decide the file-length check from collected line counts.
fn evaluate_file_lengths(files: &[(String, usize)], max_lines: usize) -> CheckOutcome {
    let violations: Vec<String> = files
        .iter()
        .filter(|(_, count)| *count > max_lines)
        .map(|(path, count)| format!("  {path} ({count} lines)"))
        .collect();

    if violations.is_empty() {
        CheckOutcome::Passed {
            output: String::new(),
        }
    } else {
        CheckOutcome::Failed {
            output: format!(
                "Files longer than {max_lines} lines:\n{}\n",
                violations.join("\n")
            ),
        }
    }
}

/// One forbidden `#[allow(clippy::too_many_arguments)]` attribute.
struct TooManyArgsFinding {
    path: String,
    line: usize,
    text: String,
}

/// Does this line declare `#[allow(...)]` or `#![allow(...)]` with
/// `clippy::too_many_arguments` among the allowed lints?
///
/// The test anchors on `#[` or `#![` at the start of the trimmed line, so prose
/// in a doc comment or a string literal cannot match.
fn line_forbids_too_many_args(attribute: &str) -> bool {
    let compact = attribute
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    let rest = compact
        .strip_prefix("#![")
        .or_else(|| compact.strip_prefix("#["));
    let Some(rest) = rest else {
        return false;
    };
    let Some(rest) = rest.strip_prefix("allow") else {
        return false;
    };
    let Some(rest) = rest.strip_prefix('(') else {
        return false;
    };
    let Some(end) = rest.find(')') else {
        return false;
    };
    rest[..end]
        .split(',')
        .any(|token| token.trim() == "clippy::too_many_arguments")
}

/// Remove comments and literals while preserving line breaks and code bytes.
///
/// The returned text is only for structural scanning. Replaced bytes become
/// spaces, so source line numbers stay stable.
fn structural_source(content: &str) -> String {
    #[derive(Clone, Copy)]
    enum State {
        Code,
        BlockComment(usize),
        String { escaped: bool },
        RawString { hashes: usize },
    }

    fn blank(output: &mut [u8], index: usize) {
        if output[index] != b'\n' && output[index] != b'\r' {
            output[index] = b' ';
        }
    }

    let input = content.as_bytes();
    let mut output = input.to_vec();
    let mut state = State::Code;
    let mut index = 0;

    while index < input.len() {
        match state {
            State::Code => {
                if input[index..].starts_with(b"//") {
                    while index < input.len() && input[index] != b'\n' {
                        blank(&mut output, index);
                        index += 1;
                    }
                } else if input[index..].starts_with(b"/*") {
                    blank(&mut output, index);
                    blank(&mut output, index + 1);
                    index += 2;
                    state = State::BlockComment(1);
                } else if input[index] == b'r' {
                    let mut quote = index + 1;
                    while quote < input.len() && input[quote] == b'#' {
                        quote += 1;
                    }
                    if quote < input.len() && input[quote] == b'"' {
                        let hashes = quote - index - 1;
                        for byte in output.iter_mut().take(quote + 1).skip(index) {
                            if *byte != b'\n' && *byte != b'\r' {
                                *byte = b' ';
                            }
                        }
                        index = quote + 1;
                        state = State::RawString { hashes };
                    } else {
                        index += 1;
                    }
                } else if input[index] == b'"' {
                    blank(&mut output, index);
                    index += 1;
                    state = State::String { escaped: false };
                } else if input[index] == b'\'' {
                    let end = if input.get(index + 1) == Some(&b'\\') {
                        let mut cursor = index + 2;
                        while cursor < input.len() && input[cursor] != b'\n' {
                            if input[cursor] == b'\'' && input[cursor - 1] != b'\\' {
                                break;
                            }
                            cursor += 1;
                        }
                        (cursor < input.len() && input[cursor] == b'\'').then_some(cursor)
                    } else {
                        content[index + 1..]
                            .chars()
                            .next()
                            .map(|character| index + 1 + character.len_utf8())
                            .filter(|&cursor| input.get(cursor) == Some(&b'\''))
                    };

                    if let Some(end) = end {
                        for byte in output.iter_mut().take(end + 1).skip(index) {
                            if *byte != b'\n' && *byte != b'\r' {
                                *byte = b' ';
                            }
                        }
                        index = end + 1;
                    } else {
                        index += 1;
                    }
                } else {
                    index += 1;
                }
            }
            State::BlockComment(depth) => {
                if input[index..].starts_with(b"/*") {
                    blank(&mut output, index);
                    blank(&mut output, index + 1);
                    index += 2;
                    state = State::BlockComment(depth + 1);
                } else if input[index..].starts_with(b"*/") {
                    blank(&mut output, index);
                    blank(&mut output, index + 1);
                    index += 2;
                    state = if depth == 1 {
                        State::Code
                    } else {
                        State::BlockComment(depth - 1)
                    };
                } else {
                    blank(&mut output, index);
                    index += 1;
                }
            }
            State::String { escaped } => {
                let byte = input[index];
                blank(&mut output, index);
                index += 1;
                state = if escaped {
                    State::String { escaped: false }
                } else if byte == b'\\' {
                    State::String { escaped: true }
                } else if byte == b'"' {
                    State::Code
                } else {
                    State::String { escaped: false }
                };
            }
            State::RawString { hashes } => {
                let closes = input[index] == b'"'
                    && input
                        .get(index + 1..index + 1 + hashes)
                        .is_some_and(|suffix| suffix.iter().all(|byte| *byte == b'#'));
                blank(&mut output, index);
                index += 1;
                if closes {
                    for _ in 0..hashes {
                        blank(&mut output, index);
                        index += 1;
                    }
                    state = State::Code;
                }
            }
        }
    }

    String::from_utf8_lossy(&output).into_owned()
}

/// Scan one file for forbidden allows, letting `#[cfg(test)]` code through.
///
/// Test gating is tracked with a brace-depth stack. Comments and literals are
/// removed before the scan, so their braces cannot change test scope.
fn scan_file_for_too_many_args(path: &str, content: &str) -> Vec<TooManyArgsFinding> {
    let mut findings = Vec::new();
    let mut depth: i32 = 0;
    let mut test_frames: Vec<i32> = Vec::new();
    let mut pending_cfg_test = false;
    let mut file_is_test = false;
    let mut attribute: Option<(usize, i32, String)> = None;
    let structural = structural_source(content);
    let original_lines = content.lines().collect::<Vec<_>>();

    for (index, line) in structural.lines().enumerate() {
        let trimmed = line.trim_start();
        let starts_attribute =
            attribute.is_none() && (trimmed.starts_with("#[") || trimmed.starts_with("#!["));
        if starts_attribute {
            attribute = Some((index + 1, 0, String::new()));
        }

        let is_attribute = attribute.is_some();
        if let Some((start_line, bracket_depth, text)) = &mut attribute {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(line);
            *bracket_depth += line.matches('[').count() as i32;
            *bracket_depth -= line.matches(']').count() as i32;

            if *bracket_depth == 0 {
                let compact = text
                    .chars()
                    .filter(|character| !character.is_whitespace())
                    .collect::<String>();
                let is_item_cfg_test = compact == "#[cfg(test)]";
                let is_file_cfg_test = compact == "#![cfg(test)]";
                if test_frames.is_empty() && !file_is_test && line_forbids_too_many_args(text) {
                    findings.push(TooManyArgsFinding {
                        path: path.to_string(),
                        line: *start_line,
                        text: original_lines[*start_line - 1].to_string(),
                    });
                }
                if is_file_cfg_test {
                    file_is_test = true;
                } else if is_item_cfg_test {
                    pending_cfg_test = true;
                }
                attribute = None;
            }
        }

        let opens = line.matches('{').count() as i32;
        let closes = line.matches('}').count() as i32;

        if pending_cfg_test {
            if opens > 0 {
                test_frames.push(depth + 1);
                pending_cfg_test = false;
            } else if !is_attribute && !trimmed.is_empty() && !trimmed.starts_with("//") {
                pending_cfg_test = false;
            }
        }
        depth += opens - closes;

        while let Some(&frame) = test_frames.last() {
            if depth < frame {
                test_frames.pop();
            } else {
                break;
            }
        }
    }

    findings
}

/// Return staged paths that also have unstaged changes, preserving staged order.
fn partially_staged_files(staged: &[String], unstaged: &[String]) -> Vec<String> {
    let unstaged = unstaged.iter().map(String::as_str).collect::<HashSet<_>>();
    staged
        .iter()
        .filter(|path| unstaged.contains(path.as_str()))
        .cloned()
        .collect()
}

/// Decide the ban check from collected findings.
fn evaluate_too_many_args(findings: &[TooManyArgsFinding]) -> CheckOutcome {
    if findings.is_empty() {
        return CheckOutcome::Passed {
            output: String::new(),
        };
    }
    let body = findings
        .iter()
        .map(|finding| {
            format!(
                "  {}:{}: {}",
                finding.path,
                finding.line,
                finding.text.trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    CheckOutcome::Failed {
        output: format!(
            "Forbidden `#[allow(clippy::too_many_arguments)]` \
             (group the arguments into a struct instead):\n{body}\n"
        ),
    }
}

// ---------------------------------------------------------------------------
// Imperative shell — processes, filesystem, orchestration
// ---------------------------------------------------------------------------

/// Run the pipeline, or manage the hooks and return.
pub fn run(args: &LintArgs) -> Result<()> {
    if args.install_hooks {
        return hooks::install_hooks();
    }
    if args.uninstall_hooks {
        return hooks::uninstall_hooks();
    }
    if args.hooks_status {
        return hooks::show_status();
    }

    let fix = args.fix || args.staged_only;

    // Read and validate the staged list before any check can repair a file.
    // Restaging a partially staged file would add its unstaged changes too.
    let staged_files = if args.staged_only {
        let staged = collect_changed_rust_files(STAGED_RUST_DIFF_ARGS, "list staged Rust files")?;
        let unstaged =
            collect_changed_rust_files(UNSTAGED_RUST_DIFF_ARGS, "list unstaged Rust files")?;
        let partial = partially_staged_files(&staged, &unstaged);
        if !partial.is_empty() {
            return Err(eyre!(
                "staged lint fix refused: these Rust files also have unstaged changes:\n  {}\nstage or discard each complete file before retrying",
                partial.join("\n  ")
            ));
        }
        Some(staged)
    } else {
        None
    };

    let log_path = resolve_log_path()?;
    let mut log_file = fs::File::create(&log_path)?;

    let mut failed_check: Option<String> = None;

    for check in CHECKS {
        if should_skip(check.id, args) {
            continue;
        }

        let result = match check.id {
            CheckId::FileLength => {
                let files = collect_file_lengths()?;
                CheckResult {
                    name: check.name.to_string(),
                    outcome: evaluate_file_lengths(&files, MAX_FILE_LINES),
                }
            }
            CheckId::TooManyArgsAllow => {
                let findings = collect_too_many_args()?;
                CheckResult {
                    name: check.name.to_string(),
                    outcome: evaluate_too_many_args(&findings),
                }
            }
            _ => {
                let overrides: Option<Vec<&str>> = if fix { fix_args(check.id) } else { None };
                let name = match &overrides {
                    Some(effective) => check_display_name(check.program, effective),
                    None => check.name.to_string(),
                };
                run_check(check, name, overrides.as_deref())?
            }
        };

        write!(log_file, "{}", format_log_entry(&result))?;

        match result.outcome {
            CheckOutcome::Skipped => {
                if args.verbose {
                    println!("[skip] {} (not installed)", result.name);
                }
            }
            CheckOutcome::Passed { ref output } => {
                if args.verbose {
                    print!("{output}");
                }
            }
            CheckOutcome::Failed { ref output } => {
                print!("{output}");
                failed_check = Some(result.name.clone());
                break;
            }
        }
    }

    if let Some(name) = failed_check {
        println!("\nlint failed at: {name}");
        println!("log: {log_path}");
        drop(log_file);
        std::process::exit(1);
    }

    if let Some(files) = staged_files {
        restage_files(&files)?;
    }

    Ok(())
}

/// Spawn one check and read its result.
fn run_check(check: &Check, name: String, override_args: Option<&[&str]>) -> Result<CheckResult> {
    let args: &[&str] = override_args.unwrap_or(check.args);

    let output = Command::new(check.program).args(args).output();

    let (success, text) = match output {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            (output.status.success(), text)
        }
        // A missing program is reported the same way a failing one is, so
        // `is_tool_not_found` can still turn it into a skip for an optional check.
        Err(error) => (false, format!("failed to run {}: {error}\n", check.program)),
    };

    let outcome = determine_outcome(success, text, check.optional);
    Ok(CheckResult { name, outcome })
}

/// The absolute path of the log file inside `target/`.
fn resolve_log_path() -> Result<String> {
    let target_dir = std::env::current_dir()?.join("target");
    fs::create_dir_all(&target_dir)?;
    Ok(target_dir.join(LOG_NAME).to_string_lossy().into_owned())
}

/// Every directory holding Rust source this task judges.
fn source_roots() -> Result<Vec<PathBuf>> {
    let mut roots = Vec::new();
    for entry in fs::read_dir("crates").wrap_err("failed to read crates directory")? {
        let entry = entry.wrap_err("failed to read an entry in crates directory")?;
        let src = entry.path().join("src");
        if src.is_dir() {
            roots.push(src);
        }
    }
    let xtask_src = PathBuf::from("xtask/src");
    if xtask_src.is_dir() {
        roots.push(xtask_src);
    }
    roots.sort();
    Ok(roots)
}

/// Collect every `.rs` file under one directory, deepest last.
fn rust_files_under(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir)
        .wrap_err_with(|| format!("failed to read source directory {}", dir.display()))?
    {
        let entry =
            entry.wrap_err_with(|| format!("failed to read an entry in {}", dir.display()))?;
        entries.push(entry.path());
    }
    entries.sort();
    for path in entries {
        if path.is_dir() {
            rust_files_under(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// Every Rust source file in the workspace, in a settled order.
fn workspace_rust_files() -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for root in source_roots()? {
        rust_files_under(&root, &mut files)?;
    }
    Ok(files)
}

/// Read the line count of every source file.
fn collect_file_lengths() -> Result<Vec<(String, usize)>> {
    let mut results = Vec::new();
    for path in workspace_rust_files()? {
        let content = fs::read_to_string(&path)?;
        results.push((path.display().to_string(), content.lines().count()));
    }
    Ok(results)
}

/// Scan every source file for forbidden allows.
fn collect_too_many_args() -> Result<Vec<TooManyArgsFinding>> {
    let mut findings = Vec::new();
    for path in workspace_rust_files()? {
        let content = fs::read_to_string(&path)?;
        findings.extend(scan_file_for_too_many_args(
            &path.display().to_string(),
            &content,
        ));
    }
    Ok(findings)
}

/// Run a Git diff query and return its Rust paths.
fn collect_changed_rust_files(args: &[&str], action: &str) -> Result<Vec<String>> {
    let output = Command::new("git")
        .args(args)
        .output()
        .wrap_err_with(|| format!("failed to run git to {action}"))?;
    if !output.status.success() {
        return Err(eyre!(
            "git failed to {action} (status {}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let listing = String::from_utf8_lossy(&output.stdout);
    Ok(listing
        .lines()
        .filter(|line| line.ends_with(".rs"))
        .map(String::from)
        .collect())
}

/// Stage the repaired files again, so the commit carries the repair.
fn restage_files(files: &[String]) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    let mut args = vec!["add".to_string(), "--".to_string()];
    args.extend(files.iter().cloned());
    let output = Command::new("git")
        .args(&args)
        .output()
        .wrap_err("failed to run git to restage repaired Rust files")?;
    if !output.status.success() {
        return Err(eyre!(
            "git failed to restage repaired Rust files (status {}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}
