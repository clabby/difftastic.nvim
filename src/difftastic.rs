//! Types and parsing for difftastic JSON output.
//!
//! [Difftastic](https://difftastic.wilfred.me.uk/) is a structural diff tool that understands
//! programming language syntax. When the `DFT_DISPLAY=json` environment variable is set,
//! difftastic outputs machine-readable JSON describing the differences between files.
//!
//! This module provides the types to deserialize that JSON output into Rust structs
//! that can be processed by the [`crate::processor`] module.
//!
//! ## JSON Format
//!
//! Difftastic outputs JSON in two formats depending on the VCS:
//!
//! - **jj format**: A JSON array of file objects: `[{...}, {...}]`
//! - **git format**: Newline-separated JSON objects: `{...}\n{...}`
//!
//! The [`parse`] function handles both formats transparently.
//!
//! ## Example JSON Structure
//!
//! ```json
//! {
//!   "path": "src/lib.rs",
//!   "language": "Rust",
//!   "status": "changed",
//!   "aligned_lines": [[0, 0], [1, 1], [null, 2]],
//!   "chunks": [[
//!     {
//!       "lhs": {"line_number": 1, "changes": [{"start": 0, "end": 5, "content": "hello", "highlight": "string"}]},
//!       "rhs": {"line_number": 1, "changes": [{"start": 0, "end": 5, "content": "world", "highlight": "string"}]}
//!     }
//!   ]]
//! }
//! ```

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Unchanged,
    Created,
    Deleted,
    Changed,
}

/// A file entry from difftastic's JSON output.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DifftFile {
    pub path: PathBuf,
    pub language: String,
    pub status: Status,
    /// Pre-computed line alignment: `(lhs_line, rhs_line)` pairs, `None` = filler.
    #[serde(default)]
    pub aligned_lines: Vec<(Option<u32>, Option<u32>)>,
    /// Groups of related changes (hunks).
    #[serde(default)]
    pub chunks: Vec<Chunk>,
}

/// A chunk (hunk) of changes within a file.
///
/// A chunk represents a contiguous group of related changes, similar to a
/// "hunk" in unified diff format. Each chunk contains one or more [`DiffLine`]
/// entries describing the specific changes.
pub type Chunk = Vec<DiffLine>;

/// A single diff line entry, which may have content on the left side, right side, or both.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct DiffLine {
    /// The left-hand side (old/before) content, if any.
    pub lhs: Option<Side>,

    /// The right-hand side (new/after) content, if any.
    pub rhs: Option<Side>,
}

/// One side (left or right) of a diff line.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Side {
    /// The 0-indexed line number in the original file.
    pub line_number: u32,

    /// The changed regions within this line.
    ///
    /// Each [`Change`] describes a contiguous region of text that differs
    /// between the old and new versions. Multiple changes can exist on
    /// the same line (e.g., when multiple parts of a line were modified).
    pub changes: Vec<Change>,
}

/// A specific change region within a line.
///
/// Represents a contiguous span of text that has changed, along with
/// syntax highlighting information from difftastic.
///
/// # Byte Offsets
///
/// The `start` and `end` fields are byte offsets within the line, not
/// character offsets. This is important for non-ASCII text.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Change {
    /// Start byte offset within the line (0-indexed, inclusive).
    pub start: u32,

    /// End byte offset within the line (exclusive).
    pub end: u32,

    /// The text content of this change region.
    pub content: String,

    /// Syntax highlight type from difftastic's parser.
    ///
    /// Common values include:
    /// - `"keyword"` - Language keywords (fn, let, if, etc.)
    /// - `"string"` - String literals
    /// - `"comment"` - Comments
    /// - `"type"` - Type names
    /// - `"normal"` - Regular code without special highlighting
    ///
    /// This is always present in difftastic JSON output.
    pub highlight: Highlight,
}

/// Token highlight categories emitted by difftastic JSON.
///
/// These variants map directly to difftastic's `display::json::Highlight` enum.
/// The value appears in each [`Change`] as the `highlight` field and indicates
/// the token class for syntax-aware rendering.
///
/// - `Delimiter`: punctuation and structural delimiters (`{}`, `()`, `,`, `;`)
/// - `Normal`: plain identifiers/text without a special token class
/// - `String`: string literal content
/// - `Type`: type names and type-like symbols
/// - `Comment`: line or block comments
/// - `Keyword`: language keywords (`fn`, `let`, `if`, etc.)
/// - `TreeSitterError`: parser error spans reported by tree-sitter
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Highlight {
    Delimiter,
    Normal,
    String,
    Type,
    Comment,
    Keyword,
    TreeSitterError,
}

/// Parses difftastic JSON output into a list of file entries.
///
/// Handles two formats:
/// - jj format: JSON array `[{...}, {...}]`
/// - git format: newline-separated JSON objects
pub fn parse(json: &str) -> Result<Vec<DifftFile>, serde_json::Error> {
    // Try array format first (jj outputs this format)
    if let Ok(files) = serde_json::from_str::<Vec<DifftFile>>(json) {
        return Ok(files);
    }

    // Fall back to newline-separated JSON objects (git outputs this format)
    json.lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_array() {
        let json = "[]";
        let files = parse(json).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn parse_created_file() {
        let json = r#"[{
            "path": "src/new.rs",
            "language": "Rust",
            "status": "created",
            "chunks": []
        }]"#;

        let files = parse(json).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, PathBuf::from("src/new.rs"));
        assert_eq!(files[0].language, "Rust");
        assert_eq!(files[0].status, Status::Created);
        assert!(files[0].chunks.is_empty());
    }

    #[test]
    fn parse_deleted_file() {
        let json = r#"[{
            "path": "src/old.rs",
            "language": "Rust",
            "status": "deleted",
            "chunks": []
        }]"#;

        let files = parse(json).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, Status::Deleted);
    }

    #[test]
    fn parse_modified_file_with_changes() {
        let json = r#"[{
            "path": "src/lib.rs",
            "language": "Rust",
            "status": "changed",
            "chunks": [[
                {
                    "lhs": {"line_number": 5, "changes": [{"start": 0, "end": 10, "content": "old_code", "highlight": "normal"}]},
                    "rhs": {"line_number": 5, "changes": [{"start": 0, "end": 12, "content": "new_code_!!", "highlight": "normal"}]}
                }
            ]]
        }]"#;

        let files = parse(json).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].status, Status::Changed);
        assert_eq!(files[0].chunks.len(), 1);
        assert_eq!(files[0].chunks[0].len(), 1);

        let diff_line = &files[0].chunks[0][0];
        assert!(diff_line.lhs.is_some());
        assert!(diff_line.rhs.is_some());

        let lhs = diff_line.lhs.as_ref().unwrap();
        assert_eq!(lhs.line_number, 5);
        assert_eq!(lhs.changes.len(), 1);
        assert_eq!(lhs.changes[0].start, 0);
        assert_eq!(lhs.changes[0].end, 10);
    }

    #[test]
    fn parse_addition_only() {
        let json = r#"[{
            "path": "src/lib.rs",
            "language": "Rust",
            "status": "changed",
            "chunks": [[
                {"rhs": {"line_number": 10, "changes": [{"start": 0, "end": 20, "content": "new line", "highlight": "normal"}]}}
            ]]
        }]"#;

        let files = parse(json).unwrap();
        let diff_line = &files[0].chunks[0][0];
        assert!(diff_line.lhs.is_none());
        assert!(diff_line.rhs.is_some());
    }

    #[test]
    fn parse_deletion_only() {
        let json = r#"[{
            "path": "src/lib.rs",
            "language": "Rust",
            "status": "changed",
            "chunks": [[
                {"lhs": {"line_number": 10, "changes": [{"start": 0, "end": 20, "content": "deleted line", "highlight": "normal"}]}}
            ]]
        }]"#;

        let files = parse(json).unwrap();
        let diff_line = &files[0].chunks[0][0];
        assert!(diff_line.lhs.is_some());
        assert!(diff_line.rhs.is_none());
    }

    #[test]
    fn parse_multiple_chunks() {
        let json = r#"[{
            "path": "src/lib.rs",
            "language": "Rust",
            "status": "changed",
            "chunks": [
                [{"lhs": {"line_number": 5, "changes": []}}],
                [{"rhs": {"line_number": 50, "changes": []}}]
            ]
        }]"#;

        let files = parse(json).unwrap();
        assert_eq!(files[0].chunks.len(), 2);
    }

    #[test]
    fn parse_empty_changes() {
        let json = r#"[{
            "path": "src/lib.rs",
            "language": "Rust",
            "status": "changed",
            "chunks": [[
                {"lhs": {"line_number": 5, "changes": []}, "rhs": {"line_number": 5, "changes": []}}
            ]]
        }]"#;

        let files = parse(json).unwrap();
        let diff_line = &files[0].chunks[0][0];
        assert!(diff_line.lhs.as_ref().unwrap().changes.is_empty());
        assert!(diff_line.rhs.as_ref().unwrap().changes.is_empty());
    }

    #[test]
    fn parse_multiple_changes_per_line() {
        let json = r#"[{
            "path": "src/lib.rs",
            "language": "Rust",
            "status": "changed",
            "chunks": [[
                {
                    "rhs": {
                        "line_number": 5,
                        "changes": [
                            {"start": 0, "end": 3, "content": "let", "highlight": "keyword"},
                            {"start": 4, "end": 7, "content": "foo", "highlight": "normal"},
                            {"start": 10, "end": 13, "content": "bar", "highlight": "string"}
                        ]
                    }
                }
            ]]
        }]"#;

        let files = parse(json).unwrap();
        let rhs = files[0].chunks[0][0].rhs.as_ref().unwrap();
        assert_eq!(rhs.changes.len(), 3);
        assert_eq!(rhs.changes[0].highlight, Highlight::Keyword);
        assert_eq!(rhs.changes[2].highlight, Highlight::String);
    }

    #[test]
    fn parse_newline_separated_objects() {
        // Git format: newline-separated JSON objects
        let json = r#"{"path":"a.rs","language":"Rust","status":"changed","chunks":[]}
{"path":"b.rs","language":"Rust","status":"created","chunks":[]}"#;

        let files = parse(json).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, PathBuf::from("a.rs"));
        assert_eq!(files[1].path, PathBuf::from("b.rs"));
    }

    #[test]
    fn parse_with_aligned_lines() {
        let json = r#"[{
            "path": "src/lib.rs",
            "language": "Rust",
            "status": "changed",
            "aligned_lines": [[0, 0], [1, null], [2, 1]],
            "chunks": []
        }]"#;

        let files = parse(json).unwrap();
        assert_eq!(files[0].aligned_lines.len(), 3);
        assert_eq!(files[0].aligned_lines[0], (Some(0), Some(0)));
        assert_eq!(files[0].aligned_lines[1], (Some(1), None));
        assert_eq!(files[0].aligned_lines[2], (Some(2), Some(1)));
    }

    #[test]
    fn parse_unchanged_status() {
        let json = r#"[{
            "path": "src/lib.rs",
            "language": "Rust",
            "status": "unchanged",
            "chunks": []
        }]"#;

        let files = parse(json).unwrap();
        assert_eq!(files[0].status, Status::Unchanged);
    }
}
