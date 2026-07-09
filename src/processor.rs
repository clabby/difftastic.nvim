//! Processing difftastic output into display-ready format.
//!
//! This module transforms parsed difftastic data into aligned side-by-side display rows
//! suitable for rendering in Neovim's diff viewer. It handles line alignment, filler lines,
//! highlight computation, and hunk detection for navigation.
//!
//! ## Processing Flow
//!
//! 1. The [`process_file`] function dispatches to the appropriate handler based on file status
//! 2. For created/deleted files, all lines are treated as additions/deletions
//! 3. For changed files, the pre-computed `aligned_lines` from difftastic guides row alignment
//! 4. Highlights are computed by analyzing the change regions and merging adjacent regions
//!
//! ## Highlight Strategy
//!
//! The highlight computation aims to provide useful visual feedback:
//!
//! - Full-line highlight: Used when an entire line is new/deleted, or when changes
//!   cover all non-whitespace content
//! - Partial highlight: Used when only specific regions of a line changed, showing
//!   exactly which characters differ
//! - Merged regions: Adjacent change regions separated only by whitespace are merged
//!   for cleaner visual presentation

use crate::difftastic::{Change, Chunk, DifftFile, Status};
use mlua::prelude::*;
use smallvec::SmallVec;
use std::collections::HashMap;
use std::path::PathBuf;

/// Most lines have 0-2 highlight regions; inline storage avoids heap allocation.
type Highlights = SmallVec<[HighlightRegion; 2]>;

/// A highlight region within a line, specified by column range.
///
/// Represents a contiguous span of characters that should be highlighted
/// in the diff viewer to indicate changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HighlightRegion {
    /// Start column (0-indexed, inclusive).
    pub start: u32,

    /// End column (exclusive), or -1 to indicate full-line highlight.
    ///
    /// Using -1 as a sentinel value allows the Lua side to easily detect
    /// when the entire line should be highlighted without needing to know
    /// the actual line length.
    pub end: i32,
}

impl HighlightRegion {
    /// Creates a highlight region for the full display line.
    #[inline]
    #[must_use]
    fn full_line() -> Self {
        Self { start: 0, end: -1 }
    }

    /// Creates a highlight region for a specific column range.
    #[inline]
    #[must_use]
    fn columns(start: u32, end: u32) -> Self {
        Self {
            start,
            end: i32::try_from(end).unwrap_or(i32::MAX),
        }
    }
}

/// One side (left or right) of a diff row for display.
///
/// Contains the line content, whether it's a filler (placeholder) line,
/// and the regions to highlight within the line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Side {
    /// The text content of this line.
    ///
    /// Empty string for filler lines.
    pub content: String,

    /// Whether this is a filler (placeholder) line.
    ///
    /// Filler lines are inserted to maintain row alignment when one side
    /// has content but the other doesn't (e.g., for pure additions or deletions).
    pub is_filler: bool,

    /// Regions within the line to highlight as changed.
    ///
    /// Empty for unchanged lines and filler lines. Uses SmallVec to avoid
    /// heap allocation for the common case of 0-2 highlights per line.
    pub highlights: Highlights,
}

impl Side {
    /// Creates a new side with the given properties.
    #[inline]
    fn new(content: String, is_filler: bool, highlights: Highlights) -> Self {
        Self {
            content,
            is_filler,
            highlights,
        }
    }

    /// Creates a filler (placeholder) side.
    ///
    /// Filler sides have no content and no highlights. They're used to
    /// maintain alignment when the other side has content.
    #[inline]
    #[must_use]
    fn filler() -> Self {
        Self::new(String::new(), true, Highlights::new())
    }

    /// Creates a side with content and full-line highlighting.
    ///
    /// Used for lines that are entirely new (in created files or additions)
    /// or entirely removed (in deleted files or deletions).
    #[inline]
    #[must_use]
    fn with_full_highlight(content: String) -> Self {
        Self::new(
            content,
            false,
            smallvec::smallvec![HighlightRegion::full_line()],
        )
    }
}

/// A single row in the diff display.
///
/// Each row contains both left (old) and right (new) sides, which may be:
/// - Both with content: A modified line showing old and new versions
/// - Left with content, right filler: A deleted line
/// - Left filler, right with content: An added line
/// - Both unchanged: Context line (no highlights)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The left side (old/before version) of this row.
    pub left: Side,

    /// The right side (new/after version) of this row.
    pub right: Side,
}

/// A processed file ready for display in the diff viewer.
///
/// Contains all the information needed to render a file's diff in Neovim:
/// file metadata, the aligned rows for display, and navigation aids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayFile {
    pub path: PathBuf,

    /// Original path for moved/renamed files, if any.
    ///
    /// When present, `path` is the destination (new path) and `moved_from`
    /// is the source (old path).
    pub moved_from: Option<PathBuf>,

    /// The detected programming language.
    pub language: String,

    pub status: Status,

    /// Count of added lines (for display in file list).
    pub additions: u32,

    /// Count of deleted lines (for display in file list).
    pub deletions: u32,

    /// The aligned rows for side-by-side display.
    pub rows: Vec<Row>,

    /// Row indices (0-indexed) where hunks start.
    ///
    /// Used for navigation commands like "jump to next hunk".
    pub hunk_starts: Vec<u32>,

    /// Original line number mapping: `(left_line, right_line)` for each display row.
    ///
    /// `None` means filler line. Line numbers are 0-indexed into the source file.
    /// Used for "goto file" navigation to jump from diff view to actual file location.
    pub aligned_lines: Vec<(Option<u32>, Option<u32>)>,
}

/// Processes a difftastic file into display-ready format.
///
/// Main entry point that dispatches to handlers based on file status:
/// - Created files: all `new_lines` become additions (right side only)
/// - Deleted files: all `old_lines` become deletions (left side only)
/// - Changed files: uses `aligned_lines` to pair up lines from both versions
///
/// The `stats` parameter provides line-based diff stats from the VCS (additions, deletions).
/// If `None`, stats are computed from the file content.
#[must_use]
pub fn process_file(
    file: DifftFile,
    old_lines: Vec<String>,
    new_lines: Vec<String>,
    stats: Option<(u32, u32)>,
) -> DisplayFile {
    match file.status {
        Status::Created => process_created(file, new_lines, stats),
        Status::Deleted => process_deleted(file, old_lines, stats),
        Status::Changed | Status::Unchanged => process_changed(file, &old_lines, &new_lines, stats),
    }
}

/// Processes a newly created file.
///
/// All lines appear on the right side with full-line highlighting,
/// with filler lines on the left side.
fn process_created(
    file: DifftFile,
    new_lines: Vec<String>,
    stats: Option<(u32, u32)>,
) -> DisplayFile {
    let num_lines = new_lines.len();
    let rows: Vec<Row> = new_lines
        .into_iter()
        .map(|line| Row {
            left: Side::filler(),
            right: Side::with_full_highlight(line),
        })
        .collect();

    // For created files: left is always None, right maps 0..n
    let aligned_lines: Vec<(Option<u32>, Option<u32>)> =
        (0..num_lines).map(|i| (None, Some(i as u32))).collect();

    let (additions, deletions) = stats.unwrap_or((rows.len() as u32, 0));
    let hunk_starts = if rows.is_empty() { vec![] } else { vec![0] };

    DisplayFile {
        path: file.path,
        moved_from: None,
        language: file.language,
        status: file.status,
        additions,
        deletions,
        rows,
        hunk_starts,
        aligned_lines,
    }
}

/// Processes a deleted file.
///
/// All lines appear on the left side with full-line highlighting,
/// with filler lines on the right side.
fn process_deleted(
    file: DifftFile,
    old_lines: Vec<String>,
    stats: Option<(u32, u32)>,
) -> DisplayFile {
    let num_lines = old_lines.len();
    let rows: Vec<Row> = old_lines
        .into_iter()
        .map(|line| Row {
            left: Side::with_full_highlight(line),
            right: Side::filler(),
        })
        .collect();

    // For deleted files: left maps 0..n, right is always None
    let aligned_lines: Vec<(Option<u32>, Option<u32>)> =
        (0..num_lines).map(|i| (Some(i as u32), None)).collect();

    let (additions, deletions) = stats.unwrap_or((0, rows.len() as u32));
    let hunk_starts = if rows.is_empty() { vec![] } else { vec![0] };

    DisplayFile {
        path: file.path,
        moved_from: None,
        language: file.language,
        status: file.status,
        additions,
        deletions,
        rows,
        hunk_starts,
        aligned_lines,
    }
}

/// Change info for a line: the changes slice for highlight computation.
type ChangeInfo<'a> = &'a [Change];

/// Extracts change information from chunks into lookup maps.
///
/// Returns `(lhs_changes, rhs_changes)` hashmaps keyed by line number
/// for efficient lookup during row processing.
#[allow(clippy::type_complexity)]
fn extract_changes(
    chunks: &[Chunk],
) -> (HashMap<u32, ChangeInfo<'_>>, HashMap<u32, ChangeInfo<'_>>) {
    // Pre-calculate capacity hint from total diff lines
    let capacity: usize = chunks.iter().map(|c| c.len()).sum();
    let mut lhs_changes: HashMap<u32, ChangeInfo<'_>> = HashMap::with_capacity(capacity);
    let mut rhs_changes: HashMap<u32, ChangeInfo<'_>> = HashMap::with_capacity(capacity);

    for chunk in chunks {
        for diff_line in chunk {
            if let Some(side) = &diff_line.lhs {
                lhs_changes.insert(side.line_number, &side.changes);
            }
            if let Some(side) = &diff_line.rhs {
                rhs_changes.insert(side.line_number, &side.changes);
            }
        }
    }

    (lhs_changes, rhs_changes)
}

/// Processes a changed (modified) file.
///
/// Uses the pre-computed `aligned_lines` from difftastic to create
/// properly aligned rows. Computes highlights based on the change
/// information in the chunks.
fn process_changed(
    file: DifftFile,
    old_lines: &[String],
    new_lines: &[String],
    stats: Option<(u32, u32)>,
) -> DisplayFile {
    let (lhs_changes, rhs_changes) = extract_changes(&file.chunks);
    let num_rows = file.aligned_lines.len();

    let mut rows = Vec::with_capacity(num_rows);
    let mut hunk_starts = Vec::new();
    let mut in_hunk = false;

    for (row_idx, (lhs_ln, rhs_ln)) in file.aligned_lines.iter().enumerate() {
        // Get content for each side (using line number as 0-indexed into lines)
        let left_content = lhs_ln
            .and_then(|ln| old_lines.get(ln as usize))
            .map_or_else(String::new, |s| s.clone());
        let right_content = rhs_ln
            .and_then(|ln| new_lines.get(ln as usize))
            .map_or_else(String::new, |s| s.clone());

        // Get changes for each side
        let left_changes = lhs_ln.and_then(|ln| lhs_changes.get(&ln).copied());
        let right_changes = rhs_ln.and_then(|ln| rhs_changes.get(&ln).copied());

        // Compute highlights based on change information
        let left_highlights = left_changes.map_or_else(Highlights::new, |changes| {
            compute_highlights(&left_content, changes)
        });
        let right_highlights = right_changes.map_or_else(Highlights::new, |changes| {
            compute_highlights(&right_content, changes)
        });

        // Determine if this row is part of a hunk (has changes or fillers)
        let is_changed = lhs_ln.is_none()
            || rhs_ln.is_none()
            || !left_highlights.is_empty()
            || !right_highlights.is_empty();

        // Track hunk boundaries for navigation
        if is_changed && !in_hunk {
            hunk_starts.push(row_idx as u32);
            in_hunk = true;
        } else if !is_changed {
            in_hunk = false;
        }

        rows.push(Row {
            left: Side::new(left_content, lhs_ln.is_none(), left_highlights),
            right: Side::new(right_content, rhs_ln.is_none(), right_highlights),
        });
    }

    // Use VCS stats if available, otherwise default to 0
    let (additions, deletions) = stats.unwrap_or((0, 0));

    DisplayFile {
        path: file.path,
        moved_from: None,
        language: file.language,
        status: file.status,
        additions,
        deletions,
        rows,
        hunk_starts,
        aligned_lines: file.aligned_lines,
    }
}

/// Computes highlight regions for a line based on its changes.
///
/// Difftastic's JSON output reports the structural diff spans as `changes`.
/// Preserve those spans; the renderer adds muted line context separately.
fn compute_highlights(_content: &str, changes: &[Change]) -> Highlights {
    if changes.is_empty() {
        return Highlights::new();
    }

    let mut regions: SmallVec<[(u32, u32); 4]> = changes.iter().map(|c| (c.start, c.end)).collect();
    regions.sort_unstable_by_key(|r| r.0);
    merge_overlapping_regions(&regions)
        .into_iter()
        .map(|(start, end)| HighlightRegion::columns(start, end))
        .collect()
}

fn merge_overlapping_regions(regions: &[(u32, u32)]) -> SmallVec<[(u32, u32); 4]> {
    let mut merged: SmallVec<[(u32, u32); 4]> = SmallVec::with_capacity(regions.len());

    for &(start, end) in regions {
        if let Some((_, last_end)) = merged.last_mut()
            && *last_end >= start
        {
            *last_end = (*last_end).max(end);
            continue;
        }
        merged.push((start, end));
    }

    merged
}

impl IntoLua for HighlightRegion {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        let table = lua.create_table()?;
        table.set("start", self.start)?;
        table.set("end", self.end)?;
        Ok(LuaValue::Table(table))
    }
}

impl IntoLua for Side {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        let table = lua.create_table()?;
        table.set("content", self.content)?;
        table.set("is_filler", self.is_filler)?;

        let highlights = lua.create_table_with_capacity(self.highlights.len(), 0)?;
        for (i, highlight) in self.highlights.into_iter().enumerate() {
            highlights.set(i + 1, highlight.into_lua(lua)?)?;
        }
        table.set("highlights", highlights)?;

        Ok(LuaValue::Table(table))
    }
}

impl IntoLua for Row {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        let table = lua.create_table()?;
        table.set("left", self.left.into_lua(lua)?)?;
        table.set("right", self.right.into_lua(lua)?)?;
        Ok(LuaValue::Table(table))
    }
}

impl IntoLua for DisplayFile {
    fn into_lua(self, lua: &Lua) -> LuaResult<LuaValue> {
        let table = lua.create_table()?;
        table.set("path", self.path.to_string_lossy().as_ref())?;
        table.set(
            "moved_from",
            self.moved_from
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
        )?;
        table.set("language", self.language)?;
        table.set(
            "status",
            match self.status {
                Status::Unchanged => "unchanged",
                Status::Created => "created",
                Status::Deleted => "deleted",
                Status::Changed => "changed",
            },
        )?;
        table.set("additions", self.additions)?;
        table.set("deletions", self.deletions)?;

        let rows = lua.create_table_with_capacity(self.rows.len(), 0)?;
        for (i, row) in self.rows.into_iter().enumerate() {
            rows.set(i + 1, row.into_lua(lua)?)?;
        }
        table.set("rows", rows)?;

        let hunk_starts = lua.create_table_with_capacity(self.hunk_starts.len(), 0)?;
        for (i, hunk_start) in self.hunk_starts.into_iter().enumerate() {
            hunk_starts.set(i + 1, hunk_start)?;
        }
        table.set("hunk_starts", hunk_starts)?;

        let aligned_lines = lua.create_table_with_capacity(self.aligned_lines.len(), 0)?;
        for (i, (left, right)) in self.aligned_lines.into_iter().enumerate() {
            let pair = lua.create_table_with_capacity(2, 0)?;
            pair.set(1, left)?;
            pair.set(2, right)?;
            aligned_lines.set(i + 1, pair)?;
        }
        table.set("aligned_lines", aligned_lines)?;

        Ok(LuaValue::Table(table))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::difftastic::{DiffLine, Highlight, Side as DiffSide};

    /// Helper to create a Change with only start/end (content and highlight empty).
    fn change(start: u32, end: u32) -> Change {
        Change {
            start,
            end,
            content: String::new(),
            highlight: Highlight::Normal,
        }
    }

    /// Helper to create a DiffSide with given line number and changes.
    fn diff_side(line: u32, changes: Vec<Change>) -> DiffSide {
        DiffSide {
            line_number: line,
            changes,
        }
    }

    #[test]
    fn created_file_all_additions() {
        let file = DifftFile {
            path: "new.rs".into(),
            language: "Rust".into(),
            status: Status::Created,
            aligned_lines: vec![],
            chunks: vec![],
        };
        let result = process_file(file, vec![], vec!["a".into(), "b".into()], Some((2, 0)));

        assert_eq!(result.rows.len(), 2);
        assert!(result.rows[0].left.is_filler);
        assert_eq!(result.rows[0].right.content, "a");
        assert!(!result.rows[0].right.is_filler);
        assert_eq!(result.rows[0].right.highlights.len(), 1);
        assert_eq!(result.rows[0].right.highlights[0].start, 0);
        assert_eq!(result.rows[0].right.highlights[0].end, -1);
        assert_eq!(result.additions, 2);
        assert_eq!(result.deletions, 0);
    }

    #[test]
    fn deleted_file_all_deletions() {
        let file = DifftFile {
            path: "old.rs".into(),
            language: "Rust".into(),
            status: Status::Deleted,
            aligned_lines: vec![],
            chunks: vec![],
        };
        let result = process_file(file, vec!["x".into(), "y".into()], vec![], Some((0, 2)));

        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.rows[0].left.content, "x");
        assert!(!result.rows[0].left.is_filler);
        assert!(result.rows[0].right.is_filler);
        assert_eq!(result.additions, 0);
        assert_eq!(result.deletions, 2);
    }

    #[test]
    fn modification_with_aligned_lines() {
        let file = DifftFile {
            path: "mod.rs".into(),
            language: "Rust".into(),
            status: Status::Changed,
            aligned_lines: vec![(Some(0), Some(0)), (Some(1), Some(1)), (Some(2), Some(2))],
            chunks: vec![vec![DiffLine {
                lhs: Some(diff_side(1, vec![change(0, 3)])),
                rhs: Some(diff_side(1, vec![change(0, 6)])),
            }]],
        };
        let result = process_file(
            file,
            vec!["line1".into(), "foo".into(), "line3".into()],
            vec!["line1".into(), "foobar".into(), "line3".into()],
            Some((1, 1)),
        );

        assert_eq!(result.rows.len(), 3);
        assert_eq!(result.rows[1].left.content, "foo");
        assert_eq!(result.rows[1].right.content, "foobar");
        assert!(!result.rows[1].left.highlights.is_empty());
        assert!(!result.rows[1].right.highlights.is_empty());
    }

    #[test]
    fn addition_with_filler_line() {
        let file = DifftFile {
            path: "add.rs".into(),
            language: "Rust".into(),
            status: Status::Changed,
            aligned_lines: vec![(Some(0), Some(0)), (None, Some(1)), (Some(1), Some(2))],
            chunks: vec![vec![DiffLine {
                lhs: None,
                rhs: Some(diff_side(1, vec![change(0, 8)])),
            }]],
        };
        let result = process_file(
            file,
            vec!["line 1".into(), "line 3".into()],
            vec!["line 1".into(), "new line".into(), "line 3".into()],
            Some((1, 0)),
        );

        assert_eq!(result.rows.len(), 3);
        assert!(result.rows[1].left.is_filler);
        assert_eq!(result.rows[1].left.content, "");
        assert_eq!(result.rows[1].right.content, "new line");
        assert!(!result.rows[1].right.is_filler);
    }

    #[test]
    fn deletion_with_filler_line() {
        let file = DifftFile {
            path: "del.rs".into(),
            language: "Rust".into(),
            status: Status::Changed,
            aligned_lines: vec![(Some(0), Some(0)), (Some(1), None), (Some(2), Some(1))],
            chunks: vec![vec![DiffLine {
                lhs: Some(diff_side(1, vec![change(0, 7)])),
                rhs: None,
            }]],
        };
        let result = process_file(
            file,
            vec!["line 1".into(), "deleted".into(), "line 3".into()],
            vec!["line 1".into(), "line 3".into()],
            Some((0, 1)),
        );

        assert_eq!(result.rows.len(), 3);
        assert_eq!(result.rows[1].left.content, "deleted");
        assert!(!result.rows[1].left.is_filler);
        assert!(result.rows[1].right.is_filler);
    }

    #[test]
    fn highlight_empty_changes_is_empty() {
        let highlights = compute_highlights("content", &[]);
        assert!(highlights.is_empty());
    }

    #[test]
    fn highlight_full_coverage_preserves_span() {
        let highlights = compute_highlights("hello", &[change(0, 5)]);
        assert_eq!(highlights.len(), 1);
        assert_eq!(highlights[0].start, 0);
        assert_eq!(highlights[0].end, 5);
    }

    #[test]
    fn highlight_full_coverage_with_indent_preserves_span() {
        let highlights = compute_highlights("  hello  ", &[change(0, 9)]);
        assert_eq!(highlights.len(), 1);
        assert_eq!(highlights[0].start, 0);
        assert_eq!(highlights[0].end, 9);
    }

    #[test]
    fn highlight_partial_coverage() {
        let highlights = compute_highlights("hello world", &[change(0, 5)]);
        assert_eq!(highlights[0].start, 0);
        assert_eq!(highlights[0].end, 5);
    }

    #[test]
    fn highlight_all_non_whitespace_across_spans_preserves_spans() {
        let highlights = compute_highlights("foo bar", &[change(0, 3), change(4, 7)]);
        assert_eq!(highlights.len(), 2);
        assert_eq!(highlights[0].start, 0);
        assert_eq!(highlights[0].end, 3);
        assert_eq!(highlights[1].start, 4);
        assert_eq!(highlights[1].end, 7);
    }

    #[test]
    fn highlight_all_non_whitespace_with_indent_preserves_spans() {
        let highlights = compute_highlights("  foo bar  ", &[change(2, 5), change(6, 9)]);
        assert_eq!(highlights.len(), 2);
        assert_eq!(highlights[0].start, 2);
        assert_eq!(highlights[0].end, 5);
        assert_eq!(highlights[1].start, 6);
        assert_eq!(highlights[1].end, 9);
    }

    #[test]
    fn highlight_partial_spans_do_not_merge_across_whitespace() {
        let highlights = compute_highlights("foo bar baz", &[change(0, 3), change(4, 7)]);
        assert_eq!(highlights.len(), 2);
        assert_eq!(highlights[0].start, 0);
        assert_eq!(highlights[0].end, 3);
        assert_eq!(highlights[1].start, 4);
        assert_eq!(highlights[1].end, 7);
    }

    #[test]
    fn highlight_numeric_literal_change_preserves_literal_span() {
        let highlights = compute_highlights("M.bg_opacity = 0.38", &[change(15, 19)]);
        assert_eq!(highlights.len(), 1);
        assert_eq!(highlights[0].start, 15);
        assert_eq!(highlights[0].end, 19);
    }

    #[test]
    fn highlight_no_merge_across_non_whitespace() {
        let highlights = compute_highlights("foo.bar", &[change(0, 3), change(4, 7)]);
        assert_eq!(highlights.len(), 2);
    }

    #[test]
    fn expansion_multiline_to_single() {
        let file = DifftFile {
            path: "expand.rs".into(),
            language: "Rust".into(),
            status: Status::Changed,
            aligned_lines: vec![
                (Some(0), Some(0)),
                (None, Some(1)),
                (None, Some(2)),
                (None, Some(3)),
                (None, Some(4)),
            ],
            chunks: vec![vec![
                DiffLine {
                    lhs: Some(diff_side(0, vec![change(0, 16)])),
                    rhs: Some(diff_side(0, vec![change(0, 6)])),
                },
                DiffLine {
                    lhs: None,
                    rhs: Some(diff_side(1, vec![change(0, 6)])),
                },
                DiffLine {
                    lhs: None,
                    rhs: Some(diff_side(2, vec![change(0, 6)])),
                },
                DiffLine {
                    lhs: None,
                    rhs: Some(diff_side(3, vec![change(0, 6)])),
                },
                DiffLine {
                    lhs: None,
                    rhs: Some(diff_side(4, vec![change(0, 1)])),
                },
            ]],
        };

        let old_lines = vec!["Self { a, b, c }".into()];
        let new_lines = vec![
            "Self {".into(),
            "    a,".into(),
            "    b,".into(),
            "    c,".into(),
            "}".into(),
        ];

        let result = process_file(file, old_lines, new_lines, None);

        assert_eq!(result.rows.len(), 5);
        assert_eq!(result.rows[0].left.content, "Self { a, b, c }");
        assert_eq!(result.rows[0].right.content, "Self {");
        assert!(result.rows[1].left.is_filler);
        assert_eq!(result.rows[1].right.content, "    a,");
    }

    #[test]
    fn contraction_single_to_multiline() {
        let file = DifftFile {
            path: "contract.rs".into(),
            language: "Rust".into(),
            status: Status::Changed,
            aligned_lines: vec![
                (Some(0), None),
                (Some(1), None),
                (Some(2), None),
                (Some(3), Some(0)),
                (Some(4), None),
            ],
            chunks: vec![vec![
                DiffLine {
                    lhs: Some(diff_side(0, vec![change(0, 6)])),
                    rhs: None,
                },
                DiffLine {
                    lhs: Some(diff_side(1, vec![change(0, 6)])),
                    rhs: None,
                },
                DiffLine {
                    lhs: Some(diff_side(2, vec![change(0, 6)])),
                    rhs: None,
                },
                DiffLine {
                    lhs: Some(diff_side(3, vec![change(0, 6)])),
                    rhs: Some(diff_side(0, vec![change(0, 16)])),
                },
                DiffLine {
                    lhs: Some(diff_side(4, vec![change(0, 1)])),
                    rhs: None,
                },
            ]],
        };

        let old_lines = vec![
            "Self {".into(),
            "    a,".into(),
            "    b,".into(),
            "    c,".into(),
            "}".into(),
        ];
        let new_lines = vec!["Self { a, b, c }".into()];

        let result = process_file(file, old_lines, new_lines, None);

        assert_eq!(result.rows.len(), 5);
        assert_eq!(result.rows[0].left.content, "Self {");
        assert!(result.rows[0].right.is_filler);
        assert_eq!(result.rows[3].left.content, "    c,");
        assert_eq!(result.rows[3].right.content, "Self { a, b, c }");
    }

    #[test]
    fn hunk_starts_detected_correctly() {
        let file = DifftFile {
            path: "hunks.rs".into(),
            language: "Rust".into(),
            status: Status::Changed,
            aligned_lines: vec![
                (Some(0), Some(0)), // unchanged
                (Some(1), Some(1)), // changed
                (Some(2), Some(2)), // changed
                (Some(3), Some(3)), // unchanged
                (Some(4), Some(4)), // unchanged
                (None, Some(5)),    // added - new hunk
            ],
            chunks: vec![
                vec![
                    DiffLine {
                        lhs: Some(diff_side(1, vec![change(0, 3)])),
                        rhs: Some(diff_side(1, vec![change(0, 3)])),
                    },
                    DiffLine {
                        lhs: Some(diff_side(2, vec![change(0, 3)])),
                        rhs: Some(diff_side(2, vec![change(0, 3)])),
                    },
                ],
                vec![DiffLine {
                    lhs: None,
                    rhs: Some(diff_side(5, vec![change(0, 5)])),
                }],
            ],
        };

        let old_lines = vec![
            "aaa".into(),
            "bbb".into(),
            "ccc".into(),
            "ddd".into(),
            "eee".into(),
        ];
        let new_lines = vec![
            "aaa".into(),
            "BBB".into(),
            "CCC".into(),
            "ddd".into(),
            "eee".into(),
            "fff".into(),
        ];

        let result = process_file(file, old_lines, new_lines, None);

        // Should have two hunks: one starting at row 1, one at row 5
        assert_eq!(result.hunk_starts.len(), 2);
        assert_eq!(result.hunk_starts[0], 1);
        assert_eq!(result.hunk_starts[1], 5);
    }

    #[test]
    fn aligned_lines_created_file() {
        let file = DifftFile {
            path: "new.rs".into(),
            language: "Rust".into(),
            status: Status::Created,
            aligned_lines: vec![],
            chunks: vec![],
        };
        let result = process_file(file, vec![], vec!["a".into(), "b".into(), "c".into()], None);

        // Created files: left is always None, right maps 0..n
        assert_eq!(result.aligned_lines.len(), 3);
        assert_eq!(result.aligned_lines[0], (None, Some(0)));
        assert_eq!(result.aligned_lines[1], (None, Some(1)));
        assert_eq!(result.aligned_lines[2], (None, Some(2)));
    }

    #[test]
    fn aligned_lines_deleted_file() {
        let file = DifftFile {
            path: "old.rs".into(),
            language: "Rust".into(),
            status: Status::Deleted,
            aligned_lines: vec![],
            chunks: vec![],
        };
        let result = process_file(file, vec!["x".into(), "y".into()], vec![], None);

        // Deleted files: left maps 0..n, right is always None
        assert_eq!(result.aligned_lines.len(), 2);
        assert_eq!(result.aligned_lines[0], (Some(0), None));
        assert_eq!(result.aligned_lines[1], (Some(1), None));
    }

    #[test]
    fn aligned_lines_changed_file_preserved() {
        let aligned = vec![
            (Some(0), Some(0)),
            (Some(1), Some(1)),
            (None, Some(2)), // Addition
            (Some(2), Some(3)),
        ];
        let file = DifftFile {
            path: "mod.rs".into(),
            language: "Rust".into(),
            status: Status::Changed,
            aligned_lines: aligned.clone(),
            chunks: vec![],
        };
        let result = process_file(
            file,
            vec!["a".into(), "b".into(), "c".into()],
            vec!["a".into(), "b".into(), "new".into(), "c".into()],
            None,
        );

        // Changed files: aligned_lines should be passed through from difftastic
        assert_eq!(result.aligned_lines, aligned);
    }

    #[test]
    fn aligned_lines_with_deletion_filler() {
        let aligned = vec![
            (Some(0), Some(0)),
            (Some(1), None), // Deletion - right side is filler
            (Some(2), Some(1)),
        ];
        let file = DifftFile {
            path: "del.rs".into(),
            language: "Rust".into(),
            status: Status::Changed,
            aligned_lines: aligned.clone(),
            chunks: vec![],
        };
        let result = process_file(
            file,
            vec!["a".into(), "deleted".into(), "b".into()],
            vec!["a".into(), "b".into()],
            None,
        );

        assert_eq!(result.aligned_lines, aligned);
        // Row 1 should have right side as filler (None in aligned_lines)
        assert_eq!(result.aligned_lines[1], (Some(1), None));
    }
}
