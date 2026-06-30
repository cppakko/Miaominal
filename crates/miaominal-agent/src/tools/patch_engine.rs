use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePatch {
    pub old_path: String,
    pub new_path: String,
    pub is_new_file: bool,
    pub is_deleted: bool,
    pub hunks: Vec<Hunk>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<HunkLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HunkLine {
    Context(String),
    Added(String),
    Removed(String),
    NoNewlineAtEof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchError {
    Parse(String),
    Apply(String),
    NotFound(String),
}

impl std::fmt::Display for PatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Parse(msg) => write!(f, "patch parse error: {msg}"),
            Self::Apply(msg) => write!(f, "patch apply error: {msg}"),
            Self::NotFound(path) => write!(f, "file not found: {path}"),
        }
    }
}

pub type PatchResult<T> = Result<T, PatchError>;

pub fn parse_unified_diff(patch: &str) -> PatchResult<Vec<FilePatch>> {
    let mut files: Vec<FilePatch> = Vec::new();
    let mut current_file: Option<FilePatchBuilder> = None;
    let mut current_hunk: Option<HunkBuilder> = None;

    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix("--- ") {
            if let Some(file) = current_file.take() {
                files.push(file.build(current_hunk.take())?);
                current_hunk = None;
            }
            let (path, _timestamp) = split_path_ts(rest);
            current_file = Some(FilePatchBuilder {
                old_path: strip_diff_prefix(path),
                new_path: String::new(),
                is_new_file: path == "/dev/null",
                is_deleted: false,
                hunks: Vec::new(),
            });
        } else if let Some(rest) = line.strip_prefix("+++ ") {
            let (path, _timestamp) = split_path_ts(rest);
            if let Some(ref mut file) = current_file {
                file.new_path = strip_diff_prefix(path);
                file.is_deleted = path == "/dev/null";
            }
        } else if let Some(header) = line.strip_prefix("@@") {
            if let Some(file) = current_file.as_mut()
                && let Some(hunk) = current_hunk.take()
            {
                file.hunks.push(hunk.build()?);
            }
            current_hunk = Some(HunkBuilder::parse(header)?);
        } else if let Some(hunk) = current_hunk.as_mut() {
            if line == r"\ No newline at end of file" {
                hunk.lines.push(HunkLine::NoNewlineAtEof);
            } else if let Some(content) = line.strip_prefix(' ') {
                hunk.lines.push(HunkLine::Context(content.to_string()));
            } else if let Some(content) = line.strip_prefix('+') {
                hunk.lines.push(HunkLine::Added(content.to_string()));
            } else if let Some(content) = line.strip_prefix('-') {
                hunk.lines.push(HunkLine::Removed(content.to_string()));
            }
        }
    }

    if let Some(file) = current_file.take() {
        files.push(file.build(current_hunk.take())?);
    }

    if files.is_empty() {
        return Err(PatchError::Parse("no file patches found in diff".into()));
    }

    Ok(files)
}

pub fn apply_file_patch(original: &str, hunks: &[Hunk]) -> PatchResult<String> {
    let original_lines: Vec<&str> = if original.is_empty() {
        Vec::new()
    } else {
        original.lines().collect()
    };

    let mut result: Vec<String> = Vec::new();
    let mut orig_pos: usize = 0;
    let mut result_ends_with_newline = original.ends_with('\n');

    for (hunk_idx, hunk) in hunks.iter().enumerate() {
        while orig_pos + 1 < hunk.old_start && orig_pos < original_lines.len() {
            result.push(original_lines[orig_pos].to_string());
            result_ends_with_newline =
                original_line_has_newline(&original_lines, original, orig_pos);
            orig_pos += 1;
        }

        if hunk.old_start > 0 && orig_pos + 1 < hunk.old_start {
            return Err(PatchError::Apply(format!(
                "hunk {hunk_idx}: expected to reach line {} but original ends before",
                hunk.old_start,
            )));
        }

        let orig_hunk_start = orig_pos;
        let mut line_iter = original_lines[orig_pos..].iter();
        let mut lines_consumed: usize = 0;
        let mut previous_line_wrote_output = false;

        for line in &hunk.lines {
            match line {
                HunkLine::Context(expected) => match line_iter.next() {
                    Some(actual) if actual == expected => {
                        let original_line_index = orig_pos + lines_consumed;
                        result.push(actual.to_string());
                        result_ends_with_newline = original_line_has_newline(
                            &original_lines,
                            original,
                            original_line_index,
                        );
                        lines_consumed += 1;
                        previous_line_wrote_output = true;
                    }
                    Some(actual) => {
                        return Err(PatchError::Apply(format!(
                            "hunk {hunk_idx}: context mismatch: expected {}, got {}",
                            describe_line(expected),
                            describe_line(actual),
                        )));
                    }
                    None => {
                        return Err(PatchError::Apply(format!(
                            "hunk {hunk_idx}: context mismatch: expected {}, file ended",
                            describe_line(expected),
                        )));
                    }
                },
                HunkLine::Added(content) => {
                    result.push(content.to_string());
                    result_ends_with_newline = true;
                    previous_line_wrote_output = true;
                }
                HunkLine::Removed(expected) => match line_iter.next() {
                    Some(actual) if actual == expected => {
                        lines_consumed += 1;
                        previous_line_wrote_output = false;
                    }
                    Some(actual) => {
                        return Err(PatchError::Apply(format!(
                            "hunk {hunk_idx}: removal mismatch: expected {}, got {}",
                            describe_line(expected),
                            describe_line(actual),
                        )));
                    }
                    None => {
                        return Err(PatchError::Apply(format!(
                            "hunk {hunk_idx}: removal mismatch: expected {}, file ended",
                            describe_line(expected),
                        )));
                    }
                },
                HunkLine::NoNewlineAtEof => {
                    if previous_line_wrote_output {
                        result_ends_with_newline = false;
                    }
                }
            }
        }

        orig_pos = orig_hunk_start + lines_consumed;
    }

    while orig_pos < original_lines.len() {
        result.push(original_lines[orig_pos].to_string());
        result_ends_with_newline = original_line_has_newline(&original_lines, original, orig_pos);
        orig_pos += 1;
    }

    if result.is_empty() {
        Ok(String::new())
    } else {
        let mut output = result.join("\n");
        if result_ends_with_newline {
            output.push('\n');
        }
        Ok(output)
    }
}

fn original_line_has_newline(original_lines: &[&str], original: &str, line_index: usize) -> bool {
    line_index + 1 < original_lines.len() || original.ends_with('\n')
}

pub fn extract_target_path(file: &FilePatch) -> &str {
    if file.is_deleted || file.new_path.is_empty() {
        &file.old_path
    } else {
        &file.new_path
    }
}

/// Build a summary string describing the patch application result.
pub fn build_summary(
    files: &[FilePatch],
    results: &HashMap<String, PatchResult<Vec<u8>>>,
) -> String {
    let mut lines = Vec::new();
    for file in files {
        let path = extract_target_path(file);
        match results.get(path) {
            Some(Ok(_)) => {
                if file.is_new_file {
                    lines.push(format!("created: {path}"));
                } else if file.is_deleted {
                    lines.push(format!("deleted: {path}"));
                } else {
                    let hunk_count = file.hunks.len();
                    lines.push(format!("patched: {path} ({hunk_count} hunk(s))"));
                }
            }
            Some(Err(PatchError::NotFound(_))) => {
                lines.push(format!("skipped: {path} (not found)"));
            }
            Some(Err(e)) => {
                lines.push(format!("FAILED: {path} - {e}"));
            }
            None => {
                lines.push(format!("unknown: {path}"));
            }
        }
    }
    lines.join("\n")
}

// ── internal builders ──

struct FilePatchBuilder {
    old_path: String,
    new_path: String,
    is_new_file: bool,
    is_deleted: bool,
    hunks: Vec<Hunk>,
}

impl FilePatchBuilder {
    fn build(mut self, last_hunk: Option<HunkBuilder>) -> PatchResult<FilePatch> {
        if let Some(hb) = last_hunk {
            self.hunks.push(hb.build()?);
        }
        let target = if self.is_deleted || self.new_path.is_empty() || self.new_path == "/dev/null" {
            self.old_path.clone()
        } else {
            self.new_path.clone()
        };
        if target.is_empty() || target == "/dev/null" {
            return Err(PatchError::Parse(
                "cannot determine target file path from diff headers".into(),
            ));
        }
        Ok(FilePatch {
            old_path: self.old_path,
            new_path: self.new_path,
            is_new_file: self.is_new_file,
            is_deleted: self.is_deleted,
            hunks: self.hunks,
        })
    }
}

struct HunkBuilder {
    old_start: usize,
    old_count: usize,
    new_start: usize,
    new_count: usize,
    lines: Vec<HunkLine>,
}

impl HunkBuilder {
    fn parse(header: &str) -> PatchResult<Self> {
        let inner = header.trim_start().trim_end();
        let parts: Vec<&str> = inner.splitn(2, '@').collect();
        let ranges = parts
            .first()
            .ok_or_else(|| PatchError::Parse("invalid hunk header".into()))?;
        let ranges = ranges.trim();

        let range_parts: Vec<&str> = ranges.split_whitespace().collect();
        if range_parts.len() < 2 {
            return Err(PatchError::Parse(format!("invalid hunk header: {header}")));
        }

        let old = range_parts[0].trim_start_matches('-');
        let new = range_parts[1].trim_start_matches('+');

        let (old_start, old_count) = parse_range(old)?;
        let (new_start, new_count) = parse_range(new)?;

        Ok(Self {
            old_start,
            old_count,
            new_start,
            new_count,
            lines: Vec::new(),
        })
    }

    fn build(self) -> PatchResult<Hunk> {
        Ok(Hunk {
            old_start: self.old_start,
            old_count: self.old_count,
            new_start: self.new_start,
            new_count: self.new_count,
            lines: self.lines,
        })
    }
}

fn parse_range(s: &str) -> PatchResult<(usize, usize)> {
    let parts: Vec<&str> = s.split(',').collect();
    let start: usize = parts[0]
        .parse()
        .map_err(|_| PatchError::Parse(format!("invalid hunk range: {s}")))?;
    let count: usize = if parts.len() > 1 {
        parts[1]
            .parse()
            .map_err(|_| PatchError::Parse(format!("invalid hunk count: {s}")))?
    } else {
        1
    };
    Ok((start, count))
}

fn strip_diff_prefix(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed == "/dev/null" {
        return trimmed.to_string();
    }
    if let Some(rest) = trimmed.strip_prefix("a/") {
        rest.to_string()
    } else if let Some(rest) = trimmed.strip_prefix("b/") {
        rest.to_string()
    } else {
        trimmed.to_string()
    }
}

fn split_path_ts(input: &str) -> (&str, Option<&str>) {
    let parts: Vec<&str> = input.splitn(2, '\t').collect();
    (parts[0], parts.get(1).copied())
}

fn describe_line(line: &str) -> String {
    let mut trailing_spaces = 0;
    let mut trailing_tabs = 0;
    for character in line.chars().rev() {
        match character {
            ' ' => trailing_spaces += 1,
            '\t' => trailing_tabs += 1,
            _ => break,
        }
    }
    let mut description = format!("{line:?}");
    if trailing_spaces > 0 {
        description.push_str(&format!(" (trailing spaces: {trailing_spaces})"));
    }
    if trailing_tabs > 0 {
        description.push_str(&format!(" (trailing tabs: {trailing_tabs})"));
    }
    description
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_diff() {
        let patch = "\
--- a/file.txt
+++ b/file.txt
@@ -1,3 +1,3 @@
 line1
-old
+new
 line3
";
        let files = parse_unified_diff(patch).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].old_path, "file.txt");
        assert_eq!(files[0].new_path, "file.txt");
        assert_eq!(files[0].hunks.len(), 1);

        let hunk = &files[0].hunks[0];
        assert_eq!(hunk.old_start, 1);
        assert_eq!(hunk.old_count, 3);
        assert_eq!(hunk.new_start, 1);
        assert_eq!(hunk.new_count, 3);
        assert_eq!(hunk.lines.len(), 4);
        assert_eq!(hunk.lines[0], HunkLine::Context("line1".into()));
        assert_eq!(hunk.lines[1], HunkLine::Removed("old".into()));
        assert_eq!(hunk.lines[2], HunkLine::Added("new".into()));
        assert_eq!(hunk.lines[3], HunkLine::Context("line3".into()));
    }

    #[test]
    fn applies_simple_hunk() {
        let original = "line1\nold\nline3\n";
        let patch = "\
--- a/file.txt
+++ b/file.txt
@@ -1,3 +1,3 @@
 line1
-old
+new
 line3
";
        let files = parse_unified_diff(patch).unwrap();
        let result = apply_file_patch(original, &files[0].hunks).unwrap();
        assert_eq!(result, "line1\nnew\nline3\n");
    }

    #[test]
    fn applies_multi_hunk() {
        let original = "a\nb\nc\nd\ne\nf\n";
        let patch = "\
--- a/file.txt
+++ b/file.txt
@@ -1,3 +1,3 @@
 a
-b
+B
 c
@@ -4,3 +4,3 @@
 d
-e
+E
 f
";
        let files = parse_unified_diff(patch).unwrap();
        let result = apply_file_patch(original, &files[0].hunks).unwrap();
        assert_eq!(result, "a\nB\nc\nd\nE\nf\n");
    }

    #[test]
    fn creates_new_file() {
        let patch = "\
--- /dev/null
+++ b/new.txt
@@ -0,0 +1,3 @@
+line1
+line2
+line3
";
        let files = parse_unified_diff(patch).unwrap();
        assert!(files[0].is_new_file);
        assert_eq!(files[0].new_path, "new.txt");
        let result = apply_file_patch("", &files[0].hunks).unwrap();
        assert_eq!(result, "line1\nline2\nline3\n");
    }

    #[test]
    fn deletes_file() {
        let patch = "\
--- a/old.txt
+++ /dev/null
@@ -1,3 +0,0 @@
-line1
-line2
-line3
";
        let files = parse_unified_diff(patch).unwrap();
        assert!(files[0].is_deleted);
        assert_eq!(extract_target_path(&files[0]), "old.txt");
        let result = apply_file_patch("line1\nline2\nline3\n", &files[0].hunks).unwrap();
        assert_eq!(result, "");
    }

    #[test]
    fn adds_lines_at_end() {
        let original = "line1\nline2\n";
        let patch = "\
--- a/file.txt
+++ b/file.txt
@@ -2,0 +3,2 @@
 line2
+line3
+line4
";
        let files = parse_unified_diff(patch).unwrap();
        let result = apply_file_patch(original, &files[0].hunks).unwrap();
        assert_eq!(result, "line1\nline2\nline3\nline4\n");
    }

    #[test]
    fn strips_a_b_prefixes() {
        let patch = "\
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1 +1 @@
-old
+new
";
        let files = parse_unified_diff(patch).unwrap();
        assert_eq!(files[0].old_path, "src/lib.rs");
        assert_eq!(files[0].new_path, "src/lib.rs");
    }

    #[test]
    fn handles_no_prefix_paths() {
        let patch = "\
--- file.txt
+++ file.txt
@@ -1 +1 @@
-old
+new
";
        let files = parse_unified_diff(patch).unwrap();
        assert_eq!(files[0].old_path, "file.txt");
    }

    #[test]
    fn context_mismatch_is_error() {
        let original = "line1\nunexpected\nline3\n";
        let patch = "\
--- a/file.txt
+++ b/file.txt
@@ -1,3 +1,3 @@
 line1
-expected
+new
 line3
";
        let files = parse_unified_diff(patch).unwrap();
        let result = apply_file_patch(original, &files[0].hunks);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("mismatch"),
            "expected mismatch error, got: {msg}"
        );
    }

    #[test]
    fn mismatch_error_describes_trailing_whitespace() {
        let original = "line1  \t\n";
        let patch = "\
--- a/file.txt
+++ b/file.txt
@@ -1 +1 @@
-line1
+line2
";
        let files = parse_unified_diff(patch).unwrap();
        let err = apply_file_patch(original, &files[0].hunks).unwrap_err();
        let msg = err.to_string();

        assert!(msg.contains("trailing spaces: 2"), "{msg}");
        assert!(msg.contains("trailing tabs: 1"), "{msg}");
    }

    #[test]
    fn no_newline_at_eof_is_tolerated() {
        let original = "line1\nline2";
        let patch = "\
--- a/file.txt
+++ b/file.txt
@@ -1,2 +1,2 @@
 line1
-line2
+new2
\\ No newline at end of file
";
        let files = parse_unified_diff(patch).unwrap();
        let hunk = &files[0].hunks[0];
        assert!(
            hunk.lines
                .iter()
                .any(|l| matches!(l, HunkLine::NoNewlineAtEof)),
            "should parse no-newline marker"
        );
        let result = apply_file_patch(original, &files[0].hunks).unwrap();
        assert_eq!(result, "line1\nnew2");
    }

    #[test]
    fn no_newline_marker_after_removed_line_does_not_force_output_eof() {
        let original = "line1\nold";
        let patch = "\
--- a/file.txt
+++ b/file.txt
@@ -1,2 +1,2 @@
 line1
-old
\\ No newline at end of file
+new
";
        let files = parse_unified_diff(patch).unwrap();
        let result = apply_file_patch(original, &files[0].hunks).unwrap();
        assert_eq!(result, "line1\nnew\n");
    }

    #[test]
    fn preserves_unmodified_original_missing_trailing_newline() {
        let original = "line1\nold\nline3";
        let patch = "\
--- a/file.txt
+++ b/file.txt
@@ -1,3 +1,3 @@
 line1
-old
+new
 line3
\\ No newline at end of file
";
        let files = parse_unified_diff(patch).unwrap();
        let result = apply_file_patch(original, &files[0].hunks).unwrap();
        assert_eq!(result, "line1\nnew\nline3");
    }

    #[test]
    fn empty_patch_returns_error() {
        let result = parse_unified_diff("");
        assert!(result.is_err());
    }

    #[test]
    fn build_summary_reports_all_results() {
        let files = vec![FilePatch {
            old_path: "a/file.txt".into(),
            new_path: "file.txt".into(),
            is_new_file: false,
            is_deleted: false,
            hunks: vec![Hunk {
                old_start: 1,
                old_count: 1,
                new_start: 1,
                new_count: 1,
                lines: vec![
                    HunkLine::Removed("old".into()),
                    HunkLine::Added("new".into()),
                ],
            }],
        }];

        let mut results = HashMap::new();
        results.insert("file.txt".to_string(), Ok(b"new content".to_vec()));

        let summary = build_summary(&files, &results);
        assert!(summary.contains("patched: file.txt"));
        assert!(summary.contains("1 hunk"));
    }
}
