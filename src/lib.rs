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
use std::collections::HashMap;
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

    let commit = String::from_utf8_lossy(&output.stdout).trim().to_string();
    // Valid git commit hash is 40 hex characters
    (commit.len() == 40 && commit.chars().all(|c| c.is_ascii_hexdigit())).then_some(commit)
}

/// Gets diff stats from jj by translating revsets to git commits.
/// For colocated repos, uses `git diff --numstat` for accurate stats.
fn jj_diff_stats(revset: &str) -> FileStats {
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
            let files = run_git_diff(&[range]).map_err(LuaError::RuntimeError)?;
            let stats = git_diff_stats(&[range]);
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
    let display_files: Vec<_> = match (&mode, vcs) {
        (DiffMode::Range(range), "git") => {
            let (old_ref, new_ref) = parse_git_range(range);
            files
                .into_par_iter()
                .map(|file| {
                    let file_stats = stats.get(&file.path).copied();
                    let old_lines = into_lines(git_file_content(&old_ref, &file.path));
                    let new_lines = into_lines(git_file_content(&new_ref, &file.path));
                    processor::process_file(file, old_lines, new_lines, file_stats)
                })
                .collect()
        }
        (DiffMode::Range(range), _) => {
            let old_ref = format!("roots({range})-");
            let new_ref = format!("heads({range})");
            files
                .into_par_iter()
                .map(|file| {
                    let file_stats = stats.get(&file.path).copied();
                    let old_lines = into_lines(jj_file_content(&old_ref, &file.path));
                    let new_lines = into_lines(jj_file_content(&new_ref, &file.path));
                    processor::process_file(file, old_lines, new_lines, file_stats)
                })
                .collect()
        }
        (DiffMode::Unstaged, "git") => files
            .into_par_iter()
            .map(|file| {
                let file_stats = stats.get(&file.path).copied();
                let old_lines = into_lines(git_index_content(&file.path));
                let new_lines = into_lines(working_tree_content_for_vcs(&file.path, "git"));
                processor::process_file(file, old_lines, new_lines, file_stats)
            })
            .collect(),
        (DiffMode::Unstaged, _) => files
            .into_par_iter()
            .map(|file| {
                let file_stats = stats.get(&file.path).copied();
                let old_lines = into_lines(jj_file_content("@", &file.path));
                let new_lines = into_lines(working_tree_content_for_vcs(&file.path, "jj"));
                processor::process_file(file, old_lines, new_lines, file_stats)
            })
            .collect(),
        (DiffMode::Staged, "git") => files
            .into_par_iter()
            .map(|file| {
                let file_stats = stats.get(&file.path).copied();
                let old_lines = into_lines(git_file_content("HEAD", &file.path));
                let new_lines = into_lines(git_index_content(&file.path));
                processor::process_file(file, old_lines, new_lines, file_stats)
            })
            .collect(),
        (DiffMode::Staged, _) => files
            .into_par_iter()
            .map(|file| {
                let file_stats = stats.get(&file.path).copied();
                let old_lines = into_lines(jj_file_content("@-", &file.path));
                let new_lines = into_lines(jj_file_content("@", &file.path));
                processor::process_file(file, old_lines, new_lines, file_stats)
            })
            .collect(),
    };

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
}
