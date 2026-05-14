use std::{
    collections::{HashMap, HashSet},
    ops::Range,
};

use acp_thread::{AgentThreadEntry, AssistantMessageChunk, ContentBlock, ToolCallContent};
use editor::Editor;
use gpui::{
    Action, AnyElement, AppContext, Entity, FocusHandle, Focusable, IntoElement, ParentElement,
    Styled, Subscription, Window, div,
};
use markdown::Markdown;
use ui::{
    ActiveTheme, ButtonCommon, Clickable, Color, IconButton, IconButtonShape, IconName, IconSize,
    Label, LabelCommon, LabelSize, Toggleable, Tooltip, h_flex,
};

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

    pub fn update_matches(&mut self, sources: &[(usize, Entity<Markdown>)], cx: &gpui::App) {
        let query = self.query(cx);
        self.matches.clear();

        if query.is_empty() {
            self.active_match_index = None;
            return;
        }

        for (entry_index, markdown) in sources {
            let ranges = {
                let markdown = markdown.read(cx);
                find_matches_in_text(&query, markdown.source())
            };

            for range in ranges {
                self.matches.push(SearchMatch {
                    entry_index: *entry_index,
                    markdown: markdown.clone(),
                    range,
                });
            }
        }

        self.active_match_index = (!self.matches.is_empty()).then_some(0);
    }

    pub fn apply_highlights(&self, cx: &mut gpui::App) {
        let mut highlights_by_markdown: HashMap<
            Entity<Markdown>,
            (Vec<Range<usize>>, Option<usize>),
        > = HashMap::default();

        for (match_index, search_match) in self.matches.iter().enumerate() {
            let (ranges, active) = highlights_by_markdown
                .entry(search_match.markdown.clone())
                .or_default();
            let local_match_index = ranges.len();
            ranges.push(search_match.range.clone());

            if self.active_match_index == Some(match_index) {
                *active = Some(local_match_index);
            }
        }

        for (markdown, (ranges, active)) in highlights_by_markdown {
            markdown.update(cx, |markdown, cx| {
                markdown.set_search_highlights(ranges, active, cx);
            });
        }
    }

    pub fn step_active(&mut self, delta: isize) -> Option<usize> {
        if self.matches.is_empty() {
            self.active_match_index = None;
            return None;
        }

        let match_count = self.matches.len();
        let current_match_index = self
            .active_match_index
            .filter(|index| *index < match_count)
            .unwrap_or(0);
        let next_match_index =
            (current_match_index as isize + delta).rem_euclid(match_count as isize) as usize;

        self.active_match_index = Some(next_match_index);
        self.matches
            .get(next_match_index)
            .map(|search_match| search_match.entry_index)
    }

    pub fn clear_highlights(&self, cx: &mut gpui::App) {
        let mut cleared_markdowns = HashSet::<Entity<Markdown>>::new();

        for search_match in &self.matches {
            if cleared_markdowns.insert(search_match.markdown.clone()) {
                search_match.markdown.update(cx, |markdown, cx| {
                    markdown.clear_search_highlights(cx);
                });
            }
        }
    }

    pub fn deploy(&mut self, window: &mut Window, cx: &mut gpui::App) {
        self.dismissed = false;
        self.query_editor.update(cx, |editor, cx| {
            editor.focus_handle(cx).focus(window, cx);
            editor.select_all(&editor::actions::SelectAll, window, cx);
        });
    }

    pub fn dismiss(&mut self, _window: &mut Window, _cx: &mut gpui::App) {
        self.dismissed = true;
        self.matches.clear();
        self.active_match_index = None;
    }

    pub fn query_editor_focus_handle(&self, cx: &gpui::App) -> FocusHandle {
        self.query_editor.read(cx).focus_handle(cx)
    }

    pub fn render(&self, cx: &mut gpui::App) -> Option<AnyElement> {
        if self.dismissed {
            return None;
        }

        Some(
            h_flex()
                .px_2()
                .py_1()
                .gap_2()
                .border_b_1()
                .border_color(cx.theme().colors().border_variant)
                .bg(cx.theme().colors().editor_background)
                .child(div().flex_1().min_w_0().child(self.query_editor.clone()))
                .child(
                    Label::new(format_match_counter(
                        self.active_match_index,
                        self.matches.len(),
                    ))
                    .size(LabelSize::Small)
                    .color(Color::Muted),
                )
                .child(
                    IconButton::new("thread-search-prev", IconName::ChevronUp)
                        .shape(IconButtonShape::Square)
                        .icon_size(IconSize::Small)
                        .on_click(|_, window, cx| {
                            window.dispatch_action(menu::SelectPrevious.boxed_clone(), cx);
                        })
                        .tooltip(Tooltip::text("Previous Match")),
                )
                .child(
                    IconButton::new("thread-search-next", IconName::ChevronDown)
                        .shape(IconButtonShape::Square)
                        .icon_size(IconSize::Small)
                        .on_click(|_, window, cx| {
                            window.dispatch_action(menu::SelectNext.boxed_clone(), cx);
                        })
                        .tooltip(Tooltip::text("Next Match")),
                )
                .child(
                    IconButton::new("thread-search-toggle-tools", IconName::ToolHammer)
                        .shape(IconButtonShape::Square)
                        .icon_size(IconSize::Small)
                        .toggle_state(self.include_tool_calls)
                        .tooltip(Tooltip::text("Include Tool Calls")),
                )
                .child(
                    IconButton::new("thread-search-close", IconName::Close)
                        .shape(IconButtonShape::Square)
                        .icon_size(IconSize::Small)
                        .on_click(|_, window, cx| {
                            window.dispatch_action(crate::ToggleSearch.boxed_clone(), cx);
                        })
                        .tooltip(Tooltip::text("Close Search")),
                )
                .into_any_element(),
        )
    }
}

pub fn collect_searchable_markdowns(
    entries: &[AgentThreadEntry],
    include_tool_calls: bool,
) -> Vec<(usize, Entity<Markdown>)> {
    entries
        .iter()
        .enumerate()
        .flat_map(|(entry_index, entry)| {
            let mut markdowns = Vec::new();

            match entry {
                AgentThreadEntry::UserMessage(message) => {
                    if let ContentBlock::Markdown { markdown } = &message.content {
                        markdowns.push((entry_index, markdown.clone()));
                    }
                }
                AgentThreadEntry::AssistantMessage(message) => {
                    for chunk in &message.chunks {
                        let block = match chunk {
                            AssistantMessageChunk::Message { block }
                            | AssistantMessageChunk::Thought { block } => block,
                        };

                        if let ContentBlock::Markdown { markdown } = block {
                            markdowns.push((entry_index, markdown.clone()));
                        }
                    }
                }
                AgentThreadEntry::ToolCall(tool_call) => {
                    if include_tool_calls {
                        markdowns.push((entry_index, tool_call.label.clone()));

                        for content in &tool_call.content {
                            if let ToolCallContent::ContentBlock(ContentBlock::Markdown {
                                markdown,
                            }) = content
                            {
                                markdowns.push((entry_index, markdown.clone()));
                            }
                        }
                    }
                }
                AgentThreadEntry::CompletedPlan(_) => {}
            }

            markdowns
        })
        .collect()
}

fn format_match_counter(active_match_index: Option<usize>, match_count: usize) -> String {
    let active_match_number = active_match_index.map_or(0, |index| index + 1);
    format!("{active_match_number}/{match_count}")
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
    use gpui::{Render, TestAppContext};
    use std::{cell::RefCell, rc::Rc};

    struct TestWindow;

    impl Render for TestWindow {
        fn render(
            &mut self,
            _window: &mut Window,
            _cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            div()
        }
    }

    #[gpui::test]
    fn apply_highlights_groups_by_markdown_and_sets_local_active(cx: &mut TestAppContext) {
        crate::test_support::init_test(cx);

        let markdown_a = cx.new(|cx| Markdown::new("one two one".into(), None, None, cx));
        let markdown_b = cx.new(|cx| Markdown::new("two one".into(), None, None, cx));
        let query_editor = Rc::new(RefCell::new(None));
        cx.add_window({
            let query_editor = query_editor.clone();
            move |window, cx| {
                let editor = cx.new(|cx| Editor::single_line(window, cx));
                *query_editor.borrow_mut() = Some(editor);
                TestWindow
            }
        });
        let query_editor = match query_editor.borrow().clone() {
            Some(query_editor) => query_editor,
            None => panic!("test window should create query editor"),
        };
        let thread_search = ThreadSearch {
            query_editor,
            dismissed: false,
            include_tool_calls: false,
            options: SearchOptions::default(),
            matches: vec![
                SearchMatch {
                    entry_index: 0,
                    markdown: markdown_a.clone(),
                    range: 0..3,
                },
                SearchMatch {
                    entry_index: 1,
                    markdown: markdown_b.clone(),
                    range: 4..7,
                },
                SearchMatch {
                    entry_index: 0,
                    markdown: markdown_a.clone(),
                    range: 8..11,
                },
            ],
            active_match_index: Some(2),
            _subscriptions: Vec::new(),
        };

        cx.update(|cx| thread_search.apply_highlights(cx));

        cx.update(|cx| {
            assert_eq!(markdown_a.read(cx).search_highlights(), &[0..3, 8..11]);
            assert_eq!(markdown_a.read(cx).active_search_highlight(), Some(1));
            assert_eq!(markdown_b.read(cx).search_highlights(), &[4..7]);
            assert_eq!(markdown_b.read(cx).active_search_highlight(), None);
        });

        cx.update(|cx| thread_search.clear_highlights(cx));

        cx.update(|cx| {
            assert!(markdown_a.read(cx).search_highlights().is_empty());
            assert_eq!(markdown_a.read(cx).active_search_highlight(), None);
            assert!(markdown_b.read(cx).search_highlights().is_empty());
            assert_eq!(markdown_b.read(cx).active_search_highlight(), None);
        });
    }

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

    #[test]
    fn formats_match_counter() {
        assert_eq!(format_match_counter(None, 0), "0/0");
        assert_eq!(format_match_counter(Some(0), 3), "1/3");
        assert_eq!(format_match_counter(Some(2), 3), "3/3");
    }

    #[gpui::test]
    fn step_active_wraps_through_matches(cx: &mut TestAppContext) {
        crate::test_support::init_test(cx);

        let markdown = cx.new(|cx| Markdown::new("one two three".into(), None, None, cx));
        let query_editor = Rc::new(RefCell::new(None));
        cx.add_window({
            let query_editor = query_editor.clone();
            move |window, cx| {
                let editor = cx.new(|cx| Editor::single_line(window, cx));
                *query_editor.borrow_mut() = Some(editor);
                TestWindow
            }
        });
        let query_editor = match query_editor.borrow().clone() {
            Some(query_editor) => query_editor,
            None => panic!("test window should create query editor"),
        };
        let mut thread_search = ThreadSearch {
            query_editor,
            dismissed: false,
            include_tool_calls: false,
            options: SearchOptions::default(),
            matches: vec![
                SearchMatch {
                    entry_index: 1,
                    markdown: markdown.clone(),
                    range: 0..3,
                },
                SearchMatch {
                    entry_index: 4,
                    markdown: markdown.clone(),
                    range: 4..7,
                },
                SearchMatch {
                    entry_index: 9,
                    markdown,
                    range: 8..13,
                },
            ],
            active_match_index: Some(0),
            _subscriptions: Vec::new(),
        };

        assert_eq!(thread_search.step_active(1), Some(4));
        assert_eq!(thread_search.active_match_index, Some(1));
        assert_eq!(thread_search.step_active(1), Some(9));
        assert_eq!(thread_search.active_match_index, Some(2));
        assert_eq!(thread_search.step_active(1), Some(1));
        assert_eq!(thread_search.active_match_index, Some(0));
        assert_eq!(thread_search.step_active(-1), Some(9));
        assert_eq!(thread_search.active_match_index, Some(2));
    }

    #[gpui::test]
    fn step_active_clears_active_when_empty(cx: &mut TestAppContext) {
        crate::test_support::init_test(cx);

        let query_editor = Rc::new(RefCell::new(None));
        cx.add_window({
            let query_editor = query_editor.clone();
            move |window, cx| {
                let editor = cx.new(|cx| Editor::single_line(window, cx));
                *query_editor.borrow_mut() = Some(editor);
                TestWindow
            }
        });
        let query_editor = match query_editor.borrow().clone() {
            Some(query_editor) => query_editor,
            None => panic!("test window should create query editor"),
        };
        let mut thread_search = ThreadSearch {
            query_editor,
            dismissed: false,
            include_tool_calls: false,
            options: SearchOptions::default(),
            matches: Vec::new(),
            active_match_index: Some(2),
            _subscriptions: Vec::new(),
        };

        assert_eq!(thread_search.step_active(1), None);
        assert_eq!(thread_search.active_match_index, None);
    }
}
