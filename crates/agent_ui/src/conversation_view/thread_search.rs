use std::ops::Range;

use editor::Editor;
use gpui::{Entity, Subscription, Window};
use markdown::Markdown;

#[derive(Default, Clone, Copy)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}

pub struct ThreadSearch {
    pub query_editor: Entity<Editor>,
    pub dismissed: bool,
    pub include_tool_calls: bool,
    pub options: SearchOptions,
    pub matches: Vec<SearchMatch>,
    pub active_match_index: Option<usize>,
    pub _subscriptions: Vec<Subscription>,
}

impl ThreadSearch {
    pub fn new(window: &mut Window, cx: &mut gpui::App) -> Self {
        let query_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_placeholder_text("Search messages…", window, cx);
            editor
        });
        Self {
            query_editor,
            dismissed: true,
            include_tool_calls: false,
            options: SearchOptions::default(),
            matches: Vec::new(),
            active_match_index: None,
            _subscriptions: Vec::new(),
        }
    }

    pub fn query(&self, cx: &gpui::App) -> String {
        self.query_editor.read(cx).text(cx)
    }
}

#[derive(Clone)]
pub struct SearchMatch {
    pub entry_index: usize,
    pub markdown: Entity<Markdown>,
    pub range: Range<usize>,
}

/// Find every case-insensitive substring match of `query` in `text`.
///
/// Returns ranges over byte offsets into `text` (suitable for
/// [`markdown::Markdown::set_search_highlights`]). Empty queries return no
/// matches. Matches do not overlap; the search advances past each match.
pub fn find_matches_in_text(query: &str, text: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }

    let query_lower = query.to_lowercase();
    let mut matches = Vec::new();
    let mut start = 0;

    while start < text.len() {
        if let Some(match_end) = find_match_at_start(&query_lower, text, start) {
            matches.push(start..match_end);
            start = match_end;
        } else if let Some(char) = text[start..].chars().next() {
            start += char.len_utf8();
        } else {
            break;
        }
    }

    matches
}

fn find_match_at_start(query_lower: &str, text: &str, start: usize) -> Option<usize> {
    let mut folded_text = String::new();

    for (offset, char) in text[start..].char_indices() {
        folded_text.extend(char.to_lowercase());

        if folded_text.len() >= query_lower.len() {
            return (folded_text == query_lower).then_some(start + offset + char.len_utf8());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_returns_no_matches() {
        assert!(find_matches_in_text("", "hello world").is_empty());
    }

    #[test]
    fn no_matches_for_missing_substring() {
        assert!(find_matches_in_text("xyz", "hello world").is_empty());
    }

    #[test]
    fn finds_single_match() {
        assert_eq!(find_matches_in_text("world", "hello world"), vec![6..11]);
    }

    #[test]
    fn finds_multiple_non_overlapping_matches() {
        assert_eq!(
            find_matches_in_text("ab", "abcabcab"),
            vec![0..2, 3..5, 6..8],
        );
    }

    #[test]
    fn is_case_insensitive() {
        assert_eq!(
            find_matches_in_text("Hello", "hello HELLO HeLLo"),
            vec![0..5, 6..11, 12..17],
        );
    }

    #[test]
    fn advances_past_each_match_no_overlap() {
        assert_eq!(find_matches_in_text("aa", "aaa"), vec![0..2]);
    }

    #[test]
    fn returns_original_byte_ranges_when_lowercase_expands() {
        assert_eq!(find_matches_in_text("x", "İx"), vec![2..3]);
    }
}
