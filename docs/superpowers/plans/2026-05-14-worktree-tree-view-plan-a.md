# Worktree Tree View (Plan A) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Insert a per-worktree subheader layer between project headers and threads in the agents sidebar (`crates/sidebar`), each subheader showing the worktree name with a branch chip colored by git state, supporting collapse, inline rename (persisted + auto-cleaned), and empty groups.

**Architecture:** Add a `ListEntry::WorktreeHeader { project_group_key, worktree_path, ... }` variant. In `rebuild_contents`, after each expanded `ProjectHeader`, iterate over the project group's main worktree paths and emit one `WorktreeHeader` per path followed by the threads whose **primary main worktree path** (first path in `thread.main_worktree_paths()` that is also a path in `group_key.path_list()`) equals that path. Custom names + per-worktree collapse state live in a new SQLite table (`worktree_group_overrides`) on `ThreadMetadataDb`, with cleanup of any rows whose path is no longer referenced by an open project group.

**Out of scope (deferred to Plan B):** Drag-and-drop reordering of project headers and worktree subheaders.

**Tech Stack:** Rust, GPUI, sqlez (SQLite), Zed UI primitives (`Disclosure`, `Editor::single_line`, `ContextMenu`).

---

## File Structure

**Modify:**

- `crates/agent_ui/src/thread_metadata_store.rs`
  - New struct `WorktreeGroupOverride { custom_name: Option<SharedString>, collapsed: bool }`
  - New migration creating `worktree_group_overrides` table
  - New cached field `worktree_overrides: HashMap<PathBuf, WorktreeGroupOverride>`
  - New methods: `worktree_override`, `set_worktree_custom_name`, `clear_worktree_custom_name`, `set_worktree_collapsed`, `cleanup_worktree_overrides_not_in`
  - New `ThreadMetadataDb` async methods: `load_worktree_overrides`, `upsert_worktree_override`, `delete_worktree_override`, `delete_worktree_overrides_not_in`

- `crates/sidebar/src/sidebar.rs`
  - New `ListEntry::WorktreeHeader` variant
  - New helper module (inline): `WorktreeGitStatus` enum + `classify_worktree_git_status` function
  - New `worktree_branch_status_by_path: HashMap<PathBuf, WorktreeGitStatus>` built alongside `branch_by_path` in `rebuild_contents`
  - Modified `rebuild_contents`: emits `WorktreeHeader` entries; groups threads by primary main path
  - New `render_worktree_header`, `render_worktree_header_context_menu`, `render_worktree_branch_chip`
  - Modified `render_list_entry` dispatch
  - New state fields: `worktree_rename_editor: Option<WorktreeRenameEditor>`, `worktree_header_menu_handles: HashMap<PathBuf, PopoverMenuHandle<ContextMenu>>`
  - New actions: `RenameWorktreeGroup`, `ClearWorktreeGroupName`, `ToggleWorktreeGroup`
  - Orphan cleanup call at end of `rebuild_contents`

**Modify (tests):**

- `crates/sidebar/src/sidebar_tests.rs`
  - Add factory helpers and new tests covering grouping, branch chip, collapse, rename, cleanup.

No new files. Following existing pattern: helpers live inside `sidebar.rs` alongside `render_project_header`.

---

## Conventions used throughout

- The sidebar **already** has collapsibility on project headers via `MultiWorkspace::group_state_by_key`. We use a **separate** SQLite-backed mechanism for worktree subheaders so the state persists even after the user closes the project group (so reopening the project restores their preference).
- Run clippy via `./script/clippy` (project rule from `CLAUDE.md`), not `cargo clippy`.
- Use `cx.background_executor().timer(...)` in tests, not `smol::Timer::after(...)`.
- Never use `.unwrap()` outside tests — use `?`, `let ... else`, or pattern-match.
- Never silently discard fallible results with `let _ =`.
- All commits in this plan use a `Co-Authored-By` trailer for Claude when committing in agentic mode (per repo convention; the user toggles this).

---

## Task 1: Add `worktree_group_overrides` SQLite table + migration

**Files:**
- Modify: `crates/agent_ui/src/thread_metadata_store.rs:1308-1400` (extend `MIGRATIONS` array)

- [ ] **Step 1: Add migration to the `MIGRATIONS` array**

Open `crates/agent_ui/src/thread_metadata_store.rs`, find `impl Domain for ThreadMetadataDb` (around line 1305), and append a new migration string at the end of `MIGRATIONS`:

```rust
sql!(
    CREATE TABLE IF NOT EXISTS worktree_group_overrides(
        worktree_path TEXT PRIMARY KEY,
        custom_name TEXT,
        collapsed INTEGER NOT NULL DEFAULT 0
    ) STRICT;
),
```

- [ ] **Step 2: Verify schema compiles**

Run: `./script/clippy -p agent_ui`
Expected: builds without errors.

- [ ] **Step 3: Commit**

```bash
git add crates/agent_ui/src/thread_metadata_store.rs
git commit -m "agent_ui: Add worktree_group_overrides table for per-worktree sidebar state"
```

---

## Task 2: Add `WorktreeGroupOverride` struct + DB read/write methods

**Files:**
- Modify: `crates/agent_ui/src/thread_metadata_store.rs` (struct around line 440; impl `ThreadMetadataDb` around line 1405)

- [ ] **Step 1: Add the struct definition**

Insert near the other public structs (after `ArchivedGitWorktree` around line 481):

```rust
/// Per-worktree sidebar UI overrides: custom display name and collapse state.
/// Keyed by absolute worktree path. Cleaned up when no open project group
/// references the path.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorktreeGroupOverride {
    pub custom_name: Option<SharedString>,
    pub collapsed: bool,
}
```

- [ ] **Step 2: Add row type used internally for SQLite reads**

Right after the struct:

```rust
struct WorktreeOverrideRow {
    path: PathBuf,
    custom_name: Option<String>,
    collapsed: bool,
}

impl Column for WorktreeOverrideRow {
    fn column(statement: &mut Statement, start_index: i32) -> anyhow::Result<(Self, i32)> {
        let (path_str, i): (String, i32) = Column::column(statement, start_index)?;
        let (custom_name, i): (Option<String>, i32) = Column::column(statement, i)?;
        let (collapsed_int, i): (i64, i32) = Column::column(statement, i)?;
        Ok((
            WorktreeOverrideRow {
                path: PathBuf::from(path_str),
                custom_name,
                collapsed: collapsed_int != 0,
            },
            i,
        ))
    }
}
```

- [ ] **Step 3: Add async DB methods on `ThreadMetadataDb`**

Find the `impl ThreadMetadataDb` block (around line 1405) and append these methods:

```rust
pub async fn load_worktree_overrides(&self) -> anyhow::Result<Vec<WorktreeOverrideRow>> {
    self.select::<WorktreeOverrideRow>(
        "SELECT worktree_path, custom_name, collapsed FROM worktree_group_overrides",
    )?()
}

pub async fn upsert_worktree_override(
    &self,
    path: PathBuf,
    custom_name: Option<String>,
    collapsed: bool,
) -> anyhow::Result<()> {
    self.write(move |conn| {
        let path_str = path.to_string_lossy().to_string();
        let mut stmt = Statement::prepare(
            conn,
            "INSERT INTO worktree_group_overrides(worktree_path, custom_name, collapsed) \
             VALUES (?1, ?2, ?3) \
             ON CONFLICT(worktree_path) DO UPDATE SET \
                custom_name = excluded.custom_name, \
                collapsed = excluded.collapsed",
        )?;
        let i = stmt.bind(&path_str, 1)?;
        let i = stmt.bind(&custom_name, i)?;
        stmt.bind(&(collapsed as i64), i)?;
        stmt.exec()
    })
    .await
}

pub async fn delete_worktree_override(&self, path: PathBuf) -> anyhow::Result<()> {
    self.write(move |conn| {
        let path_str = path.to_string_lossy().to_string();
        let mut stmt = Statement::prepare(
            conn,
            "DELETE FROM worktree_group_overrides WHERE worktree_path = ?1",
        )?;
        stmt.bind(&path_str, 1)?;
        stmt.exec()
    })
    .await
}

pub async fn delete_worktree_overrides_not_in(
    &self,
    keep_paths: Vec<PathBuf>,
) -> anyhow::Result<()> {
    self.write(move |conn| {
        if keep_paths.is_empty() {
            let mut stmt = Statement::prepare(conn, "DELETE FROM worktree_group_overrides")?;
            return stmt.exec();
        }
        let placeholders = std::iter::repeat("?")
            .take(keep_paths.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "DELETE FROM worktree_group_overrides WHERE worktree_path NOT IN ({placeholders})"
        );
        let mut stmt = Statement::prepare(conn, &sql)?;
        let mut i = 1;
        for path in &keep_paths {
            let path_str = path.to_string_lossy().to_string();
            i = stmt.bind(&path_str, i)?;
        }
        stmt.exec()
    })
    .await
}
```

- [ ] **Step 4: Build to verify**

Run: `./script/clippy -p agent_ui`
Expected: builds without errors.

- [ ] **Step 5: Commit**

```bash
git add crates/agent_ui/src/thread_metadata_store.rs
git commit -m "agent_ui: Add WorktreeGroupOverride DB read/write methods"
```

---

## Task 3: Cache `worktree_overrides` in `ThreadMetadataStore` and load on startup

**Files:**
- Modify: `crates/agent_ui/src/thread_metadata_store.rs:486-497` (struct fields) + the `new` function and the initial load logic

- [ ] **Step 1: Add cached field to `ThreadMetadataStore`**

Find the struct definition (line ~486):

```rust
pub struct ThreadMetadataStore {
    db: ThreadMetadataDb,
    threads: HashMap<ThreadId, ThreadMetadata>,
    ...
}
```

Add a new field at the end (just before `_db_operations_task`):

```rust
    worktree_overrides: HashMap<PathBuf, WorktreeGroupOverride>,
```

- [ ] **Step 2: Find the `new` function and initialize the field**

Search for `fn new(db: ThreadMetadataDb` in the file. In the returned struct literal, add the line:

```rust
    worktree_overrides: HashMap::default(),
```

- [ ] **Step 3: Make `ThreadMetadataDb` clonable**

In `crates/agent_ui/src/thread_metadata_store.rs:1303`, change:

```rust
struct ThreadMetadataDb(ThreadSafeConnection);
```

to:

```rust
#[derive(Clone)]
struct ThreadMetadataDb(ThreadSafeConnection);
```

`ThreadSafeConnection` is already `Clone`, so this compiles.

- [ ] **Step 4: Add an async initial load on construction**

Inside `fn new`, after the existing reload setup, append a spawn that populates `worktree_overrides`:

```rust
{
    let db = self.db.clone();
    cx.spawn(async move |this, cx| {
        match db.load_worktree_overrides().await {
            Ok(rows) => {
                this.update(cx, |this, cx| {
                    for row in rows {
                        this.worktree_overrides.insert(
                            row.path,
                            WorktreeGroupOverride {
                                custom_name: row.custom_name.map(SharedString::from),
                                collapsed: row.collapsed,
                            },
                        );
                    }
                    cx.notify();
                })
                .log_err();
            }
            Err(err) => log::error!("failed to load worktree overrides: {err:?}"),
        }
    })
    .detach();
}
```

Note: this code lives inside `Self::new`, where `self` is being constructed via a struct literal — adjust to schedule the load AFTER the struct is returned. The actual pattern used elsewhere in `ThreadMetadataStore::new` is to spawn from the constructor's `cx` and dispatch back via the entity handle. Search for an existing `cx.spawn(async move |this, cx|` block in `fn new` and place the load alongside it.

- [ ] **Step 5: Build**

Run: `./script/clippy -p agent_ui`
Expected: builds without errors.

- [ ] **Step 6: Commit**

```bash
git add crates/agent_ui/src/thread_metadata_store.rs
git commit -m "agent_ui: Load worktree overrides into ThreadMetadataStore on startup"
```

---

## Task 4: Add `ThreadMetadataStore` mutator methods for overrides

**Files:**
- Modify: `crates/agent_ui/src/thread_metadata_store.rs` (add methods in the `impl ThreadMetadataStore` block, near `set_title_override` around line 686)

- [ ] **Step 1: Write the failing test**

Open or create `crates/agent_ui/src/thread_metadata_store.rs` test module at the very bottom of the file:

```rust
#[cfg(test)]
mod worktree_override_tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    async fn test_set_and_clear_worktree_custom_name(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            ThreadMetadataStore::init_global(cx);
        });
        let path = PathBuf::from("/tmp/proj");

        cx.update(|cx| {
            ThreadMetadataStore::global(cx).update(cx, |store, cx| {
                store.set_worktree_custom_name(path.clone(), SharedString::from("my name"), cx);
            });
        });

        cx.update(|cx| {
            let store = ThreadMetadataStore::global(cx).read(cx);
            assert_eq!(
                store.worktree_override(&path).and_then(|o| o.custom_name.clone()),
                Some(SharedString::from("my name"))
            );
        });

        cx.update(|cx| {
            ThreadMetadataStore::global(cx).update(cx, |store, cx| {
                store.clear_worktree_custom_name(&path, cx);
            });
        });

        cx.update(|cx| {
            let store = ThreadMetadataStore::global(cx).read(cx);
            // After clearing the name, if collapsed was false (default), the row is fully gone.
            assert!(store.worktree_override(&path).is_none());
        });
    }
}
```

- [ ] **Step 2: Run test, confirm it fails**

Run: `cargo test -p agent_ui worktree_override_tests::test_set_and_clear_worktree_custom_name`
Expected: FAIL — `set_worktree_custom_name` / `clear_worktree_custom_name` / `worktree_override` do not exist.

- [ ] **Step 3: Implement the methods**

In `impl ThreadMetadataStore` (search for `pub fn set_title_override` around line 686, add the new methods near it):

```rust
pub fn worktree_override(&self, path: &Path) -> Option<&WorktreeGroupOverride> {
    self.worktree_overrides.get(path)
}

pub fn worktree_custom_name(&self, path: &Path) -> Option<SharedString> {
    self.worktree_overrides
        .get(path)
        .and_then(|o| o.custom_name.clone())
}

pub fn is_worktree_collapsed(&self, path: &Path) -> bool {
    self.worktree_overrides
        .get(path)
        .map(|o| o.collapsed)
        .unwrap_or(false)
}

pub fn set_worktree_custom_name(
    &mut self,
    path: PathBuf,
    name: SharedString,
    cx: &mut Context<Self>,
) {
    let name_trimmed = SharedString::from(name.trim().to_string());
    if name_trimmed.is_empty() {
        self.clear_worktree_custom_name(&path, cx);
        return;
    }
    let entry = self.worktree_overrides.entry(path.clone()).or_default();
    if entry.custom_name.as_ref() == Some(&name_trimmed) {
        return;
    }
    entry.custom_name = Some(name_trimmed.clone());
    let collapsed = entry.collapsed;
    self.persist_worktree_override(path, Some(name_trimmed.to_string()), collapsed, cx);
    cx.notify();
}

pub fn clear_worktree_custom_name(&mut self, path: &Path, cx: &mut Context<Self>) {
    let Some(entry) = self.worktree_overrides.get_mut(path) else {
        return;
    };
    if entry.custom_name.is_none() {
        return;
    }
    entry.custom_name = None;
    let collapsed = entry.collapsed;
    if !collapsed {
        self.worktree_overrides.remove(path);
        let path_owned = path.to_path_buf();
        let db = self.db.clone();
        cx.background_spawn(async move {
            db.delete_worktree_override(path_owned).await.log_err();
        })
        .detach();
    } else {
        self.persist_worktree_override(path.to_path_buf(), None, true, cx);
    }
    cx.notify();
}

pub fn set_worktree_collapsed(
    &mut self,
    path: PathBuf,
    collapsed: bool,
    cx: &mut Context<Self>,
) {
    let entry = self.worktree_overrides.entry(path.clone()).or_default();
    if entry.collapsed == collapsed {
        return;
    }
    entry.collapsed = collapsed;
    let custom_name = entry.custom_name.clone();
    if !collapsed && custom_name.is_none() {
        self.worktree_overrides.remove(&path);
        let db = self.db.clone();
        cx.background_spawn(async move {
            db.delete_worktree_override(path).await.log_err();
        })
        .detach();
    } else {
        self.persist_worktree_override(
            path,
            custom_name.map(|s| s.to_string()),
            collapsed,
            cx,
        );
    }
    cx.notify();
}

pub fn cleanup_worktree_overrides_not_in(
    &mut self,
    keep_paths: &HashSet<PathBuf>,
    cx: &mut Context<Self>,
) {
    let to_remove: Vec<PathBuf> = self
        .worktree_overrides
        .keys()
        .filter(|p| !keep_paths.contains(*p))
        .cloned()
        .collect();
    if to_remove.is_empty() {
        return;
    }
    for path in &to_remove {
        self.worktree_overrides.remove(path);
    }
    let keep: Vec<PathBuf> = keep_paths.iter().cloned().collect();
    let db = self.db.clone();
    cx.background_spawn(async move {
        db.delete_worktree_overrides_not_in(keep).await.log_err();
    })
    .detach();
    cx.notify();
}

fn persist_worktree_override(
    &self,
    path: PathBuf,
    custom_name: Option<String>,
    collapsed: bool,
    cx: &mut Context<Self>,
) {
    let db = self.db.clone();
    cx.background_spawn(async move {
        db.upsert_worktree_override(path, custom_name, collapsed)
            .await
            .log_err();
    })
    .detach();
}
```

- [ ] **Step 4: Run test, confirm it passes**

Run: `cargo test -p agent_ui worktree_override_tests::test_set_and_clear_worktree_custom_name`
Expected: PASS.

- [ ] **Step 5: Add a second test for collapse + cleanup**

In the same `mod worktree_override_tests`, add:

```rust
#[gpui::test]
async fn test_cleanup_removes_unreferenced(cx: &mut TestAppContext) {
    cx.update(|cx| {
        let settings_store = settings::SettingsStore::test(cx);
        cx.set_global(settings_store);
        ThreadMetadataStore::init_global(cx);
    });
    let keep = PathBuf::from("/tmp/keep");
    let drop = PathBuf::from("/tmp/drop");

    cx.update(|cx| {
        ThreadMetadataStore::global(cx).update(cx, |store, cx| {
            store.set_worktree_custom_name(keep.clone(), SharedString::from("k"), cx);
            store.set_worktree_custom_name(drop.clone(), SharedString::from("d"), cx);
        });
    });

    cx.update(|cx| {
        ThreadMetadataStore::global(cx).update(cx, |store, cx| {
            let mut keep_set = HashSet::default();
            keep_set.insert(keep.clone());
            store.cleanup_worktree_overrides_not_in(&keep_set, cx);
        });
    });

    cx.update(|cx| {
        let store = ThreadMetadataStore::global(cx).read(cx);
        assert!(store.worktree_override(&keep).is_some());
        assert!(store.worktree_override(&drop).is_none());
    });
}
```

Run: `cargo test -p agent_ui worktree_override_tests`
Expected: both tests PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/agent_ui/src/thread_metadata_store.rs
git commit -m "agent_ui: Add ThreadMetadataStore methods for worktree overrides"
```

---

## Task 5: Add `WorktreeGitStatus` enum + classifier in sidebar

**Files:**
- Modify: `crates/sidebar/src/sidebar.rs` (add module-level helper near the other free functions around line 410)

- [ ] **Step 1: Add the enum and classifier**

Insert after `root_repository_snapshots` (around line 410 in `crates/sidebar/src/sidebar.rs`):

```rust
/// Aggregate git state used to color the branch chip on a worktree header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WorktreeGitStatus {
    Clean,
    UntrackedOnly,
    Modified,
    Conflicted,
}

impl WorktreeGitStatus {
    fn chip_color(self, cx: &App) -> gpui::Hsla {
        let colors = cx.theme().colors();
        match self {
            // Muted by default — clean is the visual baseline.
            WorktreeGitStatus::Clean => colors.text_muted,
            WorktreeGitStatus::UntrackedOnly => colors.editor_document_highlight_read_background, // blue-ish
            WorktreeGitStatus::Modified => colors.warning_background,
            WorktreeGitStatus::Conflicted => colors.error_background,
        }
    }
}

fn classify_worktree_git_status(
    snapshot: &project::git_store::RepositorySnapshot,
) -> WorktreeGitStatus {
    if !snapshot.merge.merge_heads_by_conflicted_path.is_empty() {
        return WorktreeGitStatus::Conflicted;
    }
    let mut has_untracked = false;
    let mut has_other_changes = false;
    for entry in snapshot.status_entries() {
        match entry.status {
            project::git_store::FileStatus::Untracked => has_untracked = true,
            project::git_store::FileStatus::Ignored => {}
            _ => {
                has_other_changes = true;
                break;
            }
        }
    }
    if has_other_changes {
        WorktreeGitStatus::Modified
    } else if has_untracked {
        WorktreeGitStatus::UntrackedOnly
    } else {
        WorktreeGitStatus::Clean
    }
}
```

If `RepositorySnapshot::status_entries()` is a method that doesn't exist as named, search for the iterator method: in `crates/project/src/git_store.rs` the snapshot exposes `statuses_by_path` (a TreeMap) or similar. Match the actual API. The intent: iterate over `(path, FileStatus)` pairs.

If the actual API differs, the classifier still has the same shape — only the iteration is different. Concretely, in the current code these calls work:

- `snapshot.merge.merge_heads_by_conflicted_path` — a `TreeMap<RepoPath, ...>`; `.is_empty()` works.
- `snapshot.statuses_by_path.iter()` may be the actual iteration; if so, the loop becomes `for (_path, status) in snapshot.statuses_by_path.iter() { match status { ... } }`.

Use whichever the type actually exposes; replace the loop above accordingly. Verify by running the build at the end.

- [ ] **Step 2: Build to verify**

Run: `./script/clippy -p sidebar`
Expected: builds without errors. If the `status_entries()` / `statuses_by_path` call fails, adjust to the correct method name as above.

- [ ] **Step 3: Commit**

```bash
git add crates/sidebar/src/sidebar.rs
git commit -m "sidebar: Add WorktreeGitStatus classifier for worktree branch chip"
```

---

## Task 6: Build `git_status_by_path` map alongside `branch_by_path` in `rebuild_contents`

**Files:**
- Modify: `crates/sidebar/src/sidebar.rs:1173-1193` (the existing `branch_by_path` loop)

- [ ] **Step 1: Extend the loop**

Find the existing block (lines ~1173–1193). It currently looks like:

```rust
let mut branch_by_path: HashMap<PathBuf, SharedString> = HashMap::new();
for ws in &workspaces {
    let project = ws.read(cx).project().read(cx);
    for repo in project.repositories(cx).values() {
        let snapshot = repo.read(cx).snapshot();
        if let Some(branch) = &snapshot.branch {
            branch_by_path.insert(
                snapshot.work_directory_abs_path.to_path_buf(),
                SharedString::from(Arc::<str>::from(branch.name())),
            );
        }
        for linked_wt in snapshot.linked_worktrees() {
            if let Some(branch) = linked_wt.branch_name() {
                branch_by_path.insert(
                    linked_wt.path.clone(),
                    SharedString::from(Arc::<str>::from(branch)),
                );
            }
        }
    }
}
```

Replace it with:

```rust
let mut branch_by_path: HashMap<PathBuf, SharedString> = HashMap::new();
let mut git_status_by_path: HashMap<PathBuf, WorktreeGitStatus> = HashMap::new();
for ws in &workspaces {
    let project = ws.read(cx).project().read(cx);
    for repo in project.repositories(cx).values() {
        let snapshot = repo.read(cx).snapshot();
        let status = classify_worktree_git_status(&snapshot);
        let main_path = snapshot.work_directory_abs_path.to_path_buf();
        if let Some(branch) = &snapshot.branch {
            branch_by_path.insert(
                main_path.clone(),
                SharedString::from(Arc::<str>::from(branch.name())),
            );
        }
        git_status_by_path.insert(main_path, status);
        for linked_wt in snapshot.linked_worktrees() {
            if let Some(branch) = linked_wt.branch_name() {
                branch_by_path.insert(
                    linked_wt.path.clone(),
                    SharedString::from(Arc::<str>::from(branch)),
                );
            }
            // Linked worktree status: linked worktrees in Zed today reuse the
            // main repo's status surface, so we use the same value. If a
            // separate per-linked-worktree status API is added later, swap this.
            git_status_by_path.insert(linked_wt.path.clone(), status);
        }
    }
}
```

- [ ] **Step 2: Build**

Run: `./script/clippy -p sidebar`
Expected: builds without errors.

- [ ] **Step 3: Commit**

```bash
git add crates/sidebar/src/sidebar.rs
git commit -m "sidebar: Build per-worktree git status map during rebuild"
```

---

## Task 7: Add `ListEntry::WorktreeHeader` variant

**Files:**
- Modify: `crates/sidebar/src/sidebar.rs:265-278` (the `ListEntry` enum)

- [ ] **Step 1: Extend the enum**

Find the enum (line ~265):

```rust
#[derive(Clone)]
enum ListEntry {
    ProjectHeader { ... },
    Thread(ThreadEntry),
    Terminal(TerminalEntry),
}
```

Add the new variant:

```rust
#[derive(Clone)]
enum ListEntry {
    ProjectHeader { ... existing fields ... },
    WorktreeHeader {
        project_group_key: ProjectGroupKey,
        worktree_path: PathBuf,
        display_name: SharedString,
        branch_name: Option<SharedString>,
        git_status: WorktreeGitStatus,
        has_custom_name: bool,
        is_collapsed: bool,
        thread_count: usize,
    },
    Thread(ThreadEntry),
    Terminal(TerminalEntry),
}
```

- [ ] **Step 2: Update any exhaustive `match` on `ListEntry`**

Search the file: `Grep -n "match.*entry" crates/sidebar/src/sidebar.rs`. Update each match arm that lists `ProjectHeader`, `Thread`, `Terminal` to also handle `WorktreeHeader`. Concretely, expect to add `ListEntry::WorktreeHeader { .. } => { ... }` arms at:

- `display_time` helper around line 3982 — `unreachable!()` is acceptable for WorktreeHeader since it isn't sorted by time:

  ```rust
  fn display_time(entry: &ListEntry) -> DateTime<Utc> {
      match entry {
          ListEntry::Thread(thread) => Sidebar::thread_display_time(&thread.metadata),
          ListEntry::Terminal(terminal) => terminal.created_at,
          ListEntry::ProjectHeader { .. } | ListEntry::WorktreeHeader { .. } => unreachable!(),
      }
  }
  ```

- `mru_entries_for_switcher` around line 4034 — return `None` for `WorktreeHeader`:

  ```rust
  ListEntry::WorktreeHeader { .. } => None,
  ```

- `is_active_session` in `sidebar_tests.rs:55` — leave as `_ => None` (already a catch-all).

- `assert_active_thread` in `sidebar_tests.rs:37` — matches only `Thread`, leave alone.

- Any `render_list_entry` dispatch — handled in Task 9.

- [ ] **Step 3: Build**

Run: `./script/clippy -p sidebar`
Expected: builds. If any other exhaustive matches surface, add the `WorktreeHeader` arm with a sensible default (skip / no-op / continue).

- [ ] **Step 4: Commit**

```bash
git add crates/sidebar/src/sidebar.rs
git commit -m "sidebar: Add WorktreeHeader variant to ListEntry"
```

---

## Task 8: Update `rebuild_contents` to emit worktree headers and group threads

**Files:**
- Modify: `crates/sidebar/src/sidebar.rs:1195-1581` (the `for group in &groups` loop in `rebuild_contents`)

- [ ] **Step 1: Locate the existing structure**

Open `rebuild_contents` (line ~1103). The relevant section emits `ListEntry::ProjectHeader` (line ~1539-1567) and then calls `Self::push_entries_by_display_time` (line ~1573-1579). We will insert worktree-header logic between these two.

- [ ] **Step 2: Add a helper that picks a thread's primary worktree path within a group**

Above `rebuild_contents` (near `root_repository_snapshots`), add a free function:

```rust
/// The worktree subheader a thread belongs under within a project group.
/// Returns the first path in `thread_main_paths` that also appears in
/// `group_paths`. Falls back to `group_paths[0]` if no overlap (defensive;
/// shouldn't happen because the thread is in this group precisely because
/// of overlap).
fn primary_worktree_path_for_thread(
    thread_main_paths: &PathList,
    group_paths: &PathList,
) -> Option<PathBuf> {
    let group_paths_vec: &[Arc<Path>] = group_paths.paths();
    for tp in thread_main_paths.paths() {
        if group_paths_vec.iter().any(|gp| gp.as_ref() == tp.as_ref()) {
            return Some(tp.to_path_buf());
        }
    }
    group_paths_vec.first().map(|p| p.to_path_buf())
}
```

If `PathList::paths()` returns a different concrete type (e.g. `&[PathBuf]` or `&[Arc<Path>]`), match the actual signature. The intent is to compare paths by absolute path equality.

- [ ] **Step 3: Replace the section that pushes threads under a project header**

Find the code (around line ~1567):

```rust
if is_collapsed {
    continue;
}

Self::push_entries_by_display_time(
    &mut entries,
    terminals,
    threads,
    &mut current_session_ids,
    &mut current_thread_ids,
);
```

Replace with:

```rust
if is_collapsed {
    continue;
}

// Bucket threads by primary worktree path. Terminals retain top-of-group
// placement (they're workspace-scoped, not worktree-scoped).
let group_paths = group_key.path_list();
let mut threads_by_worktree: HashMap<PathBuf, Vec<ThreadEntry>> = HashMap::new();
for thread in threads {
    if let Some(primary) =
        primary_worktree_path_for_thread(thread.metadata.main_worktree_paths(), group_paths)
    {
        threads_by_worktree.entry(primary).or_default().push(thread);
    }
}

// Push terminals at the top of the project group (above worktree subheaders).
for terminal in terminals {
    let terminal_id = terminal.terminal_id;
    current_terminal_ids.insert(terminal_id);
    entries.push(ListEntry::Terminal(terminal));
}

// Emit one worktree subheader per main path in the project group.
let store = ThreadMetadataStore::global(cx).read(cx);
for worktree_path_arc in group_paths.paths() {
    let worktree_path = worktree_path_arc.to_path_buf();
    let bucket = threads_by_worktree
        .remove(&worktree_path)
        .unwrap_or_default();
    let thread_count = bucket.len();

    let override_entry = store.worktree_override(&worktree_path).cloned();
    let custom_name = override_entry
        .as_ref()
        .and_then(|o| o.custom_name.clone());
    let is_wt_collapsed = override_entry.as_ref().map(|o| o.collapsed).unwrap_or(false);

    let derived_name = worktree_path
        .file_name()
        .map(|n| SharedString::from(n.to_string_lossy().to_string()))
        .unwrap_or_else(|| {
            SharedString::from(worktree_path.to_string_lossy().to_string())
        });
    let display_name = custom_name.clone().unwrap_or(derived_name);
    let branch_name = branch_by_path.get(&worktree_path).cloned();
    let git_status = git_status_by_path
        .get(&worktree_path)
        .copied()
        .unwrap_or(WorktreeGitStatus::Clean);

    entries.push(ListEntry::WorktreeHeader {
        project_group_key: group_key.clone(),
        worktree_path: worktree_path.clone(),
        display_name,
        branch_name,
        git_status,
        has_custom_name: custom_name.is_some(),
        is_collapsed: is_wt_collapsed,
        thread_count,
    });

    if is_wt_collapsed {
        continue;
    }

    // Sort threads in this worktree by display time (matches the old behavior
    // within a project group).
    let mut bucket = bucket;
    bucket.sort_by_key(|t| std::cmp::Reverse(Self::thread_display_time(&t.metadata)));
    for thread in bucket {
        if let Some(session_id) = &thread.metadata.session_id {
            current_session_ids.insert(session_id.clone());
        }
        current_thread_ids.insert(thread.metadata.thread_id);
        entries.push(ListEntry::Thread(thread));
    }
}

// Any threads whose primary path didn't match a group path (defensive):
// stash under the first worktree subheader if it exists. Should not happen
// in practice; if it does, log once and drop them to avoid duplication.
if !threads_by_worktree.is_empty() {
    log::warn!(
        "agents sidebar: {} threads in group {:?} did not match any worktree path",
        threads_by_worktree.values().map(Vec::len).sum::<usize>(),
        group_key
    );
}
```

- [ ] **Step 4: Add `WorktreeGitStatus` and `ThreadMetadataStore` imports if missing**

Top of `crates/sidebar/src/sidebar.rs`, ensure the import list includes:

```rust
use agent_ui::thread_metadata_store::{
    ThreadMetadata, ThreadMetadataStore, WorktreeGroupOverride, WorktreePaths,
    worktree_info_from_thread_paths,
};
```

(Add `WorktreeGroupOverride` — the existing import line was at line 7-9.)

- [ ] **Step 5: Build**

Run: `./script/clippy -p sidebar`
Expected: builds.

- [ ] **Step 6: Commit**

```bash
git add crates/sidebar/src/sidebar.rs
git commit -m "sidebar: Bucket threads under worktree subheaders in rebuild_contents"
```

---

## Task 9: Update `render_list_entry` to dispatch `WorktreeHeader`

**Files:**
- Modify: `crates/sidebar/src/sidebar.rs:1675-1748` (the `render_list_entry` function)

- [ ] **Step 1: Add the dispatch arm**

Find the `match entry { ... }` inside `render_list_entry` (line ~1715-1740). It currently looks like:

```rust
let rendered = match entry {
    ListEntry::ProjectHeader { ... } => self.render_project_header(...),
    ListEntry::Thread(thread) => self.render_thread(ix, thread, ...),
    ListEntry::Terminal(terminal) => self.render_terminal(ix, terminal, ...),
};
```

Add a `WorktreeHeader` arm BEFORE `Thread`:

```rust
ListEntry::WorktreeHeader {
    project_group_key,
    worktree_path,
    display_name,
    branch_name,
    git_status,
    has_custom_name,
    is_collapsed,
    thread_count,
} => self.render_worktree_header(
    ix,
    project_group_key,
    worktree_path,
    display_name,
    branch_name.as_ref(),
    *git_status,
    *has_custom_name,
    *is_collapsed,
    *thread_count,
    window,
    cx,
),
```

(Stub `render_worktree_header` is implemented in Task 10.)

- [ ] **Step 2: Add a placeholder `render_worktree_header` so the build passes**

Insert near `render_project_header` (around line 1775):

```rust
#[allow(clippy::too_many_arguments)]
fn render_worktree_header(
    &mut self,
    _ix: usize,
    _project_group_key: &ProjectGroupKey,
    worktree_path: &Path,
    display_name: &SharedString,
    branch_name: Option<&SharedString>,
    _git_status: WorktreeGitStatus,
    _has_custom_name: bool,
    _is_collapsed: bool,
    _thread_count: usize,
    _window: &mut Window,
    _cx: &mut Context<Self>,
) -> AnyElement {
    let _ = worktree_path;
    let _ = branch_name;
    div()
        .px_2()
        .child(Label::new(display_name.clone()).size(LabelSize::Small))
        .into_any_element()
}
```

- [ ] **Step 3: Build and visually verify (manual)**

Run: `./script/clippy -p sidebar`
Expected: builds. Then `cargo run --bin zed` and open a project — confirm worktree subheader rows appear under each project header showing the worktree name. They will look unstyled; that's expected for this task.

- [ ] **Step 4: Commit**

```bash
git add crates/sidebar/src/sidebar.rs
git commit -m "sidebar: Dispatch WorktreeHeader through render_list_entry"
```

---

## Task 10: Implement `render_worktree_header` UI (disclosure + label + branch chip)

**Files:**
- Modify: `crates/sidebar/src/sidebar.rs` (replace the placeholder from Task 9)

- [ ] **Step 1: Implement `render_worktree_branch_chip`**

Add a helper right above `render_worktree_header`:

```rust
fn render_worktree_branch_chip(
    branch_name: Option<&SharedString>,
    git_status: WorktreeGitStatus,
    cx: &App,
) -> Option<AnyElement> {
    let branch = branch_name?.clone();
    let bg = git_status.chip_color(cx);
    Some(
        h_flex()
            .px_1p5()
            .py_0p5()
            .rounded_sm()
            .bg(bg)
            .child(
                Label::new(branch)
                    .size(LabelSize::XSmall)
                    .color(Color::Default),
            )
            .into_any_element(),
    )
}
```

- [ ] **Step 2: Replace the placeholder `render_worktree_header`**

```rust
#[allow(clippy::too_many_arguments)]
fn render_worktree_header(
    &mut self,
    ix: usize,
    project_group_key: &ProjectGroupKey,
    worktree_path: &Path,
    display_name: &SharedString,
    branch_name: Option<&SharedString>,
    git_status: WorktreeGitStatus,
    has_custom_name: bool,
    is_collapsed: bool,
    thread_count: usize,
    window: &mut Window,
    cx: &mut Context<Self>,
) -> AnyElement {
    let path_owned = worktree_path.to_path_buf();
    let id_prefix = SharedString::from(format!(
        "worktree-header-{}",
        worktree_path.to_string_lossy()
    ));
    let editing = matches!(
        &self.worktree_rename_editor,
        Some(e) if e.worktree_path.as_path() == worktree_path
    );

    let disclosure_icon = if is_collapsed {
        IconName::ChevronRight
    } else {
        IconName::ChevronDown
    };

    let disclosure = Disclosure::new(
        ElementId::Name(id_prefix.clone()),
        !is_collapsed,
    )
    .on_toggle_expanded({
        let path = path_owned.clone();
        cx.listener(move |this, _event: &ClickEvent, _window, cx| {
            this.toggle_worktree_collapsed(path.clone(), cx);
        })
    });

    let label_or_editor: AnyElement = if editing {
        if let Some(editor_state) = &self.worktree_rename_editor {
            editor_state.editor.clone().into_any_element()
        } else {
            Label::new(display_name.clone()).into_any_element()
        }
    } else {
        let mut label = Label::new(display_name.clone())
            .size(LabelSize::Small)
            .truncate();
        if has_custom_name {
            label = label.color(Color::Accent);
        }
        label.into_any_element()
    };

    let branch_chip = render_worktree_branch_chip(branch_name, git_status, cx);

    let menu_handle = self
        .worktree_header_menu_handles
        .entry(path_owned.clone())
        .or_default()
        .clone();

    let trailing_menu = PopoverMenu::new(SharedString::from(format!(
        "{id_prefix}-menu"
    )))
    .with_handle(menu_handle.clone())
    .trigger(
        IconButton::new(
            SharedString::from(format!("{id_prefix}-menu-btn")),
            IconName::Ellipsis,
        )
        .icon_size(IconSize::Small)
        .style(ButtonStyle::Transparent),
    )
    .menu({
        let path = path_owned.clone();
        let group_key = project_group_key.clone();
        let this = cx.entity().downgrade();
        move |window, cx| {
            let this = this.upgrade()?;
            Some(this.update(cx, |sidebar, cx| {
                sidebar.build_worktree_header_menu(
                    path.clone(),
                    group_key.clone(),
                    has_custom_name,
                    window,
                    cx,
                )
            }))
        }
    });

    let header_row = h_flex()
        .id(ElementId::Name(id_prefix.clone()))
        .w_full()
        .pl_4() // indent below project header
        .pr_2()
        .py_0p5()
        .gap_1p5()
        .child(disclosure)
        .child(label_or_editor)
        .child(div().flex_1())
        .when_some(branch_chip, |this, chip| this.child(chip))
        .child(trailing_menu)
        .on_secondary_mouse_down({
            let menu_handle = menu_handle.clone();
            cx.listener(move |_, _event: &MouseDownEvent, window, cx| {
                menu_handle.show(window, cx);
            })
        });

    // Indent thread-count badge when collapsed so the user can see the count.
    let header_row = if is_collapsed && thread_count > 0 {
        header_row.child(
            Label::new(SharedString::from(format!("{thread_count}")))
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        )
    } else {
        header_row
    };

    let _ = ix;
    let _ = window;
    header_row.into_any_element()
}
```

- [ ] **Step 3: Add the field `worktree_rename_editor` on the `Sidebar` struct**

Find `pub struct Sidebar { ... }` (line ~575). Define a small struct above it:

```rust
struct WorktreeRenameEditor {
    worktree_path: PathBuf,
    editor: Entity<Editor>,
    _subscription: gpui::Subscription,
}
```

Then add a field to `Sidebar`:

```rust
    worktree_rename_editor: Option<WorktreeRenameEditor>,
    worktree_header_menu_handles: HashMap<PathBuf, PopoverMenuHandle<ContextMenu>>,
```

Initialize in `Sidebar::new` (search for `_subscriptions: Vec::new()` around line 700) — add:

```rust
    worktree_rename_editor: None,
    worktree_header_menu_handles: HashMap::new(),
```

- [ ] **Step 4: Add `toggle_worktree_collapsed` method**

Below `toggle_collapse` (around line 2415):

```rust
fn toggle_worktree_collapsed(&mut self, worktree_path: PathBuf, cx: &mut Context<Self>) {
    let store = ThreadMetadataStore::global(cx);
    let currently_collapsed = store.read(cx).is_worktree_collapsed(&worktree_path);
    store.update(cx, |store, cx| {
        store.set_worktree_collapsed(worktree_path, !currently_collapsed, cx);
    });
    self.update_entries(cx);
}
```

- [ ] **Step 5: Add a stub `build_worktree_header_menu` and stubs for actions**

Below `toggle_worktree_collapsed`:

```rust
fn build_worktree_header_menu(
    &mut self,
    worktree_path: PathBuf,
    _project_group_key: ProjectGroupKey,
    has_custom_name: bool,
    window: &mut Window,
    cx: &mut Context<Self>,
) -> Entity<ContextMenu> {
    ContextMenu::build(window, cx, |menu, _window, _cx| {
        let menu = menu.entry("Rename Group", None, {
            let this = cx.entity().downgrade();
            let path = worktree_path.clone();
            move |window, cx| {
                if let Some(this) = this.upgrade() {
                    this.update(cx, |sidebar, cx| {
                        sidebar.begin_rename_worktree_group(path.clone(), window, cx);
                    });
                }
            }
        });
        if has_custom_name {
            menu.entry("Reset Group Name", None, {
                let this = cx.entity().downgrade();
                let path = worktree_path.clone();
                move |_window, cx| {
                    if let Some(this) = this.upgrade() {
                        this.update(cx, |_sidebar, cx| {
                            let store = ThreadMetadataStore::global(cx);
                            store.update(cx, |store, cx| {
                                store.clear_worktree_custom_name(&path, cx);
                            });
                        });
                    }
                }
            })
        } else {
            menu
        }
    })
}

fn begin_rename_worktree_group(
    &mut self,
    _worktree_path: PathBuf,
    _window: &mut Window,
    _cx: &mut Context<Self>,
) {
    // Implemented in Task 11.
}
```

- [ ] **Step 6: Build and run**

Run: `./script/clippy -p sidebar`
Expected: builds.

Run: `cargo run --bin zed` and visually verify worktree subheaders show the worktree name, branch chip on the right, a disclosure chevron on the left, and clicking the chevron toggles collapse. The "Rename Group" menu entry exists but does nothing yet.

- [ ] **Step 7: Commit**

```bash
git add crates/sidebar/src/sidebar.rs
git commit -m "sidebar: Render worktree headers with branch chip and collapse"
```

---

## Task 11: Implement inline rename of worktree group

**Files:**
- Modify: `crates/sidebar/src/sidebar.rs` — fill in `begin_rename_worktree_group`, handle key events on the editor

- [ ] **Step 1: Add action declarations**

Near the top of the file with the other `gpui::actions!` macro calls (around line 77):

```rust
gpui::actions!(
    agents_sidebar,
    [
        /// Begin editing the display name of the focused worktree group.
        RenameWorktreeGroup,
        /// Commit the in-flight worktree group rename.
        ConfirmWorktreeGroupRename,
        /// Cancel the in-flight worktree group rename.
        CancelWorktreeGroupRename,
    ]
);
```

- [ ] **Step 2: Implement `begin_rename_worktree_group`**

Replace the stub from Task 10:

```rust
fn begin_rename_worktree_group(
    &mut self,
    worktree_path: PathBuf,
    window: &mut Window,
    cx: &mut Context<Self>,
) {
    let store = ThreadMetadataStore::global(cx);
    let initial = store
        .read(cx)
        .worktree_custom_name(&worktree_path)
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            worktree_path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        });

    let editor = cx.new(|cx| {
        let mut editor = Editor::single_line(window, cx);
        editor.set_text(initial, window, cx);
        editor.select_all(&editor::actions::SelectAll, window, cx);
        editor
    });
    let focus_handle = editor.focus_handle(cx);
    window.focus(&focus_handle);

    let subscription =
        cx.subscribe_in(&editor, window, {
            let path = worktree_path.clone();
            move |this, editor, event: &editor::EditorEvent, _window, cx| match event {
                editor::EditorEvent::Blurred => {
                    this.commit_worktree_rename(path.clone(), editor, cx);
                }
                _ => {}
            }
        });

    self.worktree_rename_editor = Some(WorktreeRenameEditor {
        worktree_path,
        editor,
        _subscription: subscription,
    });
    self.update_entries(cx);
}

fn commit_worktree_rename(
    &mut self,
    worktree_path: PathBuf,
    editor: &Entity<Editor>,
    cx: &mut Context<Self>,
) {
    let text = editor.read(cx).text(cx);
    let store = ThreadMetadataStore::global(cx);
    store.update(cx, |store, cx| {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            store.clear_worktree_custom_name(&worktree_path, cx);
        } else {
            store.set_worktree_custom_name(
                worktree_path.clone(),
                SharedString::from(trimmed.to_string()),
                cx,
            );
        }
    });
    self.worktree_rename_editor = None;
    self.update_entries(cx);
}

fn cancel_worktree_rename(&mut self, cx: &mut Context<Self>) {
    self.worktree_rename_editor = None;
    self.update_entries(cx);
}
```

- [ ] **Step 3: Wire Confirm/Cancel keybinds**

Find where the sidebar's render-root element registers `on_action` (search for `.on_action(cx.listener(...` in the file's `Render` impl). Add two new handlers near the others:

```rust
.on_action(cx.listener(|this, _: &ConfirmWorktreeGroupRename, _window, cx| {
    if let Some(state) = this.worktree_rename_editor.take() {
        let editor = state.editor.clone();
        let path = state.worktree_path;
        this.commit_worktree_rename(path, &editor, cx);
    }
}))
.on_action(cx.listener(|this, _: &CancelWorktreeGroupRename, _window, cx| {
    this.cancel_worktree_rename(cx);
}))
```

Then register keybindings in `assets/keymaps/default.json` (search for an existing `agents_sidebar::` keybinding and follow the pattern):

```jsonc
{
  "context": "ThreadsSidebar > Editor",
  "bindings": {
    "enter": "agents_sidebar::ConfirmWorktreeGroupRename",
    "escape": "agents_sidebar::CancelWorktreeGroupRename"
  }
}
```

If the existing sidebar context name differs, match it (check the `dispatch_context` function in sidebar.rs around line 2426). The KeyContext is `"ThreadsSidebar"` for this file.

- [ ] **Step 4: Build**

Run: `./script/clippy -p sidebar`
Expected: builds.

- [ ] **Step 5: Manual verification**

Run: `cargo run --bin zed`. Right-click a worktree subheader → "Rename Group" → type a new name → press Enter. Confirm:

1. The new name is shown.
2. Restart Zed; the name persists.
3. Right-click → "Reset Group Name". Confirm the derived name returns.

- [ ] **Step 6: Commit**

```bash
git add crates/sidebar/src/sidebar.rs assets/keymaps/default.json
git commit -m "sidebar: Inline rename for worktree groups with persistence"
```

---

## Task 12: Cleanup orphan worktree overrides at end of `rebuild_contents`

**Files:**
- Modify: `crates/sidebar/src/sidebar.rs:1589-1594` (end of `rebuild_contents`)

- [ ] **Step 1: Collect known paths**

In `rebuild_contents`, after the `for group in &groups` loop and before assigning `self.contents = ...` (line ~1590), collect all main worktree paths currently in any project group:

```rust
let known_worktree_paths: HashSet<PathBuf> = groups
    .iter()
    .flat_map(|g| g.key.path_list().paths().iter().map(|p| p.to_path_buf()))
    .collect();
ThreadMetadataStore::global(cx).update(cx, |store, cx| {
    store.cleanup_worktree_overrides_not_in(&known_worktree_paths, cx);
});
```

Note: `cx` here is `&App`, but `cleanup_worktree_overrides_not_in` takes `&mut Context<Self>` on `ThreadMetadataStore`. The function signature of `rebuild_contents` is `fn rebuild_contents(&mut self, cx: &App)` — verify this. If `cx` is `&App` we can't `update` directly. Two options:

- **Option A:** Defer cleanup to `update_entries` (called from `rebuild_contents`'s caller), passing `&mut Context<Self>`.
- **Option B:** Schedule a microtask: `cx.spawn(...)` — but `&App` doesn't have `spawn` either.

Check the actual signature. Searching shows `rebuild_contents(&mut self, cx: &App)`, so cleanup must happen in `update_entries`. Move the call:

```rust
fn update_entries(&mut self, cx: &mut Context<Self>) {
    self.rebuild_contents(cx);
    // (existing code, e.g. update list_state...)

    // Collect known paths from the post-rebuild contents.
    let known_worktree_paths: HashSet<PathBuf> = self
        .contents
        .entries
        .iter()
        .filter_map(|entry| match entry {
            ListEntry::WorktreeHeader { worktree_path, .. } => Some(worktree_path.clone()),
            _ => None,
        })
        .collect();
    ThreadMetadataStore::global(cx).update(cx, |store, cx| {
        store.cleanup_worktree_overrides_not_in(&known_worktree_paths, cx);
    });
}
```

Find `update_entries` in `sidebar.rs` (search `fn update_entries(`). Append the cleanup block after the existing body.

- [ ] **Step 2: Write a test**

Append to `crates/sidebar/src/sidebar_tests.rs`:

```rust
#[gpui::test]
async fn test_worktree_override_cleanup_on_close(cx: &mut TestAppContext) {
    init_test(cx);
    let project = init_test_project("/my-project", cx).await;
    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
    let sidebar = setup_sidebar(&multi_workspace, cx);

    // Save a thread so the project group appears in the sidebar.
    let session_id = acp::SessionId::new(Arc::from("s1"));
    save_thread_metadata(
        session_id.clone(),
        Some("Test thread".into()),
        Utc::now(),
        None,
        None,
        &project,
        cx,
    );
    cx.run_until_parked();

    let worktree_path = PathBuf::from("/my-project");
    cx.update(|cx| {
        ThreadMetadataStore::global(cx).update(cx, |store, cx| {
            store.set_worktree_custom_name(
                worktree_path.clone(),
                SharedString::from("renamed"),
                cx,
            );
        });
    });
    sidebar.update(cx, |_, cx| cx.notify());
    cx.run_until_parked();

    cx.update(|cx| {
        assert_eq!(
            ThreadMetadataStore::global(cx)
                .read(cx)
                .worktree_custom_name(&worktree_path),
            Some(SharedString::from("renamed"))
        );
    });

    // Remove the only thread referencing this worktree path. The project
    // group disappears; cleanup should remove the override.
    cx.update(|cx| {
        ThreadMetadataStore::global(cx).update(cx, |store, cx| {
            // Find the thread id from session_id and delete.
            let tid = store.entry_by_session(&session_id).map(|e| e.thread_id);
            if let Some(tid) = tid {
                store.archive(tid, None, cx);
            }
        });
    });
    sidebar.update(cx, |s, cx| s.update_entries(cx));
    cx.run_until_parked();

    cx.update(|cx| {
        assert_eq!(
            ThreadMetadataStore::global(cx)
                .read(cx)
                .worktree_custom_name(&worktree_path),
            None,
            "custom name should have been cleaned up after worktree no longer in any group"
        );
    });
}
```

(`update_entries` is `pub(super)` or `pub(crate)` — verify visibility. If private, call `sidebar.update(cx, |s, cx| s.update_entries(cx))` may fail. In that case add `#[cfg(test)] pub fn update_entries_for_test(...)` wrapper or change visibility to `pub(crate)`.)

- [ ] **Step 3: Run the test**

Run: `cargo test -p sidebar test_worktree_override_cleanup_on_close`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/sidebar/src/sidebar.rs crates/sidebar/src/sidebar_tests.rs
git commit -m "sidebar: Cleanup orphan worktree overrides after rebuild"
```

---

## Task 13: Test — worktree headers appear under the project header

**Files:**
- Modify: `crates/sidebar/src/sidebar_tests.rs`

- [ ] **Step 1: Write the test**

Append to the test file (after `test_single_workspace_with_saved_threads`, around line 694):

```rust
#[gpui::test]
async fn test_threads_grouped_under_worktree_header(cx: &mut TestAppContext) {
    init_test(cx);
    let project = init_test_project("/my-project", cx).await;
    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
    let sidebar = setup_sidebar(&multi_workspace, cx);

    let now = Utc::now();
    save_thread_metadata(
        acp::SessionId::new(Arc::from("s1")),
        Some("Thread A".into()),
        now,
        None,
        None,
        &project,
        cx,
    );
    cx.run_until_parked();

    let lines = visible_entries_as_strings(&sidebar, cx);
    // Expected: project header, worktree subheader for `/my-project`, then thread.
    assert!(
        lines
            .iter()
            .any(|s| s.contains("my-project") && !s.contains("Thread A")),
        "expected a worktree subheader row, got: {lines:?}"
    );
    assert!(
        lines.iter().any(|s| s.contains("Thread A")),
        "expected a thread row, got: {lines:?}"
    );
}
```

`visible_entries_as_strings` is an existing helper — confirm its presence in `sidebar_tests.rs` (search). If it doesn't include worktree headers, extend it to render them as `"  [worktree] {display_name}"` strings. Search for `fn visible_entries_as_strings(` and add an arm:

```rust
ListEntry::WorktreeHeader { display_name, .. } => {
    Some(format!("  [worktree] {display_name}"))
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p sidebar test_threads_grouped_under_worktree_header`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/sidebar/src/sidebar_tests.rs
git commit -m "sidebar: Test that threads appear under worktree subheader"
```

---

## Task 14: Test — empty worktree group renders a header

**Files:**
- Modify: `crates/sidebar/src/sidebar_tests.rs`

- [ ] **Step 1: Write the test**

```rust
#[gpui::test]
async fn test_empty_worktree_renders_header(cx: &mut TestAppContext) {
    init_test(cx);
    // Open a project with two worktrees but only one has a thread.
    let project = init_test_project_multi(&["/proj-main", "/proj-feature"], cx).await;
    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
    let sidebar = setup_sidebar(&multi_workspace, cx);

    let now = Utc::now();
    save_thread_metadata_with_main_paths(
        "s1",
        "Thread A",
        PathList::new(&[PathBuf::from("/proj-main")]),
        PathList::new(&[PathBuf::from("/proj-main")]),
        now,
        cx,
    );
    cx.run_until_parked();

    let lines = visible_entries_as_strings(&sidebar, cx);
    let header_count = lines
        .iter()
        .filter(|s| s.contains("[worktree]"))
        .count();
    assert_eq!(
        header_count, 2,
        "expected one worktree header per main path (incl. empty), got: {lines:?}"
    );
}
```

`init_test_project_multi` may not exist — if not, write it next to `init_test_project` in `sidebar_tests.rs` modeled after the single-path version, accepting `&[&str]` for roots.

- [ ] **Step 2: Run**

Run: `cargo test -p sidebar test_empty_worktree_renders_header`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/sidebar/src/sidebar_tests.rs
git commit -m "sidebar: Test empty worktrees still render their headers"
```

---

## Task 15: Test — inline rename persists and is reflected on rebuild

**Files:**
- Modify: `crates/sidebar/src/sidebar_tests.rs`

- [ ] **Step 1: Write the test**

```rust
#[gpui::test]
async fn test_inline_rename_persists(cx: &mut TestAppContext) {
    init_test(cx);
    let project = init_test_project("/my-project", cx).await;
    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
    let sidebar = setup_sidebar(&multi_workspace, cx);

    save_thread_metadata(
        acp::SessionId::new(Arc::from("s1")),
        Some("Thread A".into()),
        Utc::now(),
        None,
        None,
        &project,
        cx,
    );
    cx.run_until_parked();

    let path = PathBuf::from("/my-project");
    cx.update(|cx| {
        ThreadMetadataStore::global(cx).update(cx, |store, cx| {
            store.set_worktree_custom_name(
                path.clone(),
                SharedString::from("My Renamed Group"),
                cx,
            );
        });
    });
    sidebar.update(cx, |s, cx| s.update_entries(cx));
    cx.run_until_parked();

    let lines = visible_entries_as_strings(&sidebar, cx);
    assert!(
        lines.iter().any(|s| s.contains("My Renamed Group")),
        "expected the custom name in headers, got: {lines:?}"
    );
}
```

- [ ] **Step 2: Run**

Run: `cargo test -p sidebar test_inline_rename_persists`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/sidebar/src/sidebar_tests.rs
git commit -m "sidebar: Test inline rename of worktree group"
```

---

## Task 16: Manual smoke check + final clippy

- [ ] **Step 1: Run full clippy**

Run: `./script/clippy`
Expected: zero warnings/errors in `crates/sidebar` and `crates/agent_ui`.

- [ ] **Step 2: Run full test suites for the two crates**

Run: `cargo test -p sidebar -p agent_ui`
Expected: all tests pass.

- [ ] **Step 3: Manual UI smoke**

Run: `cargo run --bin zed`. Verify:
1. Open a single-worktree project → exactly one worktree subheader appears under the project header. Branch chip is visible on the right. Chip color reflects git state (make a file dirty to see amber, add an untracked file in a clean repo to see blue, create a merge conflict to see red).
2. Open a multi-root workspace → one worktree subheader per main path.
3. Click chevron → header collapses, threads hide, count badge appears.
4. Right-click subheader → "Rename Group" → type → Enter. Name persists across app restart.
5. Right-click → "Reset Group Name" → derived name returns.
6. Close the project (Remove from sidebar) → reopen Zed → confirm the override is gone for that path (check via DB: `sqlite3 ~/Library/Application\ Support/Zed/db/0-*.sqlite "SELECT * FROM worktree_group_overrides"`).

- [ ] **Step 4: Commit any final cleanups**

If clippy or tests required tweaks, commit them:

```bash
git add -p
git commit -m "sidebar: Cleanups for worktree tree view"
```

---

## Notes for the executing engineer

- **`PathList::paths()` return type:** `&[Arc<Path>]`. Adjust dereferences accordingly when comparing to `Path` or `PathBuf`.
- **`Disclosure::new`** signature: `fn new(id: impl Into<ElementId>, is_open: bool)`. `is_open` is the *expanded* state — so pass `!is_collapsed`.
- **`Editor::set_text`** is `fn set_text(&mut self, text: impl Into<Arc<str>>, window: &mut Window, cx: &mut Context<Self>)`. Verify the exact signature when wiring step 11.2 — adapt if needed.
- **`ContextMenu::build` vs `ContextMenu::build_persistent`:** Use `build` for one-shot menus (matches our use). The existing project-header menu uses `build_persistent` because it keeps state across opens; we don't need that.
- **`PopoverMenuHandle`** is `Clone` and stores the menu state. Cache one per worktree path in `worktree_header_menu_handles` so right-click can open it programmatically.
- **Single-worktree projects:** Per the design discussion, we *always* render a worktree subheader, even for single-worktree projects, to keep the UI consistent and provide a stable home for the branch chip and rename. If user feedback during execution prefers collapsing this case, change Task 8 to only emit a `WorktreeHeader` when `group_paths.paths().len() > 1`.
- **Closed worktrees:** Out of scope. The user confirmed closing a worktree closes its threads (they move to Thread History, which is a separate view).
- **Sort order within a worktree:** Most-recent first (preserves today's behavior). Reorder is Plan B.
