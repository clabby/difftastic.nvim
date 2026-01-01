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

/// Fetches file content from the working tree.
/// Path should be relative to git root; we resolve it using git rev-parse.
fn working_tree_content(path: &Path) -> Option<String> {
    let root = git_root()?;
    std::fs::read_to_string(root.join(path)).ok()
}

/// Stats for a single file: (additions, deletions).
type FileStats = HashMap<PathBuf, (u32, u32)>;

/// Gets diff stats from git using `--numstat`.
/// Output format: "additions\tdeletions\tpath"
fn git_diff_stats(range: &str) -> FileStats {
    let output = Command::new("git")
        .args(["diff", "--numstat", range])
        .output()
        .ok();

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

/// Gets diff stats for unstaged changes (working tree vs index).
fn git_diff_stats_unstaged() -> FileStats {
    let output = Command::new("git")
        .args(["diff", "--numstat"])
        .output()
        .ok();

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

/// Gets diff stats for staged changes (index vs HEAD).
fn git_diff_stats_staged() -> FileStats {
    let output = Command::new("git")
        .args(["diff", "--cached", "--numstat"])
        .output()
        .ok();

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
    let output = Command::new("jj")
        .args(["diff", "--stat"])
        .output()
        .ok();

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
        (Some(old), Some(new)) => git_diff_stats(&format!("{old}..{new}")),
        (None, Some(new)) => git_diff_stats(&format!("{new}^..{new}")),
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
/// Executes `git diff <commit_range>` with difftastic as the external diff tool.
fn run_git_diff(commit_range: &str) -> Result<Vec<difftastic::DifftFile>, String> {
    let output = Command::new("git")
        .args(["-c", "diff.external=difft", "diff", commit_range])
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

/// Runs difftastic via git for unstaged changes (working tree vs index).
/// Executes `git diff` with no arguments.
fn run_git_diff_unstaged() -> Result<Vec<difftastic::DifftFile>, String> {
    let output = Command::new("git")
        .args(["-c", "diff.external=difft", "diff"])
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

/// Runs difftastic via git for staged changes (index vs HEAD).
/// Executes `git diff --cached`.
fn run_git_diff_staged() -> Result<Vec<difftastic::DifftFile>, String> {
    let output = Command::new("git")
        .args(["-c", "diff.external=difft", "diff", "--cached"])
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

/// Main entry point from Lua. Runs difftastic via the specified VCS, fetches
/// file contents, and processes each file into aligned side-by-side rows.
/// File processing is parallelized for performance.
fn run_diff(lua: &Lua, (range, vcs): (String, String)) -> LuaResult<LuaTable> {
    let files = match vcs.as_str() {
        "git" => run_git_diff(&range),
        _ => run_jj_diff(&range),
    }
    .map_err(LuaError::RuntimeError)?;

    // Get line-based diff stats from VCS
    let stats = if vcs == "git" {
        git_diff_stats(&range)
    } else {
        jj_diff_stats(&range)
    };

    let display_files: Vec<_> = if vcs == "git" {
        let (old_ref, new_ref) = parse_git_range(&range);
        files
            .into_par_iter()
            .map(|file| {
                let file_stats = stats.get(&file.path).copied();
                let old_lines = into_lines(git_file_content(&old_ref, &file.path));
                let new_lines = into_lines(git_file_content(&new_ref, &file.path));
                processor::process_file(file, old_lines, new_lines, file_stats)
            })
            .collect()
    } else {
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
    };

    let files_table = lua.create_table()?;
    for (i, file) in display_files.into_iter().enumerate() {
        files_table.set(i + 1, file.into_lua(lua)?)?;
    }

    let result = lua.create_table()?;
    result.set("files", files_table)?;
    Ok(result)
}

/// Runs difftastic for unstaged changes (working tree vs index).
/// For git: compares working tree to index
/// For jj: compares working copy to current commit
fn run_diff_unstaged(lua: &Lua, vcs: String) -> LuaResult<LuaTable> {
    let files = match vcs.as_str() {
        "git" => run_git_diff_unstaged(),
        _ => run_jj_diff_uncommitted(),
    }
    .map_err(LuaError::RuntimeError)?;

    let stats = if vcs == "git" {
        git_diff_stats_unstaged()
    } else {
        jj_diff_stats_uncommitted()
    };

    let display_files: Vec<_> = if vcs == "git" {
        files
            .into_par_iter()
            .map(|file| {
                let file_stats = stats.get(&file.path).copied();
                // For unstaged: old = index, new = working tree
                let old_lines = into_lines(git_index_content(&file.path));
                let new_lines = into_lines(working_tree_content(&file.path));
                processor::process_file(file, old_lines, new_lines, file_stats)
            })
            .collect()
    } else {
        // For jj, use the current revision as old and working copy content as new
        files
            .into_par_iter()
            .map(|file| {
                let file_stats = stats.get(&file.path).copied();
                let old_lines = into_lines(jj_file_content("@", &file.path));
                let new_lines = into_lines(working_tree_content(&file.path));
                processor::process_file(file, old_lines, new_lines, file_stats)
            })
            .collect()
    };

    let files_table = lua.create_table()?;
    for (i, file) in display_files.into_iter().enumerate() {
        files_table.set(i + 1, file.into_lua(lua)?)?;
    }

    let result = lua.create_table()?;
    result.set("files", files_table)?;
    Ok(result)
}

/// Runs difftastic for staged changes (index vs HEAD).
/// Only supported for git. For jj, this falls back to showing @ changes.
fn run_diff_staged(lua: &Lua, vcs: String) -> LuaResult<LuaTable> {
    let files = match vcs.as_str() {
        "git" => run_git_diff_staged(),
        _ => {
            // jj doesn't have a staging area concept, so show current revision
            run_jj_diff("@")
        }
    }
    .map_err(LuaError::RuntimeError)?;

    let stats = if vcs == "git" {
        git_diff_stats_staged()
    } else {
        jj_diff_stats("@")
    };

    let display_files: Vec<_> = if vcs == "git" {
        files
            .into_par_iter()
            .map(|file| {
                let file_stats = stats.get(&file.path).copied();
                // For staged: old = HEAD, new = index
                let old_lines = into_lines(git_file_content("HEAD", &file.path));
                let new_lines = into_lines(git_index_content(&file.path));
                processor::process_file(file, old_lines, new_lines, file_stats)
            })
            .collect()
    } else {
        // For jj, show @ revision
        files
            .into_par_iter()
            .map(|file| {
                let file_stats = stats.get(&file.path).copied();
                let old_lines = into_lines(jj_file_content("@-", &file.path));
                let new_lines = into_lines(jj_file_content("@", &file.path));
                processor::process_file(file, old_lines, new_lines, file_stats)
            })
            .collect()
    };

    let files_table = lua.create_table()?;
    for (i, file) in display_files.into_iter().enumerate() {
        files_table.set(i + 1, file.into_lua(lua)?)?;
    }

    let result = lua.create_table()?;
    result.set("files", files_table)?;
    Ok(result)
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
