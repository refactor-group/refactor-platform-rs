//! Small text helpers shared by emails and transcript rendering.

/// `word` with its first letter in upper case.
pub(crate) fn capitalized(word: &str) -> String {
    let mut chars = word.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}
