//! Pure helpers for `edit_file` / `write_file`: exact replacement and
//! minimal unified diffs (applied through the existing apply_patch path).

/// Replace `old` with `new` in `content`. `old` must occur exactly once
/// unless `replace_all` is set.
pub fn replace_exact(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<(String, usize), String> {
    if old.is_empty() {
        return Err("old_string is empty; use write_file to create or replace a whole file".into());
    }
    if old == new {
        return Err("old_string and new_string are identical; nothing to change".into());
    }
    let count = content.matches(old).count();
    if count == 0 {
        // Common model mistake: CRLF files, or line-number prefixes copied
        // from read_file output.
        let hint = if content.contains("\r\n") && !old.contains('\r') {
            let crlf_old = old.replace('\n', "\r\n");
            if content.contains(&crlf_old) {
                let crlf_new = new.replace('\n', "\r\n");
                return replace_exact(content, &crlf_old, &crlf_new, replace_all);
            }
            ""
        } else if old.contains('│') {
            " Do not include the line-number prefixes from read_file."
        } else {
            " Read the file again and copy the text exactly, including indentation."
        };
        return Err(format!("old_string was not found in the file.{hint}"));
    }
    if count > 1 && !replace_all {
        return Err(format!(
            "old_string occurs {count} times; add surrounding lines to make it unique, or set replace_all to true"
        ));
    }
    let replaced = if replace_all {
        content.replace(old, new)
    } else {
        content.replacen(old, new, 1)
    };
    Ok((replaced, if replace_all { count } else { 1 }))
}

/// Split into lines keeping their terminators.
fn lines_with_endings(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

fn push_line(out: &mut String, prefix: char, line: &str) {
    out.push(prefix);
    if let Some(stripped) = line.strip_suffix('\n') {
        out.push_str(stripped);
        out.push('\n');
    } else {
        out.push_str(line);
        out.push_str("\n\\ No newline at end of file\n");
    }
}

/// Minimal single-hunk unified diff between `old` and `new` for `path`
/// (`patch -p0` style, path relative to the patch base directory).
pub fn unified_diff(path: &str, old: &str, new: &str, context: usize) -> String {
    let old_lines = lines_with_endings(old);
    let new_lines = lines_with_endings(new);
    let mut prefix = 0;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && old_lines[prefix] == new_lines[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < old_lines.len() - prefix
        && suffix < new_lines.len() - prefix
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let start = prefix.saturating_sub(context);
    let old_end = (old_lines.len() - suffix + context).min(old_lines.len());
    let new_end = (new_lines.len() - suffix + context).min(new_lines.len());

    let old_count = old_end - start;
    let new_count = new_end - start;
    let range = |start: usize, count: usize| {
        if count == 0 {
            format!("{start},0")
        } else {
            format!("{},{count}", start + 1)
        }
    };
    let mut out = format!(
        "--- {path}\n+++ {path}\n@@ -{} +{} @@\n",
        range(start, old_count),
        range(start, new_count)
    );
    for line in &old_lines[start..prefix] {
        push_line(&mut out, ' ', line);
    }
    for line in &old_lines[prefix..old_lines.len() - suffix] {
        push_line(&mut out, '-', line);
    }
    for line in &new_lines[prefix..new_lines.len() - suffix] {
        push_line(&mut out, '+', line);
    }
    for line in &old_lines[old_lines.len() - suffix..old_end] {
        push_line(&mut out, ' ', line);
    }
    out
}

/// Diff that creates `path` with `content`.
pub fn new_file_diff(path: &str, content: &str) -> String {
    let lines = lines_with_endings(content);
    let mut out = format!(
        "--- /dev/null\n+++ {path}\n@@ -0,0 +{} @@\n",
        if lines.is_empty() {
            "0,0".to_string()
        } else {
            format!("1,{}", lines.len())
        }
    );
    for line in lines {
        push_line(&mut out, '+', line);
    }
    out
}

/// Split a target path into (parent directory, file name).
pub fn split_parent(path: &str) -> (String, String) {
    let trimmed = path.trim_end_matches(['/', '\\']);
    match trimmed.rfind(['/', '\\']) {
        Some(0) => ("/".into(), trimmed[1..].into()),
        Some(index) => (trimmed[..index].into(), trimmed[index + 1..].into()),
        None => (".".into(), trimmed.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_requires_unique_match() {
        assert_eq!(
            replace_exact("a b a", "b", "c", false).unwrap(),
            ("a c a".into(), 1)
        );
        assert!(
            replace_exact("a b a", "a", "c", false)
                .unwrap_err()
                .contains("2 times")
        );
        assert_eq!(
            replace_exact("a b a", "a", "c", true).unwrap(),
            ("c b c".into(), 2)
        );
        assert!(
            replace_exact("abc", "x", "y", false)
                .unwrap_err()
                .contains("not found")
        );
        assert!(replace_exact("abc", "", "y", false).is_err());
    }

    #[test]
    fn replace_adapts_to_crlf_files() {
        assert_eq!(
            replace_exact("a\r\nb\r\n", "a\nb", "x\ny", false)
                .unwrap()
                .0,
            "x\r\ny\r\n"
        );
    }

    #[test]
    fn diff_has_minimal_hunk_with_context() {
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n";
        let new = "1\n2\n3\n4\nFIVE\n6\n7\n8\n";
        assert_eq!(
            unified_diff("f.conf", old, new, 3),
            "--- f.conf\n+++ f.conf\n@@ -2,7 +2,7 @@\n 2\n 3\n 4\n-5\n+FIVE\n 6\n 7\n 8\n"
        );
    }

    #[test]
    fn diff_marks_missing_trailing_newline() {
        let diff = unified_diff("f", "a\nb", "a\nc", 1);
        assert_eq!(
            diff,
            "--- f\n+++ f\n@@ -1,2 +1,2 @@\n a\n-b\n\\ No newline at end of file\n+c\n\\ No newline at end of file\n"
        );
        assert_eq!(
            new_file_diff("n", "x\ny\n"),
            "--- /dev/null\n+++ n\n@@ -0,0 +1,2 @@\n+x\n+y\n"
        );
    }

    #[test]
    fn parent_split() {
        assert_eq!(
            split_parent("/etc/nginx/nginx.conf"),
            ("/etc/nginx".into(), "nginx.conf".into())
        );
        assert_eq!(split_parent("/x"), ("/".into(), "x".into()));
        assert_eq!(split_parent("app.conf"), (".".into(), "app.conf".into()));
        assert_eq!(
            split_parent("C:\\a\\b.txt"),
            ("C:\\a".into(), "b.txt".into())
        );
    }
}
