# Agent Chat Prompt History Dropdown Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Commit policy:** Per the user's CLAUDE.md, **do not run `git commit`**. Each task ends at a "Stop and report" step. The user will decide when to commit.

**Goal:** Add a history button to the agent panel's top toolbar that opens a dropdown listing every user prompt in the current thread; clicking an entry scrolls that prompt into view in the chat.

**Architecture:** Add two pure helpers on `ThreadView` (one to enumerate user prompts with truncated previews, one to scroll to a given entry index). Add a `PopoverMenu<ContextMenu>` to `AgentPanel::render_toolbar`'s right-aligned button group; the menu is built lazily from the active `ThreadView`'s helpers on open. The chat list (`ListState`) is already lazy-loaded — we only iterate `thread.entries()`, which is the loaded-this-session set, so no extra work is needed for the lazy-load requirement.

**Tech Stack:** Rust, GPUI (`PopoverMenu`, `ContextMenu`, `ContextMenuEntry`, `ListState`, `ListOffset`), Zed `ui` crate icons (`IconName::HistoryRerun`).

---

## File Structure

**Modify:**
- `crates/agent_ui/src/conversation_view/thread_view.rs`
  - Adds two `pub(crate)` methods on `ThreadView`: `user_prompt_previews(cx) -> Vec<(usize, SharedString)>` and `scroll_to_user_prompt(entry_ix, cx)`.
  - Adds a unit test in the existing `tests` submodule (located near the bottom of `crates/agent_ui/src/conversation_view.rs`) verifying the helpers.
- `crates/agent_ui/src/agent_panel.rs`
  - Adds `prompt_history_menu_handle: PopoverMenuHandle<ContextMenu>` field to `AgentPanel` and initializes it in `AgentPanel::new`.
  - Adds the prompt-history button to the active-thread branch of `render_toolbar` (lines ~4676–4686), placed before the `new_thread_menu` so the high-frequency "New Thread" button stays in its current position.

**No new files** — the feature is small enough to live inside the two existing files.

---

## Task 1: ThreadView helpers — `user_prompt_previews` and `scroll_to_user_prompt`

**Files:**
- Modify: `crates/agent_ui/src/conversation_view/thread_view.rs` near line 5191 (next to existing `scroll_to_most_recent_user_prompt`)
- Test: add a `#[gpui::test]` to the existing `tests` module in `crates/agent_ui/src/conversation_view.rs`

**Context the implementer needs:**
- `ThreadView::thread: Entity<AcpThread>` exposes `.entries() -> &[AgentThreadEntry]` (in `crates/acp_thread/src/acp_thread.rs:1404`).
- The variant we want is `AgentThreadEntry::UserMessage(UserMessage)` (acp_thread.rs:207).
- `UserMessage::content: ContentBlock` has `to_markdown(&self, cx: &App) -> &str` (acp_thread.rs:815) — this returns the raw source markdown.
- `ThreadView::list_state: ListState` (thread_view.rs:293) is the scroll handle. The existing canonical scroll-to call is at thread_view.rs:5203:
  ```rust
  self.list_state.scroll_to(ListOffset {
      item_ix: ix,
      offset_in_item: px(0.0),
  });
  cx.notify();
  ```

- [ ] **Step 1: Write the failing test**

Append to the `tests` module in `crates/agent_ui/src/conversation_view.rs` (find it near `pub(crate) mod tests` around line 3074; the existing helper `setup_conversation_view` and `send_message` from `agent_panel.rs:5212` show how to drive a thread).

```rust
#[gpui::test]
async fn test_user_prompt_previews_and_scroll(cx: &mut TestAppContext) {
    init_test(cx);
    let (conversation_view, cx) =
        setup_conversation_view(StubAgentServer::default_response(), cx).await;

    send_message(&conversation_view, "First prompt", cx).await;
    send_message(&conversation_view, "Second prompt\nwith multiple lines", cx).await;
    send_message(&conversation_view, "", cx).await; // empty — should be skipped

    let thread_view = conversation_view
        .read_with(cx, |cv, _| cv.root_thread_view())
        .expect("active thread view");

    let previews = thread_view.update(cx, |tv, cx| tv.user_prompt_previews(cx));

    // Empty prompts are skipped; multi-line prompts collapse to first line.
    assert_eq!(previews.len(), 2);
    assert_eq!(previews[0].1.as_ref(), "First prompt");
    assert_eq!(previews[1].1.as_ref(), "Second prompt");

    // scroll_to_user_prompt must not panic for valid + out-of-range indices.
    thread_view.update(cx, |tv, cx| {
        tv.scroll_to_user_prompt(previews[0].0, cx);
        tv.scroll_to_user_prompt(9999, cx); // out of range — no-op
    });
}
```

- [ ] **Step 2: Run the test to verify it fails to compile**

Run from the repo root:

```
cargo test -p agent_ui --lib test_user_prompt_previews_and_scroll -- --nocapture
```

Expected: build error — `no method named user_prompt_previews / scroll_to_user_prompt found for ThreadView`.

- [ ] **Step 3: Add the helpers on `ThreadView`**

Insert just **above** the existing `scroll_to_most_recent_user_prompt` method at `crates/agent_ui/src/conversation_view/thread_view.rs:5191`:

```rust
/// Returns `(entry_index, preview)` for every non-empty user prompt in the current
/// thread, in submission order. Multi-line prompts collapse to their first non-empty
/// line; long previews are truncated with an ellipsis.
pub(crate) fn user_prompt_previews(
    &self,
    cx: &App,
) -> Vec<(usize, SharedString)> {
    const MAX_PREVIEW_CHARS: usize = 80;
    let entries = self.thread.read(cx).entries();
    let mut out = Vec::new();
    for (ix, entry) in entries.iter().enumerate() {
        if let AgentThreadEntry::UserMessage(message) = entry {
            let source = message.content.to_markdown(cx);
            let first_line = source
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty());
            let Some(line) = first_line else { continue };
            let preview: SharedString = if line.chars().count() > MAX_PREVIEW_CHARS {
                let truncated: String = line.chars().take(MAX_PREVIEW_CHARS).collect();
                format!("{truncated}…").into()
            } else {
                line.to_string().into()
            };
            out.push((ix, preview));
        }
    }
    out
}

/// Scrolls the chat so the entry at `entry_ix` is anchored to the top of the
/// viewport. No-op if the index is out of range.
pub(crate) fn scroll_to_user_prompt(
    &mut self,
    entry_ix: usize,
    cx: &mut Context<Self>,
) {
    let entries_len = self.thread.read(cx).entries().len();
    if entry_ix >= entries_len {
        return;
    }
    self.list_state.scroll_to(ListOffset {
        item_ix: entry_ix,
        offset_in_item: px(0.0),
    });
    cx.notify();
}
```

If `SharedString` is not already in scope at the top of `thread_view.rs`, add it to the existing `use ui::{...}` or `use gpui::{...}` import (search for `SharedString` already in the file — Zed uses it everywhere, so it's almost certainly imported; only add if missing).

- [ ] **Step 4: Run the test and verify it passes**

```
cargo test -p agent_ui --lib test_user_prompt_previews_and_scroll -- --nocapture
```

Expected: PASS, 1 test passed.

- [ ] **Step 5: Run clippy on the touched files**

```
./script/clippy -p agent_ui
```

Expected: no new warnings on `thread_view.rs` or `conversation_view.rs`.

- [ ] **Step 6: Stop and report**

Tell the user the helpers are in place and tested. Do **not** commit. Wait for the user to either say "commit" or say "continue."

---

## Task 2: Add `prompt_history_menu_handle` to `AgentPanel`

**Files:**
- Modify: `crates/agent_ui/src/agent_panel.rs` (struct definition and `AgentPanel::new` constructor)

**Context the implementer needs:**
- `AgentPanel` already holds several `PopoverMenuHandle<ContextMenu>` fields — search for `PopoverMenuHandle<ContextMenu>` in `agent_panel.rs` to find the existing handles (e.g. the panel options menu handle) and mimic the field declaration + initialization. The same pattern is used inside `ThreadView` at thread_view.rs:286 (`pub permission_dropdown_handle: PopoverMenuHandle<ContextMenu>`).

- [ ] **Step 1: Add the field**

Find the `pub struct AgentPanel { ... }` declaration in `crates/agent_ui/src/agent_panel.rs`. Add a new field next to the existing `PopoverMenuHandle<ContextMenu>` fields:

```rust
prompt_history_menu_handle: PopoverMenuHandle<ContextMenu>,
```

- [ ] **Step 2: Initialize the field in `AgentPanel::new`**

Find `impl AgentPanel { pub fn new(...) -> Self` (or the analogous constructor — grep for `Self {` inside `agent_panel.rs` to find where the struct is built). In the struct-literal initialization, add:

```rust
prompt_history_menu_handle: PopoverMenuHandle::default(),
```

If `PopoverMenuHandle` is not yet imported, it lives in the `ui` crate — the existing `use ui::...` import in `agent_panel.rs` should already pull it in (verify by searching the file for `PopoverMenuHandle`; it's already used for other menus).

- [ ] **Step 3: Build to verify**

```
cargo build -p agent_ui
```

Expected: clean build.

- [ ] **Step 4: Stop and report**

Tell the user Task 2 is done. Do not commit.

---

## Task 3: Add the prompt-history button + dropdown to the toolbar

**Files:**
- Modify: `crates/agent_ui/src/agent_panel.rs::render_toolbar` (active-thread branch around lines 4676–4686)

**Context the implementer needs:**
- The right-aligned button group in the active-thread branch is at agent_panel.rs:4676–4686:
  ```rust
  h_flex()
      .h_full()
      .flex_none()
      .gap_1()
      .pl_1()
      .pr_1()
      .when(can_create_entries, |this| this.child(new_thread_menu))
      .child(full_screen_button)
      .child(self.render_panel_options_menu(window, cx)),
  ```
- The canonical `PopoverMenu` + `ContextMenu` pattern in this codebase is at thread_view.rs:3960–4040 (the `effort-selector`). Mirror that: a `PopoverMenu::new(...)`, a `.trigger_with_tooltip(...)`, and a `.menu(move |window, cx| Some(ContextMenu::build(...)))` closure.
- To reach the active `ThreadView` from inside `AgentPanel`, use `self.active_thread_view(cx)` (agent_panel.rs:3037).
- The icon to use: `IconName::HistoryRerun` (already used elsewhere in the agent UI, e.g. `crates/agent_ui/src/completion_provider.rs:406`).
- `ContextMenu::build` provides a `menu` builder with `.entry(label, handler)` and `.header(...)` methods. See thread_view.rs:3960–4040 for an exact, working example of building entries dynamically from a `Vec`.

- [ ] **Step 1: Write a helper to render the button**

Add a new private method on `AgentPanel` in `crates/agent_ui/src/agent_panel.rs`, located near the other `render_*` helpers (e.g. just above or below `render_panel_options_menu`):

```rust
fn render_prompt_history_menu(
    &self,
    window: &mut Window,
    cx: &mut Context<Self>,
) -> impl IntoElement {
    let active_thread_view = self.active_thread_view(cx);
    let has_thread = active_thread_view.is_some();

    PopoverMenu::new("agent-panel-prompt-history")
        .with_handle(self.prompt_history_menu_handle.clone())
        .trigger_with_tooltip(
            IconButton::new("agent-panel-prompt-history-trigger", IconName::HistoryRerun)
                .icon_size(IconSize::Small)
                .disabled(!has_thread),
            Tooltip::text("Prompt History"),
        )
        .menu(move |window, cx| {
            let thread_view = active_thread_view.clone()?;
            let previews =
                thread_view.update(cx, |tv, cx| tv.user_prompt_previews(cx));
            Some(ContextMenu::build(window, cx, |mut menu, _window, _cx| {
                menu = menu.header("Prompt History");
                if previews.is_empty() {
                    menu = menu.entry("No prompts yet", None, |_, _| {}).disabled();
                    return menu;
                }
                for (entry_ix, preview) in previews {
                    let thread_view = thread_view.clone();
                    menu = menu.entry(preview, None, move |_window, cx| {
                        thread_view.update(cx, |tv, cx| {
                            tv.scroll_to_user_prompt(entry_ix, cx);
                        });
                    });
                }
                menu
            }))
        })
}
```

Notes:
- **Verify the exact `ContextMenu` builder API.** The example at thread_view.rs:3960–4040 (`effort-selector`) is the source of truth for this codebase. If the entry API differs (some versions use `.entry(label, optional_action, handler)` vs `ContextMenuEntry::new(...).handler(...)` pushed via `push_item`), match that pattern exactly. The above is the simple form; adapt if needed.
- **`disabled()` on a menu entry:** if `ContextMenu` doesn't expose `disabled()` on a built entry, fall back to using `ContextMenuEntry::new("No prompts yet").disabled(true)` pushed via `menu.push_item(...)`. Check the local API.

- [ ] **Step 2: Wire the button into the toolbar**

In `render_toolbar` at `crates/agent_ui/src/agent_panel.rs:4676`, change the right-aligned button group to include the new menu **before** `new_thread_menu` (so "New Thread" stays adjacent to the full-screen toggle, where it is today):

```rust
.child(
    h_flex()
        .h_full()
        .flex_none()
        .gap_1()
        .pl_1()
        .pr_1()
        .child(self.render_prompt_history_menu(window, cx))
        .when(can_create_entries, |this| this.child(new_thread_menu))
        .child(full_screen_button)
        .child(self.render_panel_options_menu(window, cx)),
)
```

(Verify there is no analogous change needed in the *empty-thread* branch around lines 4572–4638. The history button is meaningless without an active thread, so leaving that branch untouched is correct.)

- [ ] **Step 3: Build**

```
cargo build -p agent_ui
```

Expected: clean build. If `IconName::HistoryRerun`, `PopoverMenu`, `ContextMenu`, or `Tooltip` aren't already imported in `agent_panel.rs`, add them to the existing `use ui::{...}` import.

- [ ] **Step 4: Run clippy**

```
./script/clippy -p agent_ui
```

Expected: no new warnings.

- [ ] **Step 5: Run the existing agent_ui test suite to confirm no regressions**

```
cargo test -p agent_ui
```

Expected: all tests pass.

- [ ] **Step 6: Stop and report**

Tell the user Task 3 is done. Do not commit.

---

## Task 4: Manual UI verification

**Files:** None (interactive verification only).

This task is **mandatory** because the bulk of the change is UI behavior that the unit test in Task 1 cannot cover (popover open/close, click targets, scroll behavior in a real viewport).

- [ ] **Step 1: Launch Zed locally**

```
cargo run --bin zed
```

- [ ] **Step 2: Verify the empty-state UX**

- Open the agent panel.
- Start a new thread but submit **no** prompts.
- Confirm: the new history icon (clock/replay icon) is visible in the toolbar but **disabled**, OR — if you adopted the alternative where the button is always enabled — clicking it shows a single disabled "No prompts yet" entry.

- [ ] **Step 3: Verify the populated-state UX**

- Send 3+ prompts to the agent (the agent responses don't matter; you can interrupt them).
- Include one prompt with multiple lines and one prompt longer than 80 characters.
- Click the history icon. Confirm:
  - The dropdown lists each prompt in submission order (oldest first).
  - The multi-line prompt shows only its first line.
  - The long prompt is truncated with `…`.
  - The dropdown is anchored to / under the button, like the other toolbar dropdowns.

- [ ] **Step 4: Verify the scroll behavior**

- Scroll the chat to the bottom.
- Click the history icon and select the first (oldest) prompt.
- Confirm: the chat scrolls so the selected prompt is anchored at the top of the viewport.
- Repeat with a middle prompt and the last prompt.

- [ ] **Step 5: Verify the lazy-load disclaimer**

- If the user has reloaded an older thread (where earlier messages are not yet loaded), confirm the dropdown only lists prompts that are currently loaded in the entries slice. This matches the design intent (only this-session prompts).

- [ ] **Step 6: Report the verification result to the user**

Summarize what worked, what didn't, and any visual nits worth following up on. Do **not** commit. The user will decide whether to commit.

---

## Self-Review Notes (from plan author)

- **Spec coverage:** Button in title bar ✅ (Task 3, right-aligned button group), dropdown of user prompts ✅ (Task 3 menu builder), click-to-scroll ✅ (Task 1 + Task 3 handler), lazy-load OK because we iterate `entries()` only ✅ (covered in Task 4 step 5).
- **Type consistency:** `user_prompt_previews` returns `Vec<(usize, SharedString)>` and `scroll_to_user_prompt` consumes `usize` — both used consistently in Tasks 1 and 3. The handle field is `PopoverMenuHandle<ContextMenu>` everywhere it appears.
- **Placeholder scan:** No "TBD"/"handle edge cases"/"similar to". The one place where the implementer must adapt to the local API (`ContextMenu` entry builder) is flagged explicitly with the canonical reference example (thread_view.rs:3960–4040) and a fallback recipe.
- **Risk:** The exact `ContextMenu` builder method names (`entry`, `header`, `push_item`, `disabled`) may differ slightly between Zed versions. The plan tells the implementer to mirror `effort-selector` if signatures differ.
