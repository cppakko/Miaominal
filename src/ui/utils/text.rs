pub(crate) fn truncate_with_ellipsis(value: &str, max_chars: usize) -> String {
    let chars: Vec<_> = value.chars().collect();
    if chars.len() <= max_chars {
        return value.to_string();
    }

    let visible: String = chars
        .into_iter()
        .take(max_chars.saturating_sub(3))
        .collect();
    format!("{visible}...")
}
