//! # difftastic-nvim
//!
//! A Neovim plugin for displaying difftastic diffs in a side-by-side viewer.
//!
//! This crate provides Lua bindings for parsing [difftastic](https://difftastic.wilfred.me.uk/)
//! JSON output and processing it into a display-ready format. It supports both
//! [jj](https://github.com/martinvonz/jj) and [git](https://git-scm.com/) version control systems.
//!
//! ## Architecture
//!
//! The crate is organized into three modules:
//!
//! - `difftastic` - Types and parsing for difftastic's JSON output format
//! - `processor` - Transforms parsed data into aligned side-by-side display rows
//! - `lib` (this module) - Lua bindings and VCS integration
//!
//! ## Usage from Lua
//!
//! ```lua
//! local difft = require("difftastic_nvim")
//!
//! -- Get diff for a jj revision
//! local result = difft.run_diff("@", "jj")
//!
//! -- Get diff for a git commit
//! local result = difft.run_diff("HEAD", "git")
//!
//! -- Get diff for a git commit range
//! local result = difft.run_diff("main..feature", "git")
//! ```
//!
//! ## Environment Variables
//!
//! This crate sets the following environment variables when invoking difftastic:
//!
//! - `DFT_DISPLAY=json` - Enables JSON output mode
//! - `DFT_UNSTABLE=yes` - Enables unstable features (required for JSON output)

use mlua::prelude::*;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

mod difftastic;
mod processor;

/// Splits file content into individual lines, or empty vector if `None`.
#[inline]
fn into_lines(content: Option<String>) -> Vec<String> {
    content
        .map(|c| c.lines().map(String::from).collect())
        .unwrap_or_default()
}

/// Fetches file content from jj at a specific revision via `jj file show`.
/// Returns `None` if the command fails or the file doesn't exist.
fn jj_file_content(revset: &str, path: &Path) -> Option<String> {
    Command::new("jj")
        .args(["file", "show", "-r", revset])
        .arg(path)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Fetches file content from git at a specific commit via `git show`.
/// Returns `None` if the command fails or the file doesn't exist.
fn git_file_content(commit: &str, path: &Path) -> Option<String> {
    Command::new("git")
        .arg("show")
        .arg(format!("{commit}:{}", path.display()))
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Fetches file content from git index (staged version).
/// Returns `None` if the command fails or the file doesn't exist in the index.
fn git_index_content(path: &Path) -> Option<String> {
    Command::new("git")
        .arg("show")
        .arg(format!(":{}", path.display()))
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Gets the git repository root directory.
fn git_root() -> Option<PathBuf> {
    Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
}

/// Gets the jj repository root directory.
fn jj_root() -> Option<PathBuf> {
    Command::new("jj")
        .args(["root"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
}

/// Stats for a single file: (additions, deletions).
type FileStats = HashMap<PathBuf, (u32, u32)>;

/// Gets diff stats from git using `--numstat`.
/// Output format: "additions\tdeletions\tpath"
///
/// Pass additional arguments to customize the diff:
/// - `&["HEAD^..HEAD"]` for a commit range
/// - `&[]` for unstaged changes (working tree vs index)
/// - `&["--cached"]` for staged changes (index vs HEAD)
fn git_diff_stats(extra_args: &[&str]) -> FileStats {
    let mut args = vec!["diff", "--numstat"];
    args.extend(extra_args);

    let output = Command::new("git").args(&args).output().ok();

    let Some(output) = output.filter(|o| o.status.success()) else {
        return HashMap::new();
    };

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let add = parts.next()?.parse().ok()?;
            let del = parts.next()?.parse().ok()?;
            let path = parts.next()?;
            Some((PathBuf::from(path), (add, del)))
        })
        .collect()
}

/// Gets diff stats for jj uncommitted changes.
fn jj_diff_stats_uncommitted() -> FileStats {
    // jj diff without -r shows uncommitted changes; use git for stats
    // For uncommitted changes, we compare working copy to the current commit
    let output = Command::new("jj").args(["diff", "--stat"]).output().ok();

    // jj --stat output is different, so we just return empty for now
    // The diff will still work, just without inline stats
    let _ = output;
    HashMap::new()
}

/// Translates a jj revset to a git commit hash.
/// Uses `jj log -r <revset> --no-graph -T 'commit_id'`.
fn jj_to_git_commit(revset: &str) -> Option<String> {
    let output = Command::new("jj")
        .args(["log", "-r", revset, "--no-graph", "-T", "commit_id"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let commits: Vec<_> = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();

    if commits.len() != 1 {
        return None;
    }

    let commit = commits[0].to_string();
    // Valid git commit hash is 40 hex characters
    (commit.len() == 40 && commit.chars().all(|c| c.is_ascii_hexdigit())).then_some(commit)
}

/// Parses a jj range of the form `A..B` into `(A, B)`.
/// Returns `None` for non-range revsets.
#[inline]
fn parse_jj_range(revset: &str) -> Option<(String, String)> {
    let (old, new) = revset.split_once("..")?;
    if old.is_empty() || new.is_empty() {
        return None;
    }
    Some((old.to_string(), new.to_string()))
}

/// Gets diff stats from jj by translating revsets to git commits.
/// For colocated repos, uses `git diff --numstat` for accurate stats.
fn jj_diff_stats(revset: &str) -> FileStats {
    if let Some((old_rev, new_rev)) = parse_jj_range(revset) {
        let old_commit = jj_to_git_commit(&old_rev);
        let new_commit = jj_to_git_commit(&new_rev);
        return match (old_commit, new_commit) {
            (Some(old), Some(new)) => git_diff_stats(&[&format!("{old}..{new}")]),
            _ => HashMap::new(),
        };
    }

    let old_commit = jj_to_git_commit(&format!("roots({revset})-"));
    let new_commit = jj_to_git_commit(&format!("heads({revset})"));

    match (old_commit, new_commit) {
        (Some(old), Some(new)) => git_diff_stats(&[&format!("{old}..{new}")]),
        (None, Some(new)) => git_diff_stats(&[&format!("{new}^..{new}")]),
        _ => HashMap::new(),
    }
}

/// Runs difftastic via jj and parses the JSON output.
/// Executes `jj diff -r <revset> --tool difft` with JSON output mode enabled.
fn run_jj_diff(revset: &str) -> Result<Vec<difftastic::DifftFile>, String> {
    let output = Command::new("jj")
        .args(["diff", "-r", revset, "--tool", "difft"])
        .env("DFT_DISPLAY", "json")
        .env("DFT_UNSTABLE", "yes")
        .output()
        .map_err(|e| format!("Failed to run jj: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("jj command failed: {stderr}"));
    }

    difftastic::parse(&String::from_utf8_lossy(&output.stdout))
        .map_err(|e| format!("Failed to parse difftastic JSON: {e}"))
}

/// Runs difftastic via jj for uncommitted changes (working copy).
/// Executes `jj diff` with no revision argument.
fn run_jj_diff_uncommitted() -> Result<Vec<difftastic::DifftFile>, String> {
    let output = Command::new("jj")
        .args(["diff", "--tool", "difft"])
        .env("DFT_DISPLAY", "json")
        .env("DFT_UNSTABLE", "yes")
        .output()
        .map_err(|e| format!("Failed to run jj: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("jj command failed: {stderr}"));
    }

    difftastic::parse(&String::from_utf8_lossy(&output.stdout))
        .map_err(|e| format!("Failed to parse difftastic JSON: {e}"))
}

/// Runs difftastic via git and parses the JSON output.
/// Executes `git diff` with difftastic as the external diff tool.
///
/// Pass additional arguments to customize the diff:
/// - `&["HEAD^..HEAD"]` for a commit range
/// - `&[]` for unstaged changes (working tree vs index)
/// - `&["--cached"]` for staged changes (index vs HEAD)
fn run_git_diff(extra_args: &[&str]) -> Result<Vec<difftastic::DifftFile>, String> {
    let mut args = vec!["-c", "diff.external=difft", "diff"];
    args.extend(extra_args);

    let output = Command::new("git")
        .args(&args)
        .env("DFT_DISPLAY", "json")
        .env("DFT_UNSTABLE", "yes")
        .output()
        .map_err(|e| format!("Failed to run git: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git command failed: {stderr}"));
    }

    difftastic::parse(&String::from_utf8_lossy(&output.stdout))
        .map_err(|e| format!("Failed to parse difftastic JSON: {e}"))
}

/// Gets the merge-base of two git refs.
fn git_merge_base(a: &str, b: &str) -> Option<String> {
    Command::new("git")
        .args(["merge-base", a, b])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}

/// Expands diff display paths for renames/moves into concrete old/new paths.
///
/// Handles common formats:
/// - `old/path => new/path`
/// - `old/path -> new/path`
/// - `src/{old => new}.rs`
fn split_display_path(path: &Path) -> (PathBuf, PathBuf) {
    let raw = path.to_string_lossy();

    if let (Some(open), Some(close)) = (raw.find('{'), raw.rfind('}'))
        && close > open
    {
        let prefix = &raw[..open];
        let suffix = &raw[(close + 1)..];
        let inner = &raw[(open + 1)..close];

        for arrow in [" => ", " -> "] {
            if let Some((lhs, rhs)) = inner.split_once(arrow)
                && !lhs.trim().is_empty()
                && !rhs.trim().is_empty()
            {
                let old_path = format!("{prefix}{}{suffix}", lhs.trim());
                let new_path = format!("{prefix}{}{suffix}", rhs.trim());
                return (PathBuf::from(old_path), PathBuf::from(new_path));
            }
        }
    }

    for arrow in [" => ", " -> "] {
        if let Some((lhs, rhs)) = raw.split_once(arrow)
            && !lhs.trim().is_empty()
            && !rhs.trim().is_empty()
        {
            return (PathBuf::from(lhs.trim()), PathBuf::from(rhs.trim()));
        }
    }

    (path.to_path_buf(), path.to_path_buf())
}

fn prepare_file_for_display(
    file: &mut difftastic::DifftFile,
    stats: &FileStats,
) -> (Option<(u32, u32)>, PathBuf, PathBuf, Option<PathBuf>) {
    let file_stats = stats.get(&file.path).copied();
    let (old_path, new_path) = split_display_path(&file.path);

    let moved_from = if old_path != new_path {
        file.path = new_path.clone();
        file.status = difftastic::Status::Created;
        Some(old_path.clone())
    } else {
        None
    };

    (file_stats, old_path, new_path, moved_from)
}

fn process_prepared_file(
    file: difftastic::DifftFile,
    old_lines: Vec<String>,
    new_lines: Vec<String>,
    file_stats: Option<(u32, u32)>,
    moved_from: Option<PathBuf>,
) -> processor::DisplayFile {
    let mut display = processor::process_file(file, old_lines, new_lines, file_stats);
    display.moved_from = moved_from;
    display
}

fn parse_jj_summary_rename(line: &str) -> Option<(PathBuf, PathBuf)> {
    let renamed = line.trim().strip_prefix("R ")?;
    let (old_path, new_path) = split_display_path(Path::new(renamed));
    (old_path != new_path).then_some((old_path, new_path))
}

fn parse_jj_summary_renames(output: &str) -> HashMap<PathBuf, PathBuf> {
    output
        .lines()
        .filter_map(parse_jj_summary_rename)
        .map(|(old_path, new_path)| (new_path, old_path))
        .collect()
}

fn parse_git_name_status_rename(line: &str) -> Option<(PathBuf, PathBuf)> {
    let mut parts = line.trim().split('\t');
    let status = parts.next()?;
    if !status.starts_with('R') {
        return None;
    }

    let old_path = parts.next()?.trim();
    let new_path = parts.next()?.trim();
    if old_path.is_empty() || new_path.is_empty() {
        return None;
    }

    Some((PathBuf::from(old_path), PathBuf::from(new_path)))
}

fn parse_git_name_status_renames(output: &str) -> HashMap<PathBuf, PathBuf> {
    output
        .lines()
        .filter_map(parse_git_name_status_rename)
        .map(|(old_path, new_path)| (new_path, old_path))
        .collect()
}

fn git_rename_map(mode: &DiffMode) -> HashMap<PathBuf, PathBuf> {
    let mut cmd = Command::new("git");
    cmd.args(["diff", "--name-status", "-M"]);

    match mode {
        DiffMode::Range(range) => {
            cmd.arg(range);
        }
        DiffMode::Unstaged => {}
        DiffMode::Staged => {
            cmd.arg("--cached");
        }
    }

    let output = cmd.output().ok();
    let Some(output) = output.filter(|o| o.status.success()) else {
        return HashMap::new();
    };

    parse_git_name_status_renames(&String::from_utf8_lossy(&output.stdout))
}

fn jj_rename_map(mode: &DiffMode) -> HashMap<PathBuf, PathBuf> {
    let mut cmd = Command::new("jj");
    cmd.arg("diff");

    match mode {
        DiffMode::Range(revset) => {
            cmd.arg("-r").arg(revset);
        }
        DiffMode::Unstaged => {}
        DiffMode::Staged => {
            cmd.args(["-r", "@"]); // mirror staged fallback semantics in this plugin
        }
    }

    let output = cmd.arg("--summary").output().ok();
    let Some(output) = output.filter(|o| o.status.success()) else {
        return HashMap::new();
    };

    parse_jj_summary_renames(&String::from_utf8_lossy(&output.stdout))
}

/// Parses a git commit range into `(old_commit, new_commit)` references.
///
/// Handles single commits, `A..B` ranges, and `A...B` (merge-base) ranges.
#[inline]
fn parse_git_range(range: &str) -> (String, String) {
    if let Some((a, b)) = range.split_once("...") {
        let base = git_merge_base(a, b).unwrap_or_else(|| format!("{a}^"));
        (base, b.to_string())
    } else if let Some((old, new)) = range.split_once("..") {
        (old.to_string(), new.to_string())
    } else {
        (format!("{range}^"), range.to_string())
    }
}

/// The type of diff to perform.
enum DiffMode {
    /// A commit range (e.g., "HEAD^..HEAD" for git, "@" for jj).
    Range(String),
    /// Unstaged changes: working tree vs index (git) or working copy vs @ (jj).
    Unstaged,
    /// Staged changes: index vs HEAD (git only, jj falls back to @).
    Staged,
}

/// Fetches file content from the working tree, using the appropriate VCS root.
fn working_tree_content_for_vcs(path: &Path, vcs: &str) -> Option<String> {
    let root = if vcs == "git" { git_root() } else { jj_root() }?;
    std::fs::read_to_string(root.join(path)).ok()
}

/// Unified implementation for running difftastic with any diff mode.
/// Handles git and jj VCS, fetches file contents, and processes files in parallel.
fn run_diff_impl(lua: &Lua, mode: DiffMode, vcs: &str) -> LuaResult<LuaTable> {
    // Get files and stats based on mode and VCS
    let (files, stats) = match (&mode, vcs) {
        (DiffMode::Range(range), "git") => {
            let (old_ref, new_ref) = parse_git_range(range);
            let git_range = format!("{old_ref}..{new_ref}");
            let files = run_git_diff(&[&git_range]).map_err(LuaError::RuntimeError)?;
            let stats = git_diff_stats(&[&git_range]);
            (files, stats)
        }
        (DiffMode::Range(range), _) => {
            let files = run_jj_diff(range).map_err(LuaError::RuntimeError)?;
            let stats = jj_diff_stats(range);
            (files, stats)
        }
        (DiffMode::Unstaged, "git") => {
            let files = run_git_diff(&[]).map_err(LuaError::RuntimeError)?;
            let stats = git_diff_stats(&[]);
            (files, stats)
        }
        (DiffMode::Unstaged, _) => {
            let files = run_jj_diff_uncommitted().map_err(LuaError::RuntimeError)?;
            let stats = jj_diff_stats_uncommitted();
            (files, stats)
        }
        (DiffMode::Staged, "git") => {
            let files = run_git_diff(&["--cached"]).map_err(LuaError::RuntimeError)?;
            let stats = git_diff_stats(&["--cached"]);
            (files, stats)
        }
        (DiffMode::Staged, _) => {
            // jj doesn't have a staging area concept, so show current revision
            let files = run_jj_diff("@").map_err(LuaError::RuntimeError)?;
            let stats = jj_diff_stats("@");
            (files, stats)
        }
    };

    // Process files based on mode and VCS
    let mut display_files: Vec<_> = match (&mode, vcs) {
        (DiffMode::Range(range), "git") => {
            let (old_ref, new_ref) = parse_git_range(range);
            files
                .into_par_iter()
                .map(|mut file| {
                    let (file_stats, old_path, new_path, moved_from) =
                        prepare_file_for_display(&mut file, &stats);
                    let old_lines = into_lines(git_file_content(&old_ref, &old_path));
                    let new_lines = into_lines(git_file_content(&new_ref, &new_path));
                    process_prepared_file(file, old_lines, new_lines, file_stats, moved_from)
                })
                .collect()
        }
        (DiffMode::Range(range), _) => {
            let (old_ref, new_ref) = parse_jj_range(range)
                .unwrap_or_else(|| (format!("roots({range})-"), format!("heads({range})")));
            files
                .into_par_iter()
                .map(|mut file| {
                    let (file_stats, old_path, new_path, moved_from) =
                        prepare_file_for_display(&mut file, &stats);
                    let old_lines = into_lines(jj_file_content(&old_ref, &old_path));
                    let new_lines = into_lines(jj_file_content(&new_ref, &new_path));
                    process_prepared_file(file, old_lines, new_lines, file_stats, moved_from)
                })
                .collect()
        }
        (DiffMode::Unstaged, "git") => files
            .into_par_iter()
            .map(|mut file| {
                let (file_stats, old_path, new_path, moved_from) =
                    prepare_file_for_display(&mut file, &stats);
                let old_lines = into_lines(git_index_content(&old_path));
                let new_lines = into_lines(working_tree_content_for_vcs(&new_path, "git"));
                process_prepared_file(file, old_lines, new_lines, file_stats, moved_from)
            })
            .collect(),
        (DiffMode::Unstaged, _) => files
            .into_par_iter()
            .map(|mut file| {
                let (file_stats, old_path, new_path, moved_from) =
                    prepare_file_for_display(&mut file, &stats);
                let old_lines = into_lines(jj_file_content("@-", &old_path));
                let new_lines = into_lines(working_tree_content_for_vcs(&new_path, "jj"));
                process_prepared_file(file, old_lines, new_lines, file_stats, moved_from)
            })
            .collect(),
        (DiffMode::Staged, "git") => files
            .into_par_iter()
            .map(|mut file| {
                let (file_stats, old_path, new_path, moved_from) =
                    prepare_file_for_display(&mut file, &stats);
                let old_lines = into_lines(git_file_content("HEAD", &old_path));
                let new_lines = into_lines(git_index_content(&new_path));
                process_prepared_file(file, old_lines, new_lines, file_stats, moved_from)
            })
            .collect(),
        (DiffMode::Staged, _) => files
            .into_par_iter()
            .map(|mut file| {
                let (file_stats, old_path, new_path, moved_from) =
                    prepare_file_for_display(&mut file, &stats);
                let old_lines = into_lines(jj_file_content("@-", &old_path));
                let new_lines = into_lines(jj_file_content("@", &new_path));
                process_prepared_file(file, old_lines, new_lines, file_stats, moved_from)
            })
            .collect(),
    };

    let renames = if vcs == "git" {
        git_rename_map(&mode)
    } else {
        jj_rename_map(&mode)
    };
    if !renames.is_empty() {
        let old_paths: HashSet<PathBuf> = renames.values().cloned().collect();

        display_files = display_files
            .into_iter()
            .filter_map(|mut file| {
                if let Some(old_path) = renames.get(&file.path) {
                    file.moved_from = Some(old_path.clone());
                    file.status = difftastic::Status::Created;
                }

                if file.status == difftastic::Status::Deleted && old_paths.contains(&file.path) {
                    return None;
                }

                Some(file)
            })
            .collect();
    }

    let files_table = lua.create_table()?;
    for (i, file) in display_files.into_iter().enumerate() {
        files_table.set(i + 1, file.into_lua(lua)?)?;
    }

    let result = lua.create_table()?;
    result.set("files", files_table)?;
    Ok(result)
}

/// Runs difftastic for a commit range.
fn run_diff(lua: &Lua, (range, vcs): (String, String)) -> LuaResult<LuaTable> {
    run_diff_impl(lua, DiffMode::Range(range), &vcs)
}

/// Runs difftastic for unstaged changes.
fn run_diff_unstaged(lua: &Lua, vcs: String) -> LuaResult<LuaTable> {
    run_diff_impl(lua, DiffMode::Unstaged, &vcs)
}

/// Runs difftastic for staged changes.
fn run_diff_staged(lua: &Lua, vcs: String) -> LuaResult<LuaTable> {
    run_diff_impl(lua, DiffMode::Staged, &vcs)
}

/// Creates the Lua module exports. Called by mlua when loaded via `require("difftastic_nvim")`.
#[mlua::lua_module]
fn difftastic_nvim(lua: &Lua) -> LuaResult<LuaTable> {
    let exports = lua.create_table()?;
    exports.set(
        "run_diff",
        lua.create_function(|lua, args: (String, String)| run_diff(lua, args))?,
    )?;
    exports.set(
        "run_diff_unstaged",
        lua.create_function(|lua, vcs: String| run_diff_unstaged(lua, vcs))?,
    )?;
    exports.set(
        "run_diff_staged",
        lua.create_function(|lua, vcs: String| run_diff_staged(lua, vcs))?,
    )?;
    Ok(exports)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_into_lines_with_content() {
        let lines = into_lines(Some("line1\nline2\nline3".to_string()));
        assert_eq!(lines, vec!["line1", "line2", "line3"]);
    }

    #[test]
    fn test_into_lines_empty() {
        let lines = into_lines(None);
        assert!(lines.is_empty());
    }

    #[test]
    fn test_into_lines_single_line() {
        let lines = into_lines(Some("single".to_string()));
        assert_eq!(lines, vec!["single"]);
    }

    #[test]
    fn test_parse_git_range_single_commit() {
        let (old, new) = parse_git_range("abc123");
        assert_eq!(old, "abc123^");
        assert_eq!(new, "abc123");
    }

    #[test]
    fn test_parse_git_range_double_dot() {
        let (old, new) = parse_git_range("main..feature");
        assert_eq!(old, "main");
        assert_eq!(new, "feature");
    }

    #[test]
    fn test_parse_git_range_empty_left() {
        let (old, new) = parse_git_range("..HEAD");
        assert_eq!(old, "");
        assert_eq!(new, "HEAD");
    }

    #[test]
    fn test_parse_jj_range_double_dot() {
        let (old, new) = parse_jj_range("main@origin..@").unwrap();
        assert_eq!(old, "main@origin");
        assert_eq!(new, "@");
    }

    #[test]
    fn test_parse_jj_range_non_range() {
        assert!(parse_jj_range("@").is_none());
    }

    #[test]
    fn test_split_display_path_plain() {
        let (old, new) = split_display_path(Path::new("src/lib.rs"));
        assert_eq!(old, PathBuf::from("src/lib.rs"));
        assert_eq!(new, PathBuf::from("src/lib.rs"));
    }

    #[test]
    fn test_split_display_path_arrow() {
        let (old, new) = split_display_path(Path::new("src/old.rs => src/new.rs"));
        assert_eq!(old, PathBuf::from("src/old.rs"));
        assert_eq!(new, PathBuf::from("src/new.rs"));
    }

    #[test]
    fn test_split_display_path_brace() {
        let (old, new) = split_display_path(Path::new("src/{old => new}.rs"));
        assert_eq!(old, PathBuf::from("src/old.rs"));
        assert_eq!(new, PathBuf::from("src/new.rs"));
    }

    #[test]
    fn test_parse_jj_summary_rename_simple() {
        let parsed = parse_jj_summary_rename("R src/old.rs => src/new.rs").unwrap();
        assert_eq!(parsed.0, PathBuf::from("src/old.rs"));
        assert_eq!(parsed.1, PathBuf::from("src/new.rs"));
    }

    #[test]
    fn test_parse_jj_summary_rename_brace() {
        let parsed = parse_jj_summary_rename("R src/{old => new}.rs").unwrap();
        assert_eq!(parsed.0, PathBuf::from("src/old.rs"));
        assert_eq!(parsed.1, PathBuf::from("src/new.rs"));
    }

    #[test]
    fn test_parse_jj_summary_renames_map() {
        let renames = parse_jj_summary_renames("R a.txt => b.txt\nA c.txt\n");
        assert_eq!(
            renames.get(Path::new("b.txt")),
            Some(&PathBuf::from("a.txt"))
        );
        assert!(!renames.contains_key(Path::new("c.txt")));
    }

    #[test]
    fn test_parse_git_name_status_rename() {
        let parsed = parse_git_name_status_rename("R100\tsrc/old.rs\tsrc/new.rs").unwrap();
        assert_eq!(parsed.0, PathBuf::from("src/old.rs"));
        assert_eq!(parsed.1, PathBuf::from("src/new.rs"));
    }

    #[test]
    fn test_parse_git_name_status_renames_map() {
        let renames = parse_git_name_status_renames("R090\ta.txt\tb.txt\nM c.txt\n");
        assert_eq!(
            renames.get(Path::new("b.txt")),
            Some(&PathBuf::from("a.txt"))
        );
        assert!(!renames.contains_key(Path::new("c.txt")));
    }
}
