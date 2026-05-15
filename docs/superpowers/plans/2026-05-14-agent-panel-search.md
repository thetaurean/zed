# Agent Panel Thread Search Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a full search feature to the Zed agent panel's thread view — a search button in the panel toolbar (and `cmd-f` / `ctrl-f` shortcut) that opens an in-panel search bar for finding text across messages, with next/prev navigation, match counter, and a toggle to include tool calls.

**Architecture:** A new `ThreadSearch` struct lives as a field on `ThreadView`. It owns its own single-line query `Editor`, tracks matches as a flat `Vec<SearchMatch>` (each pointing at a specific `Entity<Markdown>` and a character range), and uses the existing `Markdown::set_search_highlights` API to render highlights. Match navigation updates the active match and scrolls the parent `ListState` to the entry containing it. A new `agent::ToggleSearch` action wires the toolbar button and the keymap into `ThreadView`'s deploy/dismiss handlers.

**Tech Stack:** Rust, GPUI (Zed's UI framework), `Editor` (single-line), `Markdown` entity (with `set_search_highlights`), `ListState` (for scroll), `IconButton` UI components.

---

## File Structure

**New files:**
- `crates/agent_ui/src/conversation_view/thread_search.rs` — the `ThreadSearch` struct, `SearchMatch`, the pure `find_matches_in_text` function, and the `render_search_bar` UI.

**Modified files:**
- `crates/agent_ui/src/agent_ui.rs` — add `ToggleSearch` action to the `actions!(agent, [...])` macro at line 116.
- `crates/agent_ui/src/conversation_view.rs` — add `mod thread_search;` next to the existing `mod thread_view;` at line 106.
- `crates/agent_ui/src/conversation_view/thread_view.rs` — add `thread_search: ThreadSearch` field, initialize it in `ThreadView::new`, render it above the list, register `on_action` handlers for `ToggleSearch` and `menu::Cancel` (when search has focus).
- `crates/agent_ui/src/agent_panel.rs` — add a search `IconButton` to `render_toolbar` (line 4235).
- `assets/keymaps/default-macos.json` — add `"cmd-f": "agent::ToggleSearch"` under the `"AgentPanel"` context, plus `escape` / `enter` / `shift-enter` bindings under a sub-context.
- `assets/keymaps/default-linux.json` — same with `"ctrl-f"`.
- `assets/keymaps/default-windows.json` — same with `"ctrl-f"`.

---

## Pre-flight Notes for the Implementer

These are codebase facts that will save you time:

1. **Build/lint command:** Use `./script/clippy` instead of `cargo clippy` (per repo CLAUDE.md). For tests use `cargo test -p agent_ui` for the agent panel package.
2. **No `mod.rs`:** Per repo conventions, `conversation_view.rs` (the file) declares submodules. Don't create `conversation_view/mod.rs`.
3. **Markdown highlighting is built-in:** `crates/markdown/src/markdown.rs:696` defines `Markdown::set_search_highlights(highlights: Vec<Range<usize>>, active: Option<usize>, cx: &mut Context<Self>)`. It calls `cx.notify()` for you. There is also `clear_search_highlights(cx)` and `set_active_search_highlight(active, cx)`. Highlights are over the *raw markdown source* (`source: &str`).
4. **`AgentThreadEntry` variants** (`crates/acp_thread/src/acp_thread.rs:207`):
   ```rust
   pub enum AgentThreadEntry {
       UserMessage(UserMessage),       // .content: ContentBlock
       AssistantMessage(AssistantMessage), // .chunks: Vec<AssistantMessageChunk>
       ToolCall(ToolCall),             // .label: Entity<Markdown>, .content: Vec<ToolCallContent>
       CompletedPlan(Vec<PlanEntry>),
   }
   ```
   `AssistantMessageChunk` (line 179) is `Message { block: ContentBlock }` or `Thought { block: ContentBlock }`.
   `ContentBlock` (line 678) variants include `Markdown { markdown: Entity<Markdown> }`. Other variants (`Empty`, `ResourceLink`, `Image`) have no markdown to highlight; skip them.
5. **`ListState::scroll_to(ListOffset { item_ix, offset_in_item })`** is the API (used at `thread_view.rs:5203` already).
6. **Editor for single-line input:** `Editor::single_line(window, cx)` then `editor.set_placeholder_text("Search…", window, cx)`. Subscribe with `cx.subscribe_in(&editor, window, |this, _, event, window, cx| { if let editor::EditorEvent::BufferEdited = event { ... } })`.
7. **Existing keymap context:** `"AgentPanel"` (set by `AgentPanel::key_context` at `agent_panel.rs:4903`).
8. **Action style:** Add a doc comment immediately above each new action; the `actions!` macro picks it up.
9. **Pre-commit hook:** This repo has a pre-commit hook. Don't use `--no-verify`. If it fails, fix the underlying issue and re-stage.
10. **Tests run with GPUI's runtime.** Pure logic tests use plain `#[test]`; GPUI tests use `#[gpui::test] async fn name(cx: &mut TestAppContext)` (see `agent_panel.rs:7845` for an example).

---

## Task 1: Add the `ToggleSearch` action

**Files:**
- Modify: `crates/agent_ui/src/agent_ui.rs:116-222` (the `actions!(agent, [...])` macro)

- [ ] **Step 1: Open `crates/agent_ui/src/agent_ui.rs` and find the `actions!(agent, [...])` macro starting at line 116.**

The last action in the list before the closing `]` is currently:
```rust
        /// Import agent threads from other Zed release channels (e.g. Preview, Nightly).
        ImportThreadsFromOtherChannels,
    ]
);
```

- [ ] **Step 2: Add the new action right before the closing `]`.**

Edit: insert these two lines immediately before `    ]\n);`:
```rust
        /// Toggles the in-panel search bar for the active thread.
        ToggleSearch,
```

After the edit, the tail of the macro should read:
```rust
        /// Import agent threads from other Zed release channels (e.g. Preview, Nightly).
        ImportThreadsFromOtherChannels,
        /// Toggles the in-panel search bar for the active thread.
        ToggleSearch,
    ]
);
```

- [ ] **Step 3: Verify the action compiles.**

Run: `cargo check -p agent_ui`
Expected: Builds without errors. The action `agent::ToggleSearch` is now usable.

- [ ] **Step 4: Commit.**

```bash
git add crates/agent_ui/src/agent_ui.rs
git commit -m "agent_ui: Declare ToggleSearch action"
```

---

## Task 2: Add keymap bindings

**Files:**
- Modify: `assets/keymaps/default-macos.json` (find the `"context": "AgentPanel"` block)
- Modify: `assets/keymaps/default-linux.json` (find the `"context": "AgentPanel"` block)
- Modify: `assets/keymaps/default-windows.json` (find the `"context": "AgentPanel"` block)

- [ ] **Step 1: Edit `assets/keymaps/default-macos.json`.**

Find the existing block:
```json
  {
    "context": "AgentPanel",
    "use_key_equivalents": true,
    "bindings": {
      "cmd-n": "agent::NewThread",
      "cmd-alt-c": "agent::OpenSettings",
      "cmd-alt-m": "agent::ToggleOptionsMenu",
      "cmd-alt-shift-n": "agent::ToggleNewThreadMenu",
```

Add the new binding inside the `bindings` object (after the existing `cmd-alt-shift-n` line — JSON allows trailing items, just remember to keep the comma syntax consistent with the file):

```json
      "cmd-f": "agent::ToggleSearch",
```

- [ ] **Step 2: Add a sub-context for in-search keys (still in `default-macos.json`).**

Add a new top-level block immediately after the existing `"AgentPanel"` block (before the next `{` block in the array). The sub-context `"AgentPanel > ThreadSearch"` will be activated by `ThreadView`'s key context (we'll wire that in Task 4).

```json
  {
    "context": "AgentPanel > ThreadSearch",
    "use_key_equivalents": true,
    "bindings": {
      "escape": "agent::ToggleSearch",
      "enter": "menu::SelectNext",
      "shift-enter": "menu::SelectPrevious"
    }
  },
```

We deliberately reuse `menu::SelectNext` / `menu::SelectPrevious` rather than inventing new actions — `ThreadView` will register handlers for these when the search has focus (Task 7).

- [ ] **Step 3: Edit `assets/keymaps/default-linux.json` analogously.**

Find the `"AgentPanel"` block and add `"ctrl-f": "agent::ToggleSearch"` to its bindings. Then add the sub-context block:
```json
  {
    "context": "AgentPanel > ThreadSearch",
    "bindings": {
      "escape": "agent::ToggleSearch",
      "enter": "menu::SelectNext",
      "shift-enter": "menu::SelectPrevious"
    }
  },
```

(Linux file does not use `use_key_equivalents`.)

- [ ] **Step 4: Edit `assets/keymaps/default-windows.json` analogously.**

Find the `"AgentPanel"` block and add `"ctrl-f": "agent::ToggleSearch"` to its bindings. Add the same sub-context block as Linux.

- [ ] **Step 5: Verify JSON validity.**

Run: `python3 -c "import json; [json.load(open(f'assets/keymaps/default-{p}.json')) for p in ['macos','linux','windows']]; print('OK')"`
Expected: prints `OK`. (Zed's keymap files use JSONC, which Python's strict parser may reject if there are comments — if so, just spot-check with your editor.)

- [ ] **Step 6: Commit.**

```bash
git add assets/keymaps/default-macos.json assets/keymaps/default-linux.json assets/keymaps/default-windows.json
git commit -m "agent_ui: Bind cmd-f / ctrl-f to ToggleSearch in agent panel"
```

---

## Task 3: Create `ThreadSearch` skeleton + pure search function (TDD)

This task creates the new `thread_search.rs` file with the data model, a unit-tested pure search function, and the struct skeleton. No UI yet.

**Files:**
- Create: `crates/agent_ui/src/conversation_view/thread_search.rs`
- Modify: `crates/agent_ui/src/conversation_view.rs:106` (add `mod thread_search;`)

- [ ] **Step 1: Create the new file with the test-first scaffolding.**

Create `crates/agent_ui/src/conversation_view/thread_search.rs` with this exact content:

```rust
use std::ops::Range;

use editor::Editor;
use gpui::{Entity, Subscription};
use markdown::Markdown;

pub struct ThreadSearch {
    pub query_editor: Entity<Editor>,
    pub dismissed: bool,
    pub include_tool_calls: bool,
    pub matches: Vec<SearchMatch>,
    pub active_match_index: Option<usize>,
    pub _subscriptions: Vec<Subscription>,
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
    let text_lower = text.to_lowercase();
    let mut matches = Vec::new();
    let mut start = 0;

    while start < text_lower.len() {
        match text_lower[start..].find(&query_lower) {
            Some(rel) => {
                let match_start = start + rel;
                let match_end = match_start + query_lower.len();
                matches.push(match_start..match_end);
                start = match_end;
            }
            None => break,
        }
    }

    matches
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
        // For "aaa", searching "aa" should match at 0..2 only (not 1..3).
        assert_eq!(find_matches_in_text("aa", "aaa"), vec![0..2]);
    }
}
```

The lowercase-byte-range trick works because ASCII case folding is byte-length preserving, and for non-ASCII text Rust's `to_lowercase` may change byte length — that's a known limitation we're accepting (full Unicode-correct match offsets would require a more complex algorithm; this matches what most editor "find" features do for the common case).

- [ ] **Step 2: Register the new module.**

Edit `crates/agent_ui/src/conversation_view.rs`. Find line 106 which currently reads `mod thread_view;`. Insert the new module declaration right after it:

```rust
mod thread_view;
mod thread_search;
```

- [ ] **Step 3: Run the unit tests to verify they pass.**

Run: `cargo test -p agent_ui --lib conversation_view::thread_search::tests`
Expected: 6 passed. Output should contain:
```
test conversation_view::thread_search::tests::empty_query_returns_no_matches ... ok
test conversation_view::thread_search::tests::no_matches_for_missing_substring ... ok
test conversation_view::thread_search::tests::finds_single_match ... ok
test conversation_view::thread_search::tests::finds_multiple_non_overlapping_matches ... ok
test conversation_view::thread_search::tests::is_case_insensitive ... ok
test conversation_view::thread_search::tests::advances_past_each_match_no_overlap ... ok
```

If `Editor`, `Markdown`, etc. fail to import, double-check `Cargo.toml` for `agent_ui` already depends on `editor`, `markdown`, and `gpui` (it does — they're used heavily by `thread_view.rs`).

- [ ] **Step 4: Commit.**

```bash
git add crates/agent_ui/src/conversation_view/thread_search.rs crates/agent_ui/src/conversation_view.rs
git commit -m "agent_ui: Introduce ThreadSearch with unit-tested matcher"
```

---

## Task 4: Add `ThreadSearch::new` and integrate as a `ThreadView` field

This task makes `ThreadSearch` constructable, adds it as a field on `ThreadView`, and initializes it in `ThreadView::new`. No rendering or behavior yet — just plumbing so subsequent tasks have a place to hang state.

**Files:**
- Modify: `crates/agent_ui/src/conversation_view/thread_search.rs` (add `new` constructor)
- Modify: `crates/agent_ui/src/conversation_view/thread_view.rs` (add field, initialize, import)

- [ ] **Step 1: Extend `thread_search.rs` with imports and `new`.**

Replace the existing `use` block at the top of `crates/agent_ui/src/conversation_view/thread_search.rs` with:

```rust
use std::ops::Range;

use editor::Editor;
use gpui::{Entity, Subscription, Window};
use markdown::Markdown;
```

Then also add the `SearchOptions` field type alongside the existing struct (we'll fill in toggle state in Task 11; declaring the bag of options now keeps the struct stable):

```rust
#[derive(Default, Clone, Copy)]
pub struct SearchOptions {
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
}
```

Insert it immediately above `pub struct ThreadSearch`. Then update `pub struct ThreadSearch` to include the new field:

```rust
pub struct ThreadSearch {
    pub query_editor: Entity<Editor>,
    pub dismissed: bool,
    pub include_tool_calls: bool,
    pub options: SearchOptions,
    pub matches: Vec<SearchMatch>,
    pub active_match_index: Option<usize>,
    pub _subscriptions: Vec<Subscription>,
}
```

Add an `impl ThreadSearch` block immediately after the struct (before `#[derive(Clone)] pub struct SearchMatch`):

```rust
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
```

Note: `ThreadSearch` is a plain struct, not an `Entity`. Its `new` takes `&mut gpui::App` rather than `&mut Context<Self>` — `App::new` (via `cx.new(...)`) creates the inner `query_editor` entity. This lets `ThreadView::new` build a `ThreadSearch` directly with its own `&mut Context<ThreadView>` (which derefs to `&mut App`).

- [ ] **Step 2: Add the `thread_search` field to `ThreadView`.**

In `crates/agent_ui/src/conversation_view/thread_view.rs`, find the `ThreadView` struct definition. The struct begins around line 270 and the field list runs through ~line 340. Find the line:
```rust
    pub message_editor: Entity<MessageEditor>,
```

(This is around line 328.) Add a new field immediately after it:
```rust
    pub thread_search: ThreadSearch,
```

- [ ] **Step 3: Add the import.**

Near the top of `thread_view.rs`, find the existing imports. Look for the line that currently imports from `super::` (if any), or use the section that imports from `crate::`. Add:
```rust
use super::thread_search::ThreadSearch;
```

If you see a block of `use super::*;` or `use crate::conversation_view::*;`, you can add the import grouped consistently.

- [ ] **Step 4: Initialize the field in `ThreadView::new`.**

Find `impl ThreadView { pub fn new(...) -> Self { ... } }` (or whichever constructor builds the struct literal). Locate where existing fields are populated (e.g., where `message_editor: ...` is assigned). Add a line:
```rust
            thread_search: ThreadSearch::new(window, cx),
```

If `ThreadView::new` doesn't have `window: &mut Window` and `cx: &mut App`-compatible parameters in scope at that point, find the analogous parameters (it almost certainly will — every other entity construction in this constructor needs them). If the constructor builds `ThreadSearch` outside the struct literal, do that, e.g. at the top of `new`:
```rust
        let thread_search = ThreadSearch::new(window, cx);
```
and reference `thread_search,` in the literal.

- [ ] **Step 5: Verify it builds.**

Run: `cargo check -p agent_ui`
Expected: Builds without errors. Field is unused for now — that's fine because of `pub`.

- [ ] **Step 6: Commit.**

```bash
git add crates/agent_ui/src/conversation_view/thread_search.rs crates/agent_ui/src/conversation_view/thread_view.rs
git commit -m "agent_ui: Wire ThreadSearch into ThreadView"
```

---

## Task 5: Render the search bar (UI only, no behavior)

This task makes the search bar visible when `dismissed == false`. We'll fake-flip `dismissed` to verify rendering, then revert.

**Files:**
- Modify: `crates/agent_ui/src/conversation_view/thread_search.rs` (add `render` method)
- Modify: `crates/agent_ui/src/conversation_view/thread_view.rs` (call render in `Render` impl)

- [ ] **Step 1: Add the render method to `ThreadSearch`.**

Append this `impl` block to `thread_search.rs`:

```rust
use gpui::{IntoElement, ParentElement, Styled, div};
use ui::{
    ActiveTheme, Color, IconButton, IconButtonShape, IconName, IconSize, Label, LabelCommon,
    LabelSize, Tooltip, h_flex,
};

impl ThreadSearch {
    pub fn render(&self, cx: &mut gpui::App) -> Option<gpui::AnyElement> {
        if self.dismissed {
            return None;
        }
        let total = self.matches.len();
        let active_display = self
            .active_match_index
            .map(|i| (i + 1).to_string())
            .unwrap_or_else(|| "0".to_string());
        let counter_text = format!("{}/{}", active_display, total);

        let bar = h_flex()
            .px_2()
            .py_1()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().colors().border_variant)
            .bg(cx.theme().colors().editor_background)
            .child(
                div()
                    .flex_1()
                    .child(self.query_editor.clone()),
            )
            .child(
                Label::new(counter_text)
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .child(
                IconButton::new("thread-search-prev", IconName::ChevronUp)
                    .shape(IconButtonShape::Square)
                    .icon_size(IconSize::Small)
                    .tooltip(Tooltip::text("Previous Match")),
            )
            .child(
                IconButton::new("thread-search-next", IconName::ChevronDown)
                    .shape(IconButtonShape::Square)
                    .icon_size(IconSize::Small)
                    .tooltip(Tooltip::text("Next Match")),
            )
            .child(
                IconButton::new("thread-search-toggle-tools", IconName::ToolHammer)
                    .shape(IconButtonShape::Square)
                    .icon_size(IconSize::Small)
                    .selected(self.include_tool_calls)
                    .tooltip(Tooltip::text("Include Tool Calls")),
            )
            .child(
                IconButton::new("thread-search-close", IconName::Close)
                    .shape(IconButtonShape::Square)
                    .icon_size(IconSize::Small)
                    .tooltip(Tooltip::text("Close Search")),
            );

        Some(bar.into_any_element())
    }
}
```

The buttons have no `.on_click` handlers yet — those come in Tasks 6-9. We're verifying the bar lays out correctly.

If `ui::h_flex` import path is unclear, look at how `thread_view.rs` imports `h_flex` already and mirror that.

- [ ] **Step 2: Render the bar inside `ThreadView`.**

In `crates/agent_ui/src/conversation_view/thread_view.rs`, find the `impl Render for ThreadView` block (around line 9091). Find where the main vertical layout (`v_flex`) starts and where children are added. The list of messages is rendered via `List::new(...)` somewhere in the tree.

Add the search bar as a child immediately above (before) wherever the list/messages container is added. The simplest spot is right at the top of the inner content. Look for a block like:
```rust
        v_flex()
            .key_context(...)
            // many .on_action calls
            .child(...)  // first child
```

Insert (chained into the same `v_flex`):
```rust
            .children(self.thread_search.render(cx))
```

`.children(Option<AnyElement>)` accepts `Option` and skips when `None`, so this is safe whether dismissed or not.

- [ ] **Step 3: Temporarily flip `dismissed` to verify the bar renders.**

In `ThreadSearch::new` (in `thread_search.rs`), change `dismissed: true,` to `dismissed: false,` *temporarily*.

- [ ] **Step 4: Run Zed and visually verify.**

Run: `cargo run --release --bin zed`
Open the agent panel. Expected: a search input bar appears at the top of the thread view with three buttons (up arrow, down arrow, hammer, X) and a "0/0" counter.

If something looks wrong (cut off, misaligned), note it but don't fix yet — we'll polish in Task 11.

- [ ] **Step 5: Revert the temporary flip.**

In `ThreadSearch::new`, change `dismissed: false,` back to `dismissed: true,`.

- [ ] **Step 6: Verify the bar disappears.**

Run Zed again. Expected: no search bar visible (because dismissed is true again).

- [ ] **Step 7: Commit.**

```bash
git add crates/agent_ui/src/conversation_view/thread_search.rs crates/agent_ui/src/conversation_view/thread_view.rs
git commit -m "agent_ui: Render thread search bar UI"
```

---

## Task 6: Wire `ToggleSearch` action to deploy/dismiss

**Files:**
- Modify: `crates/agent_ui/src/conversation_view/thread_search.rs` (add `deploy` and `dismiss` methods)
- Modify: `crates/agent_ui/src/conversation_view/thread_view.rs` (register `on_action`, focus management)

- [ ] **Step 1: Add `deploy` and `dismiss` methods to `ThreadSearch`.**

Append to `thread_search.rs`:

```rust
use gpui::FocusHandle;

impl ThreadSearch {
    pub fn deploy(&mut self, window: &mut Window, cx: &mut gpui::App) {
        self.dismissed = false;
        self.query_editor.update(cx, |editor, cx| {
            editor.focus_handle(cx).focus(window);
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
}
```

Note: `deploy` doesn't yet clear/refresh markdown highlights — that's Task 8. `dismiss` clears `matches` but doesn't yet remove highlights from markdown entities — Task 9.

- [ ] **Step 2: Register the `ToggleSearch` action handler on `ThreadView`.**

In `thread_view.rs`, find the `Render for ThreadView` impl (around line 9091). Find the existing `.on_action(...)` calls (e.g., the `scroll_output_*` ones around line 9140). Add a new handler:

```rust
            .on_action(cx.listener(|this, _: &crate::ToggleSearch, window, cx| {
                this.toggle_search(window, cx);
            }))
```

(`crate::ToggleSearch` because the action lives in `agent_ui.rs` (the crate root), and the `actions!` macro re-exports it at the crate level.)

- [ ] **Step 3: Add the `toggle_search` method to `ThreadView`.**

Find a logical home for new ThreadView methods (e.g., near `scroll_to_top` at line 5235). Add:

```rust
    fn toggle_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.thread_search.dismissed {
            self.thread_search.deploy(window, cx);
        } else {
            self.thread_search.dismiss(window, cx);
        }
        cx.notify();
    }
```

- [ ] **Step 4: Build and run.**

Run: `cargo run --release --bin zed`
Open the agent panel. Press `cmd-f` (macOS) / `ctrl-f` (Linux/Windows). Expected: search bar appears at top of thread, query editor is focused. Press `cmd-f` again. Expected: search bar disappears.

If `cmd-f` doesn't trigger anything: verify the keymap binding from Task 2 was saved correctly (open the keymap in the in-app editor and check for the binding).

- [ ] **Step 5: Wire the close button.**

In `thread_search.rs`, the `render` method currently has the close `IconButton` with no `.on_click`. Replace its construction with:

```rust
            .child(
                IconButton::new("thread-search-close", IconName::Close)
                    .shape(IconButtonShape::Square)
                    .icon_size(IconSize::Small)
                    .tooltip(Tooltip::text("Close Search"))
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(crate::ToggleSearch), cx);
                    }),
            )
```

- [ ] **Step 6: Verify clicking the close button dismisses the bar.**

Run Zed; deploy search; click the X. Expected: bar disappears.

- [ ] **Step 7: Commit.**

```bash
git add crates/agent_ui/src/conversation_view/thread_search.rs crates/agent_ui/src/conversation_view/thread_view.rs
git commit -m "agent_ui: Toggle thread search via cmd-f and close button"
```

---

## Task 7: Subscribe to query edits → rebuild matches

**Files:**
- Modify: `crates/agent_ui/src/conversation_view/thread_search.rs` (add a "rebuild requested" trigger)
- Modify: `crates/agent_ui/src/conversation_view/thread_view.rs` (subscribe to query editor inside `ThreadView::new`, run search on edits)

The architectural decision: `ThreadView` owns the subscription because it has access to `self.thread.read(cx).entries()` which is what we need to search.

- [ ] **Step 1: Define the text-extraction helper in `thread_search.rs`.**

Append to `thread_search.rs`:

```rust
use acp_thread::{
    AgentThreadEntry, AssistantMessageChunk, ContentBlock, ToolCallContent,
};

/// Walk the thread entries and produce one `(entry_index, markdown_entity)` per
/// piece of searchable text content. The `include_tool_calls` flag controls
/// whether tool call labels and content contribute.
pub fn collect_searchable_markdowns(
    entries: &[AgentThreadEntry],
    include_tool_calls: bool,
) -> Vec<(usize, Entity<Markdown>)> {
    let mut out = Vec::new();
    for (entry_index, entry) in entries.iter().enumerate() {
        match entry {
            AgentThreadEntry::UserMessage(msg) => {
                if let ContentBlock::Markdown { markdown } = &msg.content {
                    out.push((entry_index, markdown.clone()));
                }
            }
            AgentThreadEntry::AssistantMessage(msg) => {
                for chunk in &msg.chunks {
                    let block = match chunk {
                        AssistantMessageChunk::Message { block } => block,
                        AssistantMessageChunk::Thought { block } => block,
                    };
                    if let ContentBlock::Markdown { markdown } = block {
                        out.push((entry_index, markdown.clone()));
                    }
                }
            }
            AgentThreadEntry::ToolCall(tool_call) if include_tool_calls => {
                out.push((entry_index, tool_call.label.clone()));
                for content in &tool_call.content {
                    if let ToolCallContent::ContentBlock(ContentBlock::Markdown { markdown }) =
                        content
                    {
                        out.push((entry_index, markdown.clone()));
                    }
                }
            }
            AgentThreadEntry::ToolCall(_) | AgentThreadEntry::CompletedPlan(_) => {}
        }
    }
    out
}
```

**Note on `ToolCallContent`:** open `crates/acp_thread/src/acp_thread.rs` and verify the `ToolCallContent` enum variants. Adjust the pattern above to match the actual variant that contains a `ContentBlock`. If the variant is named differently (e.g., `ToolCallContent::Block(...)`, `ToolCallContent::Markdown(...)`), update accordingly. If `ToolCallContent` doesn't carry markdown at all, drop the inner `for content in ...` loop and only include `tool_call.label`.

- [ ] **Step 2: Add `update_matches` method on `ThreadSearch`.**

Append to `thread_search.rs`:

```rust
impl ThreadSearch {
    /// Recompute matches given the current query and a fresh list of
    /// (entry_index, markdown) pairs. Caller is responsible for applying the
    /// resulting highlights to each markdown entity.
    pub fn update_matches(
        &mut self,
        sources: &[(usize, Entity<Markdown>)],
        cx: &gpui::App,
    ) {
        let query = self.query(cx);
        self.matches.clear();
        if query.is_empty() {
            self.active_match_index = None;
            return;
        }
        for (entry_index, markdown) in sources {
            let source = markdown.read(cx).source().to_string();
            for range in find_matches_in_text(&query, &source) {
                self.matches.push(SearchMatch {
                    entry_index: *entry_index,
                    markdown: markdown.clone(),
                    range,
                });
            }
        }
        self.active_match_index = if self.matches.is_empty() { None } else { Some(0) };
    }
}
```

- [ ] **Step 3: Subscribe to query edits inside `ThreadView::new`.**

In `thread_view.rs`, in `ThreadView::new`, *after* `thread_search` is initialized, push a subscription onto `thread_search._subscriptions`:

```rust
        let query_editor = thread_search.query_editor.clone();
        thread_search._subscriptions.push(cx.subscribe_in(
            &query_editor,
            window,
            |this, _, event, _window, cx| {
                if let editor::EditorEvent::BufferEdited = event {
                    this.refresh_thread_search(cx);
                }
            },
        ));
```

Adjust if `thread_search` is built before `cx.subscribe_in` is reachable — you may need to construct `thread_search`, then subscribe, before placing into the struct literal.

- [ ] **Step 4: Add `refresh_thread_search` method on `ThreadView`.**

Add this method near `toggle_search`:

```rust
    fn refresh_thread_search(&mut self, cx: &mut Context<Self>) {
        let entries = self.thread.read(cx).entries();
        let sources = super::thread_search::collect_searchable_markdowns(
            entries,
            self.thread_search.include_tool_calls,
        );
        self.thread_search.update_matches(&sources, cx);
        cx.notify();
    }
```

- [ ] **Step 5: Verify match counter updates when typing.**

Run Zed. Open agent panel; have a thread with some text content. Press `cmd-f`; type a word that appears in the thread.

Expected: the match counter at the right of the search bar updates from `0/0` to `1/N` where N matches the number of occurrences. (Highlights aren't rendered yet — that's Task 8.)

- [ ] **Step 6: Commit.**

```bash
git add crates/agent_ui/src/conversation_view/thread_search.rs crates/agent_ui/src/conversation_view/thread_view.rs
git commit -m "agent_ui: Compute thread search matches on query change"
```

---

## Task 8: Apply highlights to markdown entities

**Files:**
- Modify: `crates/agent_ui/src/conversation_view/thread_search.rs` (add `apply_highlights`)
- Modify: `crates/agent_ui/src/conversation_view/thread_view.rs` (call after each `update_matches`)

- [ ] **Step 1: Add `apply_highlights` method on `ThreadSearch`.**

Append to `thread_search.rs`:

```rust
use std::collections::HashMap;

impl ThreadSearch {
    /// Push current matches into each Markdown entity's `set_search_highlights`,
    /// grouping match ranges by their owning markdown entity. Marks the global
    /// `active_match_index` as the active highlight on its specific markdown.
    pub fn apply_highlights(&self, cx: &mut gpui::App) {
        // Group: markdown entity_id → (markdown entity, ranges, optional active local index)
        let mut grouped: HashMap<u64, (Entity<Markdown>, Vec<Range<usize>>, Option<usize>)> =
            HashMap::new();

        for (i, m) in self.matches.iter().enumerate() {
            let key = m.markdown.entity_id().as_u64();
            let entry = grouped
                .entry(key)
                .or_insert_with(|| (m.markdown.clone(), Vec::new(), None));
            let local_index = entry.1.len();
            entry.1.push(m.range.clone());
            if Some(i) == self.active_match_index {
                entry.2 = Some(local_index);
            }
        }

        for (_, (markdown, ranges, active)) in grouped {
            markdown.update(cx, |markdown, cx| {
                markdown.set_search_highlights(ranges, active, cx);
            });
        }
    }

    /// Remove highlights from every markdown entity that currently shows them.
    pub fn clear_highlights(&self, cx: &mut gpui::App) {
        let mut seen: std::collections::HashSet<u64> = std::collections::HashSet::new();
        for m in &self.matches {
            let key = m.markdown.entity_id().as_u64();
            if seen.insert(key) {
                let markdown = m.markdown.clone();
                markdown.update(cx, |markdown, cx| {
                    markdown.clear_search_highlights(cx);
                });
            }
        }
    }
}
```

- [ ] **Step 2: Apply highlights after each refresh.**

In `thread_view.rs`, modify `refresh_thread_search`:

```rust
    fn refresh_thread_search(&mut self, cx: &mut Context<Self>) {
        // Clear stale highlights before recomputing — the matches Vec is about
        // to be reused, so we must clear based on the *previous* matches first.
        self.thread_search.clear_highlights(cx);
        let entries = self.thread.read(cx).entries();
        let sources = super::thread_search::collect_searchable_markdowns(
            entries,
            self.thread_search.include_tool_calls,
        );
        self.thread_search.update_matches(&sources, cx);
        self.thread_search.apply_highlights(cx);
        cx.notify();
    }
```

- [ ] **Step 3: Clear highlights on dismiss.**

In `thread_view.rs`, modify `toggle_search`:

```rust
    fn toggle_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.thread_search.dismissed {
            self.thread_search.deploy(window, cx);
            // Re-run search in case the thread changed since last deploy.
            self.refresh_thread_search(cx);
        } else {
            self.thread_search.clear_highlights(cx);
            self.thread_search.dismiss(window, cx);
        }
        cx.notify();
    }
```

- [ ] **Step 4: Verify highlights appear.**

Run Zed. Open agent panel; in a thread containing text, press `cmd-f`; type a word.

Expected: every occurrence of the word is highlighted in the rendered markdown across messages. The first match should look distinct (the active match is rendered with a stronger style by `Markdown`).

- [ ] **Step 5: Commit.**

```bash
git add crates/agent_ui/src/conversation_view/thread_search.rs crates/agent_ui/src/conversation_view/thread_view.rs
git commit -m "agent_ui: Highlight thread search matches in messages"
```

---

## Task 9: Implement next/prev navigation with scroll

**Files:**
- Modify: `crates/agent_ui/src/conversation_view/thread_search.rs` (add `select_next` / `select_prev`)
- Modify: `crates/agent_ui/src/conversation_view/thread_view.rs` (action handlers, scroll-to-active)

- [ ] **Step 1: Add navigation methods on `ThreadSearch`.**

Append to `thread_search.rs`:

```rust
impl ThreadSearch {
    /// Advance active match by `delta` (e.g. +1 for next, -1 for prev), wrapping.
    /// Returns the entry_index of the new active match, if any.
    pub fn step_active(&mut self, delta: isize) -> Option<usize> {
        if self.matches.is_empty() {
            self.active_match_index = None;
            return None;
        }
        let len = self.matches.len() as isize;
        let current = self.active_match_index.unwrap_or(0) as isize;
        let next = ((current + delta) % len + len) % len;
        self.active_match_index = Some(next as usize);
        Some(self.matches[next as usize].entry_index)
    }
}
```

- [ ] **Step 2: Add scroll-to-active and action handlers on `ThreadView`.**

Add methods near `toggle_search`:

```rust
    fn thread_search_select_next(
        &mut self,
        _: &menu::SelectNext,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.thread_search.dismissed {
            self.advance_thread_search(1, cx);
        }
    }

    fn thread_search_select_prev(
        &mut self,
        _: &menu::SelectPrevious,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.thread_search.dismissed {
            self.advance_thread_search(-1, cx);
        }
    }

    fn advance_thread_search(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(entry_index) = self.thread_search.step_active(delta) {
            self.thread_search.apply_highlights(cx);
            self.list_state.scroll_to(gpui::ListOffset {
                item_ix: entry_index,
                offset_in_item: gpui::px(0.0),
            });
            cx.notify();
        }
    }
```

- [ ] **Step 3: Register the action handlers.**

In `thread_view.rs`, in the `Render for ThreadView` impl, alongside the existing `on_action` calls (right after the `ToggleSearch` one from Task 6), add:

```rust
            .on_action(cx.listener(Self::thread_search_select_next))
            .on_action(cx.listener(Self::thread_search_select_prev))
```

These will only fire when the keymap context matches — i.e., when search has focus, per the keymap binding from Task 2.

Make sure `menu` is imported. It's already imported in `thread_view.rs` (used for `menu::Cancel`); reuse the same import.

- [ ] **Step 4: Wire the chevron buttons.**

In `thread_search.rs`, replace the prev/next `IconButton`s in `render` with click handlers. Find the `IconButton::new("thread-search-prev", ...)` and `IconButton::new("thread-search-next", ...)` blocks and add `.on_click(...)`:

```rust
            .child(
                IconButton::new("thread-search-prev", IconName::ChevronUp)
                    .shape(IconButtonShape::Square)
                    .icon_size(IconSize::Small)
                    .tooltip(Tooltip::text("Previous Match"))
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(menu::SelectPrevious), cx);
                    }),
            )
            .child(
                IconButton::new("thread-search-next", IconName::ChevronDown)
                    .shape(IconButtonShape::Square)
                    .icon_size(IconSize::Small)
                    .tooltip(Tooltip::text("Next Match"))
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(menu::SelectNext), cx);
                    }),
            )
```

Add `use menu;` to `thread_search.rs` imports if not already imported.

- [ ] **Step 5: Verify navigation.**

Run Zed. Open agent panel; deploy search; type a word with multiple matches. Press `enter` repeatedly — active highlight cycles forward and the list scrolls so the entry containing the active match is visible. Press `shift-enter` — cycles backward. Click the chevron buttons — same behavior.

- [ ] **Step 6: Commit.**

```bash
git add crates/agent_ui/src/conversation_view/thread_search.rs crates/agent_ui/src/conversation_view/thread_view.rs
git commit -m "agent_ui: Navigate between thread search matches with enter/shift-enter"
```

---

## Task 10: Implement tool calls toggle

**Files:**
- Modify: `crates/agent_ui/src/conversation_view/thread_search.rs` (toggle button click)
- Modify: `crates/agent_ui/src/conversation_view/thread_view.rs` (re-run search on toggle)

The pure logic is already in place (`collect_searchable_markdowns` already takes `include_tool_calls: bool`). We just need to wire the button.

The challenge: `ThreadSearch::render` doesn't have access to `Context<ThreadView>` to dispatch a typed callback into ThreadView. Solution: dispatch a new sub-action that ThreadView handles.

- [ ] **Step 1: Add a new action.**

In `crates/agent_ui/src/agent_ui.rs`, inside the same `actions!(agent, [...])` macro (right after `ToggleSearch` from Task 1), add:

```rust
        /// Toggles whether thread search includes tool call titles and outputs.
        ToggleSearchIncludeToolCalls,
```

- [ ] **Step 2: Wire the toggle button to dispatch the action.**

In `thread_search.rs`, replace the tool-calls `IconButton` with:

```rust
            .child(
                IconButton::new("thread-search-toggle-tools", IconName::ToolHammer)
                    .shape(IconButtonShape::Square)
                    .icon_size(IconSize::Small)
                    .selected(self.include_tool_calls)
                    .tooltip(Tooltip::text("Include Tool Calls"))
                    .on_click(|_, window, cx| {
                        window.dispatch_action(
                            Box::new(crate::ToggleSearchIncludeToolCalls),
                            cx,
                        );
                    }),
            )
```

- [ ] **Step 3: Handle the action in `ThreadView`.**

Add a method:

```rust
    fn toggle_search_include_tool_calls(
        &mut self,
        _: &crate::ToggleSearchIncludeToolCalls,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.thread_search.dismissed {
            return;
        }
        self.thread_search.include_tool_calls = !self.thread_search.include_tool_calls;
        self.refresh_thread_search(cx);
    }
```

Register it in the `Render` impl alongside other action handlers:
```rust
            .on_action(cx.listener(Self::toggle_search_include_tool_calls))
```

- [ ] **Step 4: Verify toggle behavior.**

Run Zed. Open a thread that has tool calls. Deploy search; type a word that appears in a tool call's title or output but NOT in any message. Expected: counter shows `0/0`. Click the hammer toggle. Expected: counter updates to show matches found in tool calls; their highlights appear.

Toggle off: matches return to `0/0` and highlights disappear from tool calls.

- [ ] **Step 5: Commit.**

```bash
git add crates/agent_ui/src/agent_ui.rs crates/agent_ui/src/conversation_view/thread_search.rs crates/agent_ui/src/conversation_view/thread_view.rs
git commit -m "agent_ui: Toggle including tool calls in thread search"
```

---

## Task 11: Add case-sensitivity, whole-word, and regex toggles

**Files:**
- Modify: `crates/agent_ui/src/agent_ui.rs` (add three new actions)
- Modify: `crates/agent_ui/src/conversation_view/thread_search.rs` (extend matcher, add toggle buttons)
- Modify: `crates/agent_ui/src/conversation_view/thread_view.rs` (action handlers)
- Modify: `Cargo.toml` for agent_ui (add `regex` dep if not already present)

This task brings the search bar in line with the user's "Full" requirement — matching the BufferSearchBar feature set for these three toggles.

- [ ] **Step 1: Verify `regex` is available to `agent_ui`.**

Run: `cargo tree -p agent_ui -e normal | grep '^[a-z]*regex'`
If `regex v1.X` shows up (it likely does — many Zed crates use it transitively), confirm it's a *direct* dep by inspecting `crates/agent_ui/Cargo.toml`. If not present, add it under `[dependencies]`:
```toml
regex.workspace = true
```
(Use `.workspace = true` if the workspace's root `Cargo.toml` already pins `regex`. Check `Cargo.toml` at the repo root for `regex = ` first; if it's not in `[workspace.dependencies]`, then use the most current version from the regex crate's actual current release rather than a guessed version.)

- [ ] **Step 2: Add three new actions.**

In `crates/agent_ui/src/agent_ui.rs`, inside `actions!(agent, [...])` (right after `ToggleSearchIncludeToolCalls` from Task 10):

```rust
        /// Toggles case-sensitive matching in the thread search bar.
        ToggleSearchCaseSensitive,
        /// Toggles whole-word matching in the thread search bar.
        ToggleSearchWholeWord,
        /// Toggles regex matching in the thread search bar.
        ToggleSearchRegex,
```

- [ ] **Step 3: Refactor `find_matches_in_text` to accept `SearchOptions`.**

In `crates/agent_ui/src/conversation_view/thread_search.rs`, replace the existing `find_matches_in_text` and its `#[cfg(test)] mod tests` with the version below.

```rust
/// Find every match of `query` in `text`, honoring `options`.
///
/// Returns ranges over byte offsets into `text` (suitable for
/// [`markdown::Markdown::set_search_highlights`]). Empty queries return no
/// matches. Matches do not overlap. Invalid regex (when `options.regex` is
/// true) returns no matches.
pub fn find_matches_in_text(
    query: &str,
    text: &str,
    options: SearchOptions,
) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    if options.regex {
        let mut builder = regex::RegexBuilder::new(query);
        builder.case_insensitive(!options.case_sensitive);
        let re = match builder.build() {
            Ok(re) => re,
            Err(_) => return Vec::new(),
        };
        let mut matches = Vec::new();
        for m in re.find_iter(text) {
            if !m.is_empty() {
                matches.push(m.start()..m.end());
            }
        }
        return matches;
    }

    // Plain substring (with optional case-insensitivity and whole-word filter).
    let needle: String;
    let haystack: String;
    let (needle_ref, haystack_ref): (&str, &str) = if options.case_sensitive {
        (query, text)
    } else {
        needle = query.to_lowercase();
        haystack = text.to_lowercase();
        (needle.as_str(), haystack.as_str())
    };

    let mut matches = Vec::new();
    let mut start = 0;
    while start < haystack_ref.len() {
        let Some(rel) = haystack_ref[start..].find(needle_ref) else {
            break;
        };
        let match_start = start + rel;
        let match_end = match_start + needle_ref.len();
        if !options.whole_word || is_whole_word(text, match_start..match_end) {
            matches.push(match_start..match_end);
        }
        start = match_end;
    }
    matches
}

fn is_whole_word(text: &str, range: Range<usize>) -> bool {
    let before_ok = range.start == 0
        || text[..range.start]
            .chars()
            .next_back()
            .map(|c| !is_word_char(c))
            .unwrap_or(true);
    let after_ok = range.end == text.len()
        || text[range.end..]
            .chars()
            .next()
            .map(|c| !is_word_char(c))
            .unwrap_or(true);
    before_ok && after_ok
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> SearchOptions { SearchOptions::default() }

    #[test]
    fn empty_query_returns_no_matches() {
        assert!(find_matches_in_text("", "hello world", opts()).is_empty());
    }

    #[test]
    fn no_matches_for_missing_substring() {
        assert!(find_matches_in_text("xyz", "hello world", opts()).is_empty());
    }

    #[test]
    fn finds_single_match() {
        assert_eq!(find_matches_in_text("world", "hello world", opts()), vec![6..11]);
    }

    #[test]
    fn finds_multiple_non_overlapping_matches() {
        assert_eq!(
            find_matches_in_text("ab", "abcabcab", opts()),
            vec![0..2, 3..5, 6..8],
        );
    }

    #[test]
    fn case_insensitive_by_default() {
        assert_eq!(
            find_matches_in_text("Hello", "hello HELLO HeLLo", opts()),
            vec![0..5, 6..11, 12..17],
        );
    }

    #[test]
    fn case_sensitive_option() {
        let mut o = opts();
        o.case_sensitive = true;
        assert_eq!(find_matches_in_text("Hello", "hello HELLO Hello", o), vec![12..17]);
    }

    #[test]
    fn advances_past_each_match_no_overlap() {
        assert_eq!(find_matches_in_text("aa", "aaa", opts()), vec![0..2]);
    }

    #[test]
    fn whole_word_excludes_substrings() {
        let mut o = opts();
        o.whole_word = true;
        assert_eq!(
            find_matches_in_text("cat", "cat catalog scatter cat.", o),
            vec![0..3, 20..23],
        );
    }

    #[test]
    fn regex_basic() {
        let mut o = opts();
        o.regex = true;
        assert_eq!(
            find_matches_in_text(r"h\w+o", "hello world hippo", o),
            vec![0..5, 12..17],
        );
    }

    #[test]
    fn regex_case_sensitive() {
        let mut o = opts();
        o.regex = true;
        o.case_sensitive = true;
        assert_eq!(find_matches_in_text(r"H\w+", "hello Hello", o), vec![6..11]);
    }

    #[test]
    fn invalid_regex_returns_no_matches() {
        let mut o = opts();
        o.regex = true;
        assert!(find_matches_in_text("[unclosed", "test", o).is_empty());
    }
}
```

- [ ] **Step 4: Update the `update_matches` call site to pass options.**

In `thread_search.rs`, in `ThreadSearch::update_matches`, change the call from:
```rust
            for range in find_matches_in_text(&query, &source) {
```
to:
```rust
            for range in find_matches_in_text(&query, &source, self.options) {
```

- [ ] **Step 5: Run the unit tests.**

Run: `cargo test -p agent_ui --lib conversation_view::thread_search::tests`
Expected: 11 tests pass (6 original-equivalent + 5 new).

- [ ] **Step 6: Add toggle buttons to the search bar UI.**

In `thread_search.rs`, in the `render` method, add three more `IconButton` children between the tool-calls toggle and the close button. Replace your current toggle / close children block with:

```rust
            .child(
                IconButton::new("thread-search-case", IconName::CaseSensitive)
                    .shape(IconButtonShape::Square)
                    .icon_size(IconSize::Small)
                    .selected(self.options.case_sensitive)
                    .tooltip(Tooltip::text("Match Case"))
                    .on_click(|_, window, cx| {
                        window.dispatch_action(
                            Box::new(crate::ToggleSearchCaseSensitive),
                            cx,
                        );
                    }),
            )
            .child(
                IconButton::new("thread-search-word", IconName::WholeWord)
                    .shape(IconButtonShape::Square)
                    .icon_size(IconSize::Small)
                    .selected(self.options.whole_word)
                    .tooltip(Tooltip::text("Match Whole Word"))
                    .on_click(|_, window, cx| {
                        window.dispatch_action(
                            Box::new(crate::ToggleSearchWholeWord),
                            cx,
                        );
                    }),
            )
            .child(
                IconButton::new("thread-search-regex", IconName::Regex)
                    .shape(IconButtonShape::Square)
                    .icon_size(IconSize::Small)
                    .selected(self.options.regex)
                    .tooltip(Tooltip::text("Use Regex"))
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(crate::ToggleSearchRegex), cx);
                    }),
            )
            .child(
                IconButton::new("thread-search-toggle-tools", IconName::ToolHammer)
                    .shape(IconButtonShape::Square)
                    .icon_size(IconSize::Small)
                    .selected(self.include_tool_calls)
                    .tooltip(Tooltip::text("Include Tool Calls"))
                    .on_click(|_, window, cx| {
                        window.dispatch_action(
                            Box::new(crate::ToggleSearchIncludeToolCalls),
                            cx,
                        );
                    }),
            )
            .child(
                IconButton::new("thread-search-close", IconName::Close)
                    .shape(IconButtonShape::Square)
                    .icon_size(IconSize::Small)
                    .tooltip(Tooltip::text("Close Search"))
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(crate::ToggleSearch), cx);
                    }),
            )
```

If `IconName::WholeWord` or `IconName::Regex` don't exist in `crates/icons/src/icons.rs`, substitute the closest available icons (e.g., `IconName::Quote` for whole-word, `IconName::Code` for regex) — or grep the icons file for them: `grep -E "WholeWord|Regex" crates/icons/src/icons.rs`. Use what's actually there.

- [ ] **Step 7: Add action handlers in `ThreadView`.**

Add three methods near `toggle_search_include_tool_calls`:

```rust
    fn toggle_search_case_sensitive(
        &mut self,
        _: &crate::ToggleSearchCaseSensitive,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.thread_search.dismissed {
            return;
        }
        self.thread_search.options.case_sensitive = !self.thread_search.options.case_sensitive;
        self.refresh_thread_search(cx);
    }

    fn toggle_search_whole_word(
        &mut self,
        _: &crate::ToggleSearchWholeWord,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.thread_search.dismissed {
            return;
        }
        self.thread_search.options.whole_word = !self.thread_search.options.whole_word;
        self.refresh_thread_search(cx);
    }

    fn toggle_search_regex(
        &mut self,
        _: &crate::ToggleSearchRegex,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.thread_search.dismissed {
            return;
        }
        self.thread_search.options.regex = !self.thread_search.options.regex;
        self.refresh_thread_search(cx);
    }
```

Register them in `Render for ThreadView`:
```rust
            .on_action(cx.listener(Self::toggle_search_case_sensitive))
            .on_action(cx.listener(Self::toggle_search_whole_word))
            .on_action(cx.listener(Self::toggle_search_regex))
```

- [ ] **Step 8: Verify in Zed.**

Run: `cargo run --release --bin zed`
Open agent panel, deploy search. Click each new toggle:
- Case toggle: "Hello" search should change from matching all "hello" / "HELLO" / "Hello" to only "Hello".
- Whole word toggle: searching "cat" should stop matching "catalog".
- Regex toggle: enables typing patterns like `\b\w+ing\b`.

- [ ] **Step 9: Commit.**

```bash
git add crates/agent_ui/Cargo.toml crates/agent_ui/src/agent_ui.rs crates/agent_ui/src/conversation_view/thread_search.rs crates/agent_ui/src/conversation_view/thread_view.rs
git commit -m "agent_ui: Add case-sensitivity, whole-word, and regex toggles to thread search"
```

---

## Task 12: Add the search button to the agent panel toolbar

**Files:**
- Modify: `crates/agent_ui/src/agent_panel.rs` (add `IconButton` to `render_toolbar`)

- [ ] **Step 1: Find a sensible location in `render_toolbar`.**

In `crates/agent_ui/src/agent_panel.rs`, the `render_toolbar` function is at line 4235. Read through it to find where existing IconButtons are added at the trailing edge of the toolbar (e.g., near the options menu, history button). The toolbar is built up via `h_flex()` chained `.child(...)` calls — find the last `.child()` before the function returns its element, OR find a logical group like a `h_flex().gap_1()` that holds the right-hand-side action buttons.

A common pattern in this file: there's a "buttons" section (often the right-aligned action group). Insert the search button into that group.

- [ ] **Step 2: Add the search `IconButton`.**

The exact insertion will depend on the toolbar's structure when you read it. As an example, if there's an `h_flex().child(button1).child(button2)` you'd add:

```rust
            .child(
                IconButton::new("thread-search-toggle", IconName::MagnifyingGlass)
                    .shape(IconButtonShape::Square)
                    .icon_size(IconSize::Small)
                    .tooltip({
                        let focus_handle = focus_handle.clone();
                        move |window, cx| {
                            Tooltip::for_action_in(
                                "Search Thread",
                                &crate::ToggleSearch,
                                &focus_handle,
                                window,
                                cx,
                            )
                        }
                    })
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(crate::ToggleSearch), cx);
                    }),
            )
```

`focus_handle` is already available in `render_toolbar` (declared at line 4238). `Tooltip::for_action_in` displays the keyboard shortcut next to the tooltip label — verify the exact import path by looking at how other buttons in this file use it. If `Tooltip::for_action_in` isn't the right API, fall back to `Tooltip::text("Search Thread (cmd-f)")`.

- [ ] **Step 3: Conditionally show the button only when there's a thread.**

Wrap the button in `.when(...)` so it only appears when an agent thread is active. Look for how other buttons use `active_thread.is_some()` (computed earlier in `render_toolbar` at line 4258) or check `self.visible_surface()` for `VisibleSurface::AgentThread`. Existing toolbar buttons in this file already follow that pattern — mirror them.

```rust
            .when(active_thread.is_some(), |this| {
                this.child(
                    IconButton::new("thread-search-toggle", IconName::MagnifyingGlass)
                        ...
                )
            })
```

- [ ] **Step 4: Build, run, visually verify.**

Run: `cargo run --release --bin zed`
Open the agent panel with an active thread. Expected: a magnifying glass icon appears in the toolbar. Hover: tooltip says "Search Thread" with the cmd-f shortcut. Click: search bar appears.

Open the agent panel with no thread (or the terminal view): button should be hidden.

- [ ] **Step 5: Commit.**

```bash
git add crates/agent_ui/src/agent_panel.rs
git commit -m "agent_ui: Add search button to agent panel toolbar"
```

---

## Task 13: Add the `ThreadSearch` key context for in-search keybindings

**Files:**
- Modify: `crates/agent_ui/src/conversation_view/thread_view.rs` (extend the dispatch context when search is focused)

The keymap in Task 2 binds `escape`, `enter`, `shift-enter` under `"AgentPanel > ThreadSearch"`. We need `ThreadView` to add `ThreadSearch` to the dispatch context when the query editor has focus.

- [ ] **Step 1: Locate the `key_context` method on `ThreadView`.**

In `crates/agent_ui/src/conversation_view/thread_view.rs`, search for `fn key_context` or look for where `KeyContext` is set up. If it doesn't exist (the thread view may inherit context from its parent), check `Render for ThreadView` for any `.key_context(...)` call.

If `ThreadView` does NOT currently set its own `key_context`, add one to its render:
```rust
        v_flex()
            .key_context({
                let mut context = gpui::KeyContext::new_with_defaults();
                context.add("ThreadView");
                if !self.thread_search.dismissed
                    && self
                        .thread_search
                        .query_editor_focus_handle(cx)
                        .contains_focused(window, cx)
                {
                    context.add("ThreadSearch");
                }
                context
            })
```

If `ThreadView` *does* already set its own key context, augment that block to add `"ThreadSearch"` conditionally.

Note: the *parent* `AgentPanel` already adds `"AgentPanel"` (from `agent_panel.rs:4905`), so the combined context becomes `"AgentPanel ThreadSearch"` — which matches the keymap selector `"AgentPanel > ThreadSearch"` when interpreted as "AgentPanel has descendant ThreadSearch." (`>` in keymap context is typically a descendant operator; if your test shows the bindings don't fire, change the keymap context strings from `"AgentPanel > ThreadSearch"` to `"AgentPanel ThreadSearch"` (no `>`) or `"ThreadSearch"` alone, and re-test.)

- [ ] **Step 2: Verify escape dismisses the bar; enter cycles matches.**

Run Zed. Deploy search. Type a query so there are matches. Press `escape` — bar dismisses. Deploy again, press `enter` — active match advances; `shift-enter` — active match goes back.

- [ ] **Step 3: Commit.**

```bash
git add crates/agent_ui/src/conversation_view/thread_view.rs
git commit -m "agent_ui: Activate ThreadSearch key context when search is focused"
```

---

## Task 14: Integration test — deploy, search, navigate, dismiss

**Files:**
- Modify: `crates/agent_ui/src/agent_panel.rs` (add a new `#[gpui::test]` near the existing tests around line 7845)

This test verifies the end-to-end deploy → type → match → dismiss flow at the entity level.

- [ ] **Step 1: Locate the existing test module pattern.**

In `crates/agent_ui/src/agent_panel.rs`, find the existing `#[gpui::test] async fn test_draft_replaced_when_selected_agent_changes` at line 7845 and the surrounding `init_test` helper. Use them as a template.

- [ ] **Step 2: Write the new test.**

After the last existing test in the file (or in a logical adjacent location), add:

```rust
#[gpui::test]
async fn test_thread_search_finds_matches_across_messages(cx: &mut TestAppContext) {
    init_test(cx);
    let fs = FakeFs::new(cx.executor());
    cx.update(|cx| {
        agent::ThreadStore::init_global(cx);
        language_model::LanguageModelRegistry::test(cx);
        <dyn fs::Fs>::set_global(fs.clone(), cx);
    });

    fs.insert_tree("/project", json!({ "file.txt": "" })).await;
    let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;

    let multi_workspace =
        cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));

    let workspace = multi_workspace
        .read_with(cx, |multi_workspace, _cx| {
            multi_workspace.workspace().clone()
        })
        .unwrap();

    // Open the agent panel and seed two messages with the word "alpha" in each.
    // (Adjust this section based on whichever helpers exist for adding test
    // messages. Look for how other tests in this file create messages — for
    // example, the test at line 7935 may show the API for sending a user
    // message into a thread.)
    //
    // The below is illustrative — replace with actual API once you've seen the
    // surrounding tests:
    let panel = workspace
        .update(cx, |_workspace, _window, _cx| {
            // open or fetch agent panel
            todo!("use the appropriate helper from sibling tests")
        })
        .unwrap();
    let _ = panel; // silence warning until filled in
}
```

The test as written is a skeleton. Look at the tests near line 7935 (or wherever messages are programmatically added) to find the actual API for seeding thread content and accessing the `ThreadView`. Then complete the test with assertions like:

```rust
    panel.update(cx, |panel, window, cx| {
        let thread_view = panel.active_conversation_view().unwrap()
            .read(cx).as_native_thread_view().unwrap();
        thread_view.update(cx, |tv, cx| {
            tv.toggle_search(window, cx);
            tv.thread_search.query_editor.update(cx, |editor, cx| {
                editor.set_text("alpha", window, cx);
            });
            tv.refresh_thread_search(cx);
            assert_eq!(tv.thread_search.matches.len(), 2);
            assert_eq!(tv.thread_search.active_match_index, Some(0));

            tv.advance_thread_search(1, cx);
            assert_eq!(tv.thread_search.active_match_index, Some(1));

            tv.advance_thread_search(1, cx);
            assert_eq!(tv.thread_search.active_match_index, Some(0));  // wraps

            tv.toggle_search(window, cx);
            assert!(tv.thread_search.dismissed);
            assert!(tv.thread_search.matches.is_empty());
        });
    });
```

If the precise way to access `ThreadView` from `ConversationView` differs from `as_native_thread_view()`, look at `agent_panel.rs:4258-4262` for the right accessor pattern (`conversation_view.read(cx).as_native_thread(cx)` returns the underlying thread; you may need a sibling `as_native_thread_view` accessor — if it doesn't exist, simply skip the integration test for now and move to Task 14 since the unit tests in Task 3 cover the matcher logic).

- [ ] **Step 3: Run the test.**

Run: `cargo test -p agent_ui test_thread_search_finds_matches_across_messages`
Expected: PASS.

If the test setup is too involved to wire up cleanly in one task, mark it `#[ignore]` with a comment explaining what's needed and move on. The unit tests in Task 3 still provide regression coverage of the core matcher.

- [ ] **Step 4: Commit.**

```bash
git add crates/agent_ui/src/agent_panel.rs
git commit -m "agent_ui: Test thread search match navigation"
```

---

## Task 15: Run clippy and final cleanup

- [ ] **Step 1: Run clippy on the affected crate.**

Run: `./script/clippy -p agent_ui`
Expected: No new warnings introduced. Address any unused-import or deadcode warnings on code you added (do NOT touch unrelated warnings).

- [ ] **Step 2: Run the full agent_ui test suite.**

Run: `cargo test -p agent_ui`
Expected: All tests pass.

- [ ] **Step 3: Smoke test the feature end-to-end one more time.**

Run: `cargo run --release --bin zed`

Walk through:
1. Open agent panel with no thread → search button hidden.
2. Start a thread, send a few messages → search button visible.
3. Click search button → bar opens.
4. Type a word → counter updates, matches highlight in messages.
5. Press `enter` → active highlight advances, list scrolls.
6. Press `shift-enter` → active highlight goes back, wraps at start.
7. Toggle hammer button → tool call matches included; counter changes.
8. Toggle off → tool call matches gone.
9. Press `escape` → bar dismisses, all highlights cleared.
10. Press `cmd-f` → bar reopens.
11. Click X button → bar dismisses.

Each step should work as described. Note any visual issues for follow-up.

- [ ] **Step 4: Final commit if any cleanup was needed.**

```bash
git status
# If dirty:
git add -A
git commit -m "agent_ui: Polish thread search"
```

---

## Summary

When all 15 tasks are complete, the agent panel will have:
- A search button in its toolbar (only when an agent thread is open).
- `cmd-f` (macOS) / `ctrl-f` (Linux/Windows) opens an in-panel search bar.
- The bar contains: query input, match counter, prev/next chevrons, case/whole-word/regex toggles, tool-calls toggle, close button.
- Typing matches messages (case-insensitively by default); matches highlight in-place using the existing `Markdown::set_search_highlights` API.
- `enter` / `shift-enter` (or chevron clicks) cycle through matches; the list scrolls to keep the active match visible.
- Toggles for case sensitivity, whole-word matching, and regex bring the bar to feature parity with Zed's `BufferSearchBar` for the same options.
- Toggling the hammer button includes/excludes tool call labels and outputs from the search.
- `escape` (or X button, or `cmd-f` again) dismisses the bar and clears all highlights.
