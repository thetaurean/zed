# Reorderable Headers Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add drag-and-drop reordering to the agents sidebar (`crates/sidebar`) for both project group headers and worktree subheaders, with a horizontal drop-target indicator above/below the hovered header, and persisted order across restarts.

**Architecture:** Project header order already round-trips through `MultiWorkspace.project_groups: Vec<ProjectGroupState>` and its serialization — the project drop handler reorders that `Vec` in place and calls `mw.serialize(cx)`. Worktree subheader order is **per (project group, worktree path)** and lives in a new SQLite table `worktree_group_order` owned by `ThreadMetadataStore`, queried at render time to sort the worktree path list within each group. The UI uses GPUI's `on_drag` / `on_drag_move` / `on_drop` primitives; the sidebar tracks one transient `Option<DropTargetIndicator>` so the rebuild can render a 2px bar above/below the hovered target.

**Out of scope:**
- Reordering threads within a worktree group (still sorted by display time — most-recent first).
- Cross-group moves of worktrees (the worktree path stays in whatever `ProjectGroupKey.path_list()` it lives in; reorder only changes display order).
- Dropping a project header inside another (no nesting).

**Tech Stack:** Rust, GPUI (`on_drag`, `on_drag_move`, `on_drop`, `DragMoveEvent`), sqlez (SQLite), Zed UI primitives.

**Prior context:** This builds on the worktree tree view shipped in [`2026-05-14-worktree-tree-view-plan-a.md`](./2026-05-14-worktree-tree-view-plan-a.md). The `ListEntry::WorktreeHeader` variant, `WorktreeGroupOverride` struct, `worktree_group_overrides` table, and per-worktree disclosure already exist. This plan adds a sibling concept for order.

---

## File Structure

**Modify:**

- `crates/agent_ui/src/thread_metadata_store.rs`
  - New migration: `worktree_group_order` SQLite table
  - New row type `WorktreeOrderRow`
  - New `ThreadMetadataDb` methods: `load_worktree_order`, `upsert_worktree_order_entries`, `delete_worktree_order_for_group`, `delete_worktree_order_not_in_groups`
  - New cached field on `ThreadMetadataStore`: `worktree_order: HashMap<WorktreeOrderKey, u32>` keyed by `(remote_connection_identity, group_path_list_canonical, worktree_path)`
  - New `ThreadMetadataStore` methods: `worktree_order_for_group`, `set_worktree_order_for_group`, `cleanup_worktree_order_not_in_groups`

- `crates/workspace/src/multi_workspace.rs`
  - New mutator method `reorder_project_groups(from: &ProjectGroupKey, to: &ProjectGroupKey, edge: DropEdge)` that reorders the `project_groups: Vec<ProjectGroupState>` in place. Callers must then invoke `serialize(cx)`.
  - New public `DropEdge` enum (or re-export the one defined in `sidebar.rs` — see Task 5).

- `crates/sidebar/src/sidebar.rs`
  - New free-standing `DropEdge` enum (`Above`, `Below`).
  - New `DraggedSidebarHeader` drag-value enum: `Project(ProjectGroupKey)` and `Worktree { project_group_key: ProjectGroupKey, worktree_path: PathBuf }`.
  - New `DropTargetIndicator` enum and sidebar state field `drop_target: Option<DropTargetIndicator>`.
  - New view type `DraggedHeaderView` rendering the drag preview.
  - Modified `rebuild_contents`: sorts a group's worktree paths by stored order (paths with no stored position fall back to natural `PathList` order, stable-sort).
  - Modified `render_project_header`: wraps in `on_drag` (project drag source), `on_drag_move` (top/bottom edge detection), `on_drop` (reorder + serialize), and renders the indicator bar when this header is the active drop target.
  - Modified `render_worktree_header`: same wiring for worktree-scoped drag, but rejects drops from a different project group.
  - New helper methods: `begin_header_drag`, `compute_drop_edge`, `on_header_drop_project`, `on_header_drop_worktree`, `clear_drop_target`.

**Modify (tests):**

- `crates/sidebar/src/sidebar_tests.rs`
  - Three new tests: project reorder persists, worktree reorder within group, worktree drop rejected across groups.
  - One new helper for asserting the order of `WorktreeHeader` entries within a group.

**Modify (a few rotated arms):**

- Any exhaustive `match` on `ListEntry` does not change — we don't add new variants.

No new files. Helpers live in `sidebar.rs` alongside `render_worktree_header` (matching the pattern from Plan A).

---

## Conventions used throughout

- Run clippy via `./script/clippy` (project rule from `CLAUDE.md`), **not** `cargo clippy`.
- Use `cx.background_executor().timer(...)` in tests, **not** `smol::Timer::after(...)`.
- Never use `.unwrap()` outside tests — use `?`, `let ... else`, or pattern-match.
- Never silently discard fallible results with `let _ =` — use `.log_err()` or propagate via `?`.
- Drop handlers always: (a) mutate state, (b) persist (DB upsert or `mw.serialize(cx)`), (c) clear `self.drop_target`, (d) call `self.update_entries(cx)`.
- Drag-source elements use the entry's stable identifier as the drag value (a `ProjectGroupKey` or `(ProjectGroupKey, PathBuf)`), never the list index — indices shift on rebuild.

---

## Task 1: Add `worktree_group_order` SQLite table + migration

**Files:**
- Modify: `crates/agent_ui/src/thread_metadata_store.rs` (the `MIGRATIONS` array; the previous migration ends around line 1692)

- [ ] **Step 1: Locate `MIGRATIONS`**

Open `crates/agent_ui/src/thread_metadata_store.rs`, find `impl Domain for ThreadMetadataDb`. Inside it, `const MIGRATIONS: &[&str] = &[ ... ]`. The final entry (added by Plan A) creates `worktree_group_overrides`. Append the new migration **after** that closing `,` and before the `];`.

- [ ] **Step 2: Add the migration**

```rust
sql!(
    CREATE TABLE IF NOT EXISTS worktree_group_order(
        remote_connection_identity TEXT NOT NULL,
        group_path_list TEXT NOT NULL,
        worktree_path TEXT NOT NULL,
        position INTEGER NOT NULL,
        PRIMARY KEY(remote_connection_identity, group_path_list, worktree_path)
    ) STRICT;
),
```

`group_path_list` is the serialized canonical path list of the project group key (we will use `path_list.serialize()` from Task 2). Position is `0..N`.

- [ ] **Step 3: Verify schema compiles**

Run: `./script/clippy -p agent_ui`
Expected: builds without errors.

- [ ] **Step 4: Commit**

```bash
git add crates/agent_ui/src/thread_metadata_store.rs
git commit -m "agent_ui: Add worktree_group_order table for per-group worktree ordering"
```

---

## Task 2: Add `WorktreeOrderRow` + DB read/write methods

**Files:**
- Modify: `crates/agent_ui/src/thread_metadata_store.rs` — add row struct near `WorktreeOverrideRow`; add methods in `impl ThreadMetadataDb`

- [ ] **Step 1: Add the row struct**

Insert near `WorktreeOverrideRow` (Plan A added this; search for `struct WorktreeOverrideRow`):

```rust
#[derive(Clone, Debug)]
struct WorktreeOrderRow {
    remote_connection_identity: String,
    group_path_list: String,
    worktree_path: PathBuf,
    position: u32,
}

impl Column for WorktreeOrderRow {
    fn column(statement: &mut Statement, start_index: i32) -> anyhow::Result<(Self, i32)> {
        let (remote_connection_identity, i): (String, i32) =
            Column::column(statement, start_index)?;
        let (group_path_list, i): (String, i32) = Column::column(statement, i)?;
        let (path_str, i): (String, i32) = Column::column(statement, i)?;
        let (position, i): (i64, i32) = Column::column(statement, i)?;
        Ok((
            WorktreeOrderRow {
                remote_connection_identity,
                group_path_list,
                worktree_path: PathBuf::from(path_str),
                position: position.max(0) as u32,
            },
            i,
        ))
    }
}
```

- [ ] **Step 2: Add async DB methods on `ThreadMetadataDb`**

Find the existing `impl ThreadMetadataDb` block that contains the worktree-override methods from Plan A (`load_worktree_overrides`, `upsert_worktree_override`, etc.). Append:

```rust
pub async fn load_worktree_order(&self) -> anyhow::Result<Vec<WorktreeOrderRow>> {
    self.select::<WorktreeOrderRow>(
        "SELECT remote_connection_identity, group_path_list, worktree_path, position \
         FROM worktree_group_order \
         ORDER BY remote_connection_identity, group_path_list, position",
    )?()
}

pub async fn upsert_worktree_order_entries(
    &self,
    remote_connection_identity: String,
    group_path_list: String,
    ordered_paths: Vec<PathBuf>,
) -> anyhow::Result<()> {
    self.write(move |conn| {
        // Replace the entire ordering for (identity, group_path_list) atomically:
        // delete-then-insert under one transaction provided by `write`.
        let mut delete_stmt = Statement::prepare(
            conn,
            "DELETE FROM worktree_group_order \
             WHERE remote_connection_identity = ?1 AND group_path_list = ?2",
        )?;
        let i = delete_stmt.bind(&remote_connection_identity, 1)?;
        delete_stmt.bind(&group_path_list, i)?;
        delete_stmt.exec()?;

        if ordered_paths.is_empty() {
            return Ok(());
        }

        let mut insert_stmt = Statement::prepare(
            conn,
            "INSERT INTO worktree_group_order(\
                 remote_connection_identity, group_path_list, worktree_path, position) \
             VALUES(?1, ?2, ?3, ?4)",
        )?;
        for (position, path) in ordered_paths.iter().enumerate() {
            insert_stmt.reset()?;
            let path_str = path.to_string_lossy().to_string();
            let i = insert_stmt.bind(&remote_connection_identity, 1)?;
            let i = insert_stmt.bind(&group_path_list, i)?;
            let i = insert_stmt.bind(&path_str, i)?;
            insert_stmt.bind(&(position as i64), i)?;
            insert_stmt.exec()?;
        }
        Ok(())
    })
    .await
}

pub async fn delete_worktree_order_for_group(
    &self,
    remote_connection_identity: String,
    group_path_list: String,
) -> anyhow::Result<()> {
    self.write(move |conn| {
        let mut stmt = Statement::prepare(
            conn,
            "DELETE FROM worktree_group_order \
             WHERE remote_connection_identity = ?1 AND group_path_list = ?2",
        )?;
        let i = stmt.bind(&remote_connection_identity, 1)?;
        stmt.bind(&group_path_list, i)?;
        stmt.exec()
    })
    .await
}

pub async fn delete_worktree_order_not_in_groups(
    &self,
    keep: Vec<(String, String)>,
) -> anyhow::Result<()> {
    self.write(move |conn| {
        if keep.is_empty() {
            let mut stmt = Statement::prepare(conn, "DELETE FROM worktree_group_order")?;
            return stmt.exec();
        }
        // Build a parameterized NOT IN over the tuple (identity, group_path_list).
        // SQLite does not support tuple-NOT-IN with binds directly, so build a
        // VALUES list of placeholders and use a subquery.
        let values_clause = std::iter::repeat("(?, ?)")
            .take(keep.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "DELETE FROM worktree_group_order \
             WHERE (remote_connection_identity, group_path_list) NOT IN \
                 (VALUES {values_clause})"
        );
        let mut stmt = Statement::prepare(conn, &sql)?;
        let mut i = 1;
        for (identity, group_path_list) in &keep {
            i = stmt.bind(identity, i)?;
            i = stmt.bind(group_path_list, i)?;
        }
        stmt.exec()
    })
    .await
}
```

If `Statement::reset` is not available, drop the reset call and instead `Statement::prepare` inside the loop. The exact `sqlez` API conventions can be confirmed by inspecting how `upsert_worktree_override` from Plan A handles the prepared-statement pattern — match that style.

- [ ] **Step 3: Build**

Run: `./script/clippy -p agent_ui`
Expected: builds without errors.

- [ ] **Step 4: Commit**

```bash
git add crates/agent_ui/src/thread_metadata_store.rs
git commit -m "agent_ui: Add WorktreeOrderRow DB read/write methods"
```

---

## Task 3: Cache `worktree_order` in `ThreadMetadataStore` and load on startup

**Files:**
- Modify: `crates/agent_ui/src/thread_metadata_store.rs` — struct fields, `new`, initial load

- [ ] **Step 1: Add the cache type aliases**

Near the existing `WorktreeGroupOverrideKey` (Plan A defined this around the same place as `WorktreeGroupOverride`), add:

```rust
/// Key for per-(group, worktree) ordering state.
/// `group_path_list_canonical` is the serialized canonical path list of the
/// owning `ProjectGroupKey`. Position is 0-based ascending; missing paths sort
/// last by natural order.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct WorktreeOrderKey {
    pub remote_connection_identity: String,
    pub group_path_list_canonical: String,
    pub worktree_path: PathBuf,
}
```

- [ ] **Step 2: Add cached field to `ThreadMetadataStore`**

Find the struct (search for `pub struct ThreadMetadataStore {`). After the existing `worktree_overrides: HashMap<...>` field added by Plan A, add:

```rust
    worktree_order: HashMap<WorktreeOrderKey, u32>,
    worktree_order_loaded: bool,
```

- [ ] **Step 3: Initialize the field in `new`**

In `Self { ... }` literal inside `fn new`, add (matching alphabetical / logical group with other defaults):

```rust
    worktree_order: HashMap::default(),
    worktree_order_loaded: false,
```

- [ ] **Step 4: Spawn an async initial load alongside the existing worktree-override load**

In `fn new`, locate the spawn block added by Plan A that calls `db.load_worktree_overrides().await`. Add a sibling spawn block immediately after it:

```rust
{
    let db = self.db.clone();
    cx.spawn(async move |this, cx| {
        match db.load_worktree_order().await {
            Ok(rows) => {
                this.update(cx, |this, cx| {
                    for row in rows {
                        let key = WorktreeOrderKey {
                            remote_connection_identity: row.remote_connection_identity,
                            group_path_list_canonical: row.group_path_list,
                            worktree_path: row.worktree_path,
                        };
                        this.worktree_order.insert(key, row.position);
                    }
                    this.worktree_order_loaded = true;
                    cx.notify();
                })
                .log_err();
            }
            Err(err) => log::error!("failed to load worktree order: {err:?}"),
        }
    })
    .detach();
}
```

Note: this is the same shape as the Plan A worktree-overrides load. Plan A's code lives inside `Self::new` *after* the struct literal returns from `new` — match that placement exactly.

- [ ] **Step 5: Build**

Run: `./script/clippy -p agent_ui`
Expected: builds.

- [ ] **Step 6: Commit**

```bash
git add crates/agent_ui/src/thread_metadata_store.rs
git commit -m "agent_ui: Load worktree group order into ThreadMetadataStore on startup"
```

---

## Task 4: Add `ThreadMetadataStore` mutator + reader methods

**Files:**
- Modify: `crates/agent_ui/src/thread_metadata_store.rs` (in `impl ThreadMetadataStore`, near the Plan A `set_worktree_collapsed` / `cleanup_worktree_overrides_not_in` methods)

- [ ] **Step 1: Write the failing test**

Append to (or, if absent, create) the test module at the bottom of `thread_metadata_store.rs`. The Plan A test module is named `worktree_override_tests`; create a sibling module `worktree_order_tests`:

```rust
#[cfg(test)]
mod worktree_order_tests {
    use super::*;
    use gpui::TestAppContext;

    fn key(group: &str, path: &str) -> (String, String, PathBuf) {
        (
            "local".to_string(),
            group.to_string(),
            PathBuf::from(path),
        )
    }

    #[gpui::test]
    async fn test_set_and_read_worktree_order(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            ThreadMetadataStore::init_global(cx);
        });

        let (identity, group, a) = key("/proj-root", "/proj-root/a");
        let (_, _, b) = key("/proj-root", "/proj-root/b");
        let (_, _, c) = key("/proj-root", "/proj-root/c");

        cx.update(|cx| {
            ThreadMetadataStore::global(cx).update(cx, |store, cx| {
                store.set_worktree_order_for_group(
                    identity.clone(),
                    group.clone(),
                    vec![c.clone(), a.clone(), b.clone()],
                    cx,
                );
            });
        });

        cx.update(|cx| {
            let store = ThreadMetadataStore::global(cx).read(cx);
            let order = store.worktree_order_for_group(&identity, &group);
            assert_eq!(order, vec![c.clone(), a.clone(), b.clone()]);
        });
    }

    #[gpui::test]
    async fn test_cleanup_removes_orphan_groups(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = settings::SettingsStore::test(cx);
            cx.set_global(settings_store);
            ThreadMetadataStore::init_global(cx);
        });

        let (identity, keep_group, p1) = key("/keep", "/keep/p1");
        let (_, drop_group, p2) = key("/drop", "/drop/p2");

        cx.update(|cx| {
            ThreadMetadataStore::global(cx).update(cx, |store, cx| {
                store.set_worktree_order_for_group(
                    identity.clone(),
                    keep_group.clone(),
                    vec![p1.clone()],
                    cx,
                );
                store.set_worktree_order_for_group(
                    identity.clone(),
                    drop_group.clone(),
                    vec![p2.clone()],
                    cx,
                );
            });
        });

        cx.update(|cx| {
            ThreadMetadataStore::global(cx).update(cx, |store, cx| {
                let mut keep = std::collections::HashSet::new();
                keep.insert((identity.clone(), keep_group.clone()));
                store.cleanup_worktree_order_not_in_groups(&keep, cx);
            });
        });

        cx.update(|cx| {
            let store = ThreadMetadataStore::global(cx).read(cx);
            assert_eq!(
                store.worktree_order_for_group(&identity, &keep_group),
                vec![p1.clone()]
            );
            assert!(
                store
                    .worktree_order_for_group(&identity, &drop_group)
                    .is_empty()
            );
        });
    }
}
```

- [ ] **Step 2: Run the test, confirm it fails**

Run: `cargo test -p agent_ui worktree_order_tests`
Expected: FAIL — methods `set_worktree_order_for_group`, `worktree_order_for_group`, `cleanup_worktree_order_not_in_groups` do not exist.

- [ ] **Step 3: Implement the methods**

In `impl ThreadMetadataStore`, near the Plan A `cleanup_worktree_overrides_not_in`, add:

```rust
pub fn worktree_order_for_group(
    &self,
    remote_connection_identity: &str,
    group_path_list_canonical: &str,
) -> Vec<PathBuf> {
    let mut entries: Vec<(u32, PathBuf)> = self
        .worktree_order
        .iter()
        .filter_map(|(key, position)| {
            (key.remote_connection_identity == remote_connection_identity
                && key.group_path_list_canonical == group_path_list_canonical)
                .then(|| (*position, key.worktree_path.clone()))
        })
        .collect();
    entries.sort_by_key(|(position, _)| *position);
    entries.into_iter().map(|(_, path)| path).collect()
}

pub fn set_worktree_order_for_group(
    &mut self,
    remote_connection_identity: String,
    group_path_list_canonical: String,
    ordered_paths: Vec<PathBuf>,
    cx: &mut Context<Self>,
) {
    // Drop all existing entries for this group, then re-insert in order.
    self.worktree_order.retain(|key, _| {
        !(key.remote_connection_identity == remote_connection_identity
            && key.group_path_list_canonical == group_path_list_canonical)
    });
    for (position, path) in ordered_paths.iter().enumerate() {
        let key = WorktreeOrderKey {
            remote_connection_identity: remote_connection_identity.clone(),
            group_path_list_canonical: group_path_list_canonical.clone(),
            worktree_path: path.clone(),
        };
        self.worktree_order.insert(key, position as u32);
    }

    let db = self.db.clone();
    let identity_for_db = remote_connection_identity;
    let group_for_db = group_path_list_canonical;
    let paths_for_db = ordered_paths;
    cx.background_spawn(async move {
        db.upsert_worktree_order_entries(identity_for_db, group_for_db, paths_for_db)
            .await
            .log_err();
    })
    .detach();

    cx.notify();
}

pub fn cleanup_worktree_order_not_in_groups(
    &mut self,
    keep: &HashSet<(String, String)>,
    cx: &mut Context<Self>,
) {
    let initial_len = self.worktree_order.len();
    self.worktree_order.retain(|key, _| {
        keep.contains(&(
            key.remote_connection_identity.clone(),
            key.group_path_list_canonical.clone(),
        ))
    });
    if self.worktree_order.len() == initial_len {
        return;
    }

    let db = self.db.clone();
    let keep_vec: Vec<(String, String)> = keep.iter().cloned().collect();
    cx.background_spawn(async move {
        db.delete_worktree_order_not_in_groups(keep_vec)
            .await
            .log_err();
    })
    .detach();

    cx.notify();
}
```

- [ ] **Step 4: Run the test, confirm it passes**

Run: `cargo test -p agent_ui worktree_order_tests`
Expected: both tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/agent_ui/src/thread_metadata_store.rs
git commit -m "agent_ui: Add ThreadMetadataStore methods for per-group worktree order"
```

---

## Task 5: Add `reorder_project_groups` to `MultiWorkspace`

**Files:**
- Modify: `crates/workspace/src/multi_workspace.rs` (near `group_state_by_key_mut`, around line 836-847)

- [ ] **Step 1: Add a `DropEdge` enum to a shared spot**

We need both `multi_workspace.rs` and `sidebar.rs` to talk about "above" vs "below". Define the enum in `multi_workspace.rs` and `pub use` it. Insert near the top of the file, with the other public types:

```rust
/// Where a drop falls relative to the hovered header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropEdge {
    Above,
    Below,
}
```

- [ ] **Step 2: Write the failing test**

In a `#[cfg(test)] mod tests` block at the bottom of `multi_workspace.rs` (search for existing test module; if none, create one), add:

```rust
#[cfg(test)]
mod project_reorder_tests {
    use super::*;

    fn fake_key(path: &str) -> ProjectGroupKey {
        ProjectGroupKey::new(None, PathList::new(&[PathBuf::from(path)]))
    }

    fn fake_state(path: &str) -> ProjectGroupState {
        ProjectGroupState {
            key: fake_key(path),
            expanded: true,
            last_active_workspace: None,
        }
    }

    #[test]
    fn test_reorder_moves_group_above_target() {
        // [A, B, C], move C above A => [C, A, B]
        let mut groups = vec![fake_state("/a"), fake_state("/b"), fake_state("/c")];
        MultiWorkspace::reorder_project_groups_in_vec(
            &mut groups,
            &fake_key("/c"),
            &fake_key("/a"),
            DropEdge::Above,
        );
        assert_eq!(
            groups.iter().map(|g| g.key.clone()).collect::<Vec<_>>(),
            vec![fake_key("/c"), fake_key("/a"), fake_key("/b")]
        );
    }

    #[test]
    fn test_reorder_moves_group_below_target() {
        // [A, B, C], move A below B => [B, A, C]
        let mut groups = vec![fake_state("/a"), fake_state("/b"), fake_state("/c")];
        MultiWorkspace::reorder_project_groups_in_vec(
            &mut groups,
            &fake_key("/a"),
            &fake_key("/b"),
            DropEdge::Below,
        );
        assert_eq!(
            groups.iter().map(|g| g.key.clone()).collect::<Vec<_>>(),
            vec![fake_key("/b"), fake_key("/a"), fake_key("/c")]
        );
    }

    #[test]
    fn test_reorder_self_is_noop() {
        let mut groups = vec![fake_state("/a"), fake_state("/b")];
        MultiWorkspace::reorder_project_groups_in_vec(
            &mut groups,
            &fake_key("/a"),
            &fake_key("/a"),
            DropEdge::Above,
        );
        assert_eq!(
            groups.iter().map(|g| g.key.clone()).collect::<Vec<_>>(),
            vec![fake_key("/a"), fake_key("/b")]
        );
    }
}
```

- [ ] **Step 3: Run the test, confirm it fails**

Run: `cargo test -p workspace project_reorder_tests`
Expected: FAIL — `reorder_project_groups_in_vec` does not exist.

- [ ] **Step 4: Implement the pure helper + public method**

Inside `impl MultiWorkspace`, near `group_state_by_key_mut`:

```rust
/// Pure helper for testability. Reorders `groups` in place so that the
/// group identified by `from` is positioned `edge` relative to the group
/// identified by `to`. No-op if either group is missing, or if `from == to`.
pub(crate) fn reorder_project_groups_in_vec(
    groups: &mut Vec<ProjectGroupState>,
    from: &ProjectGroupKey,
    to: &ProjectGroupKey,
    edge: DropEdge,
) {
    if from == to {
        return;
    }
    let Some(from_ix) = groups.iter().position(|g| g.key == *from) else {
        return;
    };
    let Some(to_ix) = groups.iter().position(|g| g.key == *to) else {
        return;
    };

    let moved = groups.remove(from_ix);
    // `to_ix` may shift after the remove if `from_ix < to_ix`.
    let adjusted_to_ix = if from_ix < to_ix { to_ix - 1 } else { to_ix };
    let insert_at = match edge {
        DropEdge::Above => adjusted_to_ix,
        DropEdge::Below => adjusted_to_ix + 1,
    };
    let insert_at = insert_at.min(groups.len());
    groups.insert(insert_at, moved);
}

/// Reorder project groups in response to a drag-and-drop event.
/// Mutates the `project_groups` Vec; callers are responsible for calling
/// `self.serialize(cx)` to persist.
pub fn reorder_project_groups(
    &mut self,
    from: &ProjectGroupKey,
    to: &ProjectGroupKey,
    edge: DropEdge,
) {
    Self::reorder_project_groups_in_vec(&mut self.project_groups, from, to, edge);
}
```

- [ ] **Step 5: Run the test, confirm it passes**

Run: `cargo test -p workspace project_reorder_tests`
Expected: all three PASS.

- [ ] **Step 6: Build the whole crate**

Run: `./script/clippy -p workspace`
Expected: builds.

- [ ] **Step 7: Commit**

```bash
git add crates/workspace/src/multi_workspace.rs
git commit -m "workspace: Add MultiWorkspace::reorder_project_groups"
```

---

## Task 6: Sort worktree paths in `rebuild_contents` by stored order

**Files:**
- Modify: `crates/sidebar/src/sidebar.rs` (the `rebuild_contents` block that emits `WorktreeHeader` entries, in the for-loop added by Plan A)

- [ ] **Step 1: Add a helper for the canonical group path list string**

Above `rebuild_contents`, near the Plan A `primary_worktree_path_for_thread` helper, add:

```rust
/// Stable string form of a project group key's path list, used to scope
/// stored worktree ordering and cleanup to a specific group.
fn canonical_group_path_list(key: &ProjectGroupKey) -> String {
    // Reuse the serialized form already used by persistence so the on-disk
    // string matches the key's identity. Adapt if `PathList::serialize`
    // returns a non-string — e.g., convert via JSON / debug.
    let serialized = key.path_list().serialize();
    serde_json::to_string(&serialized).unwrap_or_else(|_| format!("{:?}", key.path_list()))
}

/// Remote connection identity string for a project group key, used for the
/// scoping prefix in stored ordering and overrides.
fn remote_connection_identity_for_group(key: &ProjectGroupKey) -> String {
    match key.host() {
        Some(host) => format!("remote:{host:?}"),
        None => "local".to_string(),
    }
}
```

**Note for executing engineer:** `ProjectGroupKey::path_list().serialize()` is the API used by `SerializedProjectGroup::from_group` (see `crates/workspace/src/persistence/model.rs`). If `serialize` is not `pub`, use the same string the persistence layer uses by inspecting how `SerializedPathList` is constructed in that file and replicating it here. The single requirement is: **the same project group key must produce the same canonical string every time**, across restarts.

Plan A's `WorktreeGroupOverrideKey` uses an identity prefix; **read what Plan A actually uses** for `remote_connection_identity` and re-use the *same* function so the two tables agree. Grep for `remote_connection_identity` to find the helper; it likely already exists in `thread_metadata_store.rs` — export and call it from `sidebar.rs` if so.

- [ ] **Step 2: Apply the stored order when emitting `WorktreeHeader` entries**

Find the section in `rebuild_contents` that iterates worktree paths to emit one `WorktreeHeader` per main path (the loop added by Plan A — currently `for worktree_path_arc in group_paths.paths() { ... }`). Replace the path iteration with an ordered iteration:

```rust
let group_paths = group_key.path_list();
let group_identity = remote_connection_identity_for_group(group_key);
let group_canonical = canonical_group_path_list(group_key);

let stored_order = store.worktree_order_for_group(&group_identity, &group_canonical);
let natural_order: Vec<PathBuf> = group_paths
    .paths()
    .iter()
    .map(|p| p.to_path_buf())
    .collect();

// Build the rendering order: stored paths first (in stored order), then any
// natural-order paths not present in stored (stable order).
let mut order: Vec<PathBuf> = stored_order
    .iter()
    .filter(|p| natural_order.contains(p))
    .cloned()
    .collect();
for p in &natural_order {
    if !order.contains(p) {
        order.push(p.clone());
    }
}

for worktree_path in &order {
    // (existing body that builds and pushes the WorktreeHeader entry,
    // unchanged — except it now iterates `order` instead of `group_paths.paths()`)
    // ...
}
```

Concretely, the body inside the loop is exactly the body Plan A wrote — moved verbatim. Only the iteration source changes.

- [ ] **Step 3: Build**

Run: `./script/clippy -p sidebar`
Expected: builds.

- [ ] **Step 4: Commit**

```bash
git add crates/sidebar/src/sidebar.rs
git commit -m "sidebar: Apply stored worktree order in rebuild_contents"
```

---

## Task 7: Add drag/drop state types to the sidebar

**Files:**
- Modify: `crates/sidebar/src/sidebar.rs` — types near the top of the file, fields on `Sidebar`, init in `Sidebar::new`

- [ ] **Step 1: Add the drag-value enum, drop-target indicator, and view type**

Insert near the top of `sidebar.rs`, after the existing top-level types (search for `enum ListEntry`):

```rust
/// Re-export the DropEdge from multi_workspace for use in sidebar code.
pub use workspace::multi_workspace::DropEdge;

/// The data carried by a drag-in-progress over the sidebar.
#[derive(Clone, Debug)]
enum DraggedSidebarHeader {
    Project(ProjectGroupKey),
    Worktree {
        project_group_key: ProjectGroupKey,
        worktree_path: PathBuf,
    },
}

/// While a drag is hovering over a header, this identifies the target and
/// the edge at which the drop indicator should be drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
enum DropTargetIndicator {
    Project {
        target_key: ProjectGroupKey,
        edge: DropEdge,
    },
    Worktree {
        project_group_key: ProjectGroupKey,
        target_path: PathBuf,
        edge: DropEdge,
    },
}

/// Rendered under the cursor while a header is being dragged.
struct DraggedHeaderView {
    label: SharedString,
    width: Pixels,
}

impl Render for DraggedHeaderView {
    fn render(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let ui_font = ThemeSettings::get_global(cx).ui_font.family.clone();
        h_flex()
            .font_family(ui_font)
            .bg(cx.theme().colors().elevated_surface_background)
            .border_1()
            .border_color(cx.theme().colors().border)
            .rounded_md()
            .shadow_md()
            .px_2()
            .py_1()
            .w(self.width)
            .child(Label::new(self.label.clone()).size(LabelSize::Small))
    }
}
```

If `ThemeSettings`, `h_flex`, `Label`, `Pixels` are not already imported into `sidebar.rs`, add the imports — they are widely used in this file so most should already exist. Check the existing import block to confirm.

- [ ] **Step 2: Add state fields to `Sidebar`**

Find `pub struct Sidebar { ... }` and add (next to other transient UI state):

```rust
    /// Set while a drag-in-progress is hovering over a header. Cleared on
    /// drop, drag-leave, or rebuild.
    drop_target: Option<DropTargetIndicator>,
```

- [ ] **Step 3: Initialize the field in `Sidebar::new`**

In the `Self { ... }` literal inside `Sidebar::new`, add:

```rust
    drop_target: None,
```

- [ ] **Step 4: Add a clearing helper**

In `impl Sidebar`, near `update_entries`:

```rust
fn clear_drop_target(&mut self, cx: &mut Context<Self>) {
    if self.drop_target.is_some() {
        self.drop_target = None;
        cx.notify();
    }
}
```

- [ ] **Step 5: Clear drop target on rebuild**

In `update_entries` (after the existing body that runs `rebuild_contents` and the cleanup), append:

```rust
// A drag may have been silently cancelled (e.g., the dragged item is no
// longer visible). Drop the indicator on every rebuild to keep state
// honest; on_drag_move will re-set it within the next frame if still active.
self.drop_target = None;
```

- [ ] **Step 6: Build**

Run: `./script/clippy -p sidebar`
Expected: builds.

- [ ] **Step 7: Commit**

```bash
git add crates/sidebar/src/sidebar.rs
git commit -m "sidebar: Add drag/drop state types for reorderable headers"
```

---

## Task 8: Add `on_drag` to the project header (drag source)

**Files:**
- Modify: `crates/sidebar/src/sidebar.rs` — `render_project_header`

- [ ] **Step 1: Wrap the rendered header element**

Inside `render_project_header`, locate the root element that the function builds and returns (typically an `h_flex().id(...)...` chain). Just before `.into_any_element()`, add:

```rust
.on_drag(
    DraggedSidebarHeader::Project(key.clone()),
    {
        let label = label.clone();
        let width = self.width;
        move |_dragged, _click_offset, _window, cx| {
            cx.new(|_| DraggedHeaderView {
                label: label.clone(),
                width,
            })
        }
    },
)
```

- [ ] **Step 2: Build and run**

Run: `./script/clippy -p sidebar`
Expected: builds.

Run: `cargo run --bin zed`. Open a project with multiple project groups. Click a project header label and drag — confirm a small floating label preview tracks the cursor. (No reorder yet; that lands in Task 9.)

- [ ] **Step 3: Commit**

```bash
git add crates/sidebar/src/sidebar.rs
git commit -m "sidebar: Make project header a drag source"
```

---

## Task 9: Add `on_drag_move` + `on_drop` to the project header (reorder)

**Files:**
- Modify: `crates/sidebar/src/sidebar.rs` — `render_project_header`, new private methods

- [ ] **Step 1: Add `compute_drop_edge` helper**

In `impl Sidebar`, near other helpers, add:

```rust
fn compute_drop_edge<T: 'static>(event: &gpui::DragMoveEvent<T>) -> DropEdge {
    let local_y = event.event.position.y - event.bounds.origin.y;
    if local_y < event.bounds.size.height / 2.0 {
        DropEdge::Above
    } else {
        DropEdge::Below
    }
}
```

- [ ] **Step 2: Add the drop handler `on_project_drop`**

In `impl Sidebar`:

```rust
fn on_project_drop(
    &mut self,
    dragged: &DraggedSidebarHeader,
    target_key: &ProjectGroupKey,
    edge: DropEdge,
    cx: &mut Context<Self>,
) {
    let DraggedSidebarHeader::Project(from_key) = dragged else {
        // Worktree drops on project headers are rejected; user can target a
        // worktree subheader instead.
        return;
    };

    if let Some(mw) = self.multi_workspace.upgrade() {
        mw.update(cx, |mw, cx| {
            mw.reorder_project_groups(from_key, target_key, edge);
            mw.serialize(cx);
        });
    }
    self.drop_target = None;
    self.update_entries(cx);
}
```

- [ ] **Step 3: Wire `on_drag_move` and `on_drop` on the project header**

Continue editing the same element chain in `render_project_header`. After the `.on_drag(...)` block from Task 8, add:

```rust
.on_drag_move::<DraggedSidebarHeader>({
    let target_key = key.clone();
    cx.listener(move |this, event: &gpui::DragMoveEvent<DraggedSidebarHeader>, _window, cx| {
        // Only show an indicator for Project drags — Worktree drags onto a
        // project header are rejected at drop time, so showing an indicator
        // here would be misleading.
        let dragged = event.drag(cx);
        if !matches!(dragged, DraggedSidebarHeader::Project(_)) {
            if this.drop_target.is_some() {
                this.drop_target = None;
                cx.notify();
            }
            return;
        }
        let edge = Self::compute_drop_edge(event);
        let new_target = DropTargetIndicator::Project {
            target_key: target_key.clone(),
            edge,
        };
        if this.drop_target.as_ref() != Some(&new_target) {
            this.drop_target = Some(new_target);
            cx.notify();
        }
    })
})
.on_drop({
    let target_key = key.clone();
    cx.listener(move |this, dragged: &DraggedSidebarHeader, _window, cx| {
        // Use the *last-observed* edge from on_drag_move; if no move event
        // fired (e.g., instantaneous drop), default to Below the target.
        let edge = match &this.drop_target {
            Some(DropTargetIndicator::Project { target_key: t, edge })
                if t == &target_key =>
            {
                *edge
            }
            _ => DropEdge::Below,
        };
        this.on_project_drop(dragged, &target_key, edge, cx);
    })
})
```

- [ ] **Step 4: Build**

Run: `./script/clippy -p sidebar`
Expected: builds.

- [ ] **Step 5: Manual verification**

Run: `cargo run --bin zed`. Open a workspace with at least 3 project groups. Drag a project header onto another and release — confirm the dragged group ends up adjacent to the drop target. Restart Zed and confirm the new order persists.

(The drop indicator bar is not yet visible — that lands in Task 12.)

- [ ] **Step 6: Commit**

```bash
git add crates/sidebar/src/sidebar.rs
git commit -m "sidebar: Reorder project groups on drag-and-drop with persistence"
```

---

## Task 10: Add `on_drag` to the worktree header (drag source)

**Files:**
- Modify: `crates/sidebar/src/sidebar.rs` — `render_worktree_header`

- [ ] **Step 1: Wrap the rendered header element**

Inside `render_worktree_header`, locate the root header element (`header_row` in the Plan A code). Just before `.into_any_element()`, add:

```rust
.on_drag(
    DraggedSidebarHeader::Worktree {
        project_group_key: project_group_key.clone(),
        worktree_path: worktree_path.to_path_buf(),
    },
    {
        let label = display_name.clone();
        let width = self.width;
        move |_dragged, _click_offset, _window, cx| {
            cx.new(|_| DraggedHeaderView {
                label: label.clone(),
                width,
            })
        }
    },
)
```

- [ ] **Step 2: Build and run**

Run: `./script/clippy -p sidebar`
Expected: builds.

Run: `cargo run --bin zed`. Drag a worktree subheader — confirm the floating preview appears. (No reorder yet.)

- [ ] **Step 3: Commit**

```bash
git add crates/sidebar/src/sidebar.rs
git commit -m "sidebar: Make worktree header a drag source"
```

---

## Task 11: Add `on_drag_move` + `on_drop` to the worktree header (within-group reorder)

**Files:**
- Modify: `crates/sidebar/src/sidebar.rs` — `render_worktree_header`, new method

- [ ] **Step 1: Add the drop handler `on_worktree_drop`**

In `impl Sidebar`:

```rust
fn on_worktree_drop(
    &mut self,
    dragged: &DraggedSidebarHeader,
    target_group_key: &ProjectGroupKey,
    target_path: &Path,
    edge: DropEdge,
    cx: &mut Context<Self>,
) {
    let DraggedSidebarHeader::Worktree {
        project_group_key: from_group_key,
        worktree_path: from_path,
    } = dragged
    else {
        // Project-on-worktree drops are silently ignored.
        return;
    };

    // Only allow reordering within the same project group.
    if from_group_key != target_group_key {
        self.drop_target = None;
        cx.notify();
        return;
    }
    if from_path == target_path {
        self.drop_target = None;
        cx.notify();
        return;
    }

    // Compute the current effective order for this group (matches what
    // rebuild_contents would render).
    let group_identity = remote_connection_identity_for_group(target_group_key);
    let group_canonical = canonical_group_path_list(target_group_key);
    let store_handle = ThreadMetadataStore::global(cx);
    let stored = store_handle
        .read(cx)
        .worktree_order_for_group(&group_identity, &group_canonical);
    let natural: Vec<PathBuf> = target_group_key
        .path_list()
        .paths()
        .iter()
        .map(|p| p.to_path_buf())
        .collect();
    let mut effective: Vec<PathBuf> = stored
        .iter()
        .filter(|p| natural.contains(p))
        .cloned()
        .collect();
    for p in &natural {
        if !effective.contains(p) {
            effective.push(p.clone());
        }
    }

    // Move `from_path` to its new position relative to `target_path`.
    let Some(from_ix) = effective.iter().position(|p| p == from_path) else {
        return;
    };
    let moved = effective.remove(from_ix);
    let Some(to_ix_after_remove) = effective.iter().position(|p| p == target_path) else {
        return;
    };
    let insert_at = match edge {
        DropEdge::Above => to_ix_after_remove,
        DropEdge::Below => to_ix_after_remove + 1,
    };
    let insert_at = insert_at.min(effective.len());
    effective.insert(insert_at, moved);

    store_handle.update(cx, |store, cx| {
        store.set_worktree_order_for_group(
            group_identity,
            group_canonical,
            effective,
            cx,
        );
    });

    self.drop_target = None;
    self.update_entries(cx);
}
```

- [ ] **Step 2: Wire `on_drag_move` and `on_drop` on the worktree header**

Continue editing the element chain in `render_worktree_header`. After the `.on_drag(...)` from Task 10, add:

```rust
.on_drag_move::<DraggedSidebarHeader>({
    let group_key = project_group_key.clone();
    let target_path = worktree_path.to_path_buf();
    cx.listener(move |this, event: &gpui::DragMoveEvent<DraggedSidebarHeader>, _window, cx| {
        // Reject cross-group hover early: don't show an indicator on
        // worktree headers belonging to a different project group.
        let dragged = event.drag(cx);
        if let DraggedSidebarHeader::Worktree { project_group_key: from_group_key, .. } = dragged
            && from_group_key != &group_key
        {
            if this.drop_target.is_some() {
                this.drop_target = None;
                cx.notify();
            }
            return;
        }
        // Don't show an indicator if a Project drag is hovering — projects
        // can only drop on other project headers.
        if matches!(dragged, DraggedSidebarHeader::Project(_)) {
            if this.drop_target.is_some() {
                this.drop_target = None;
                cx.notify();
            }
            return;
        }
        let edge = Self::compute_drop_edge(event);
        let new_target = DropTargetIndicator::Worktree {
            project_group_key: group_key.clone(),
            target_path: target_path.clone(),
            edge,
        };
        if this.drop_target.as_ref() != Some(&new_target) {
            this.drop_target = Some(new_target);
            cx.notify();
        }
    })
})
.on_drop({
    let group_key = project_group_key.clone();
    let target_path = worktree_path.to_path_buf();
    cx.listener(move |this, dragged: &DraggedSidebarHeader, _window, cx| {
        let edge = match &this.drop_target {
            Some(DropTargetIndicator::Worktree {
                project_group_key: g,
                target_path: t,
                edge,
            }) if g == &group_key && t == &target_path => *edge,
            _ => DropEdge::Below,
        };
        this.on_worktree_drop(dragged, &group_key, &target_path, edge, cx);
    })
})
```

Note: `event.drag(cx)` is documented in `crates/gpui/src/elements/div.rs` on `DragMoveEvent::drag`. It returns `&T` where `T` is the drag-value type.

- [ ] **Step 3: Build**

Run: `./script/clippy -p sidebar`
Expected: builds.

- [ ] **Step 4: Manual verification**

Run: `cargo run --bin zed`. Open a multi-root workspace (two or more main worktree paths in a single project group). Drag worktree headers within the group and confirm:

1. Drop within the same group reorders.
2. Drop on a worktree subheader belonging to a different project group is rejected (no change, no error).
3. Restart Zed — confirm worktree order persists.

- [ ] **Step 5: Commit**

```bash
git add crates/sidebar/src/sidebar.rs
git commit -m "sidebar: Reorder worktree subheaders within their project group"
```

---

## Task 12: Render the drop-indicator bar

**Files:**
- Modify: `crates/sidebar/src/sidebar.rs` — `render_project_header`, `render_worktree_header`

- [ ] **Step 1: Add a helper that renders the indicator element**

In `impl Sidebar`, near `render_worktree_header`:

```rust
fn render_drop_indicator_bar(edge: DropEdge, cx: &App) -> AnyElement {
    let color = cx.theme().colors().drop_target_background;
    let mut bar = div()
        .absolute()
        .left_0()
        .right_0()
        .h(px(2.0))
        .bg(color);
    bar = match edge {
        DropEdge::Above => bar.top_0(),
        DropEdge::Below => bar.bottom_0(),
    };
    bar.into_any_element()
}
```

- [ ] **Step 2: Show the bar on the active project drop target**

In `render_project_header`, after constructing the header row but before applying drag/drop handlers from Tasks 8-9, wrap the row in a relatively-positioned container that conditionally renders the bar:

```rust
let drop_indicator_edge = match &self.drop_target {
    Some(DropTargetIndicator::Project { target_key, edge }) if target_key == key => {
        Some(*edge)
    }
    _ => None,
};

let header_root = div()
    .relative()
    .child(header_row /* the existing built h_flex */ )
    .when_some(drop_indicator_edge, |this, edge| {
        this.child(Self::render_drop_indicator_bar(edge, cx))
    });
```

Apply the `.on_drag(...).on_drag_move(...).on_drop(...)` handlers from Tasks 8-9 to `header_root` instead of the inner `header_row` — drop events should fire on the wrapping element so the indicator bar is included in the hit box.

- [ ] **Step 3: Show the bar on the active worktree drop target**

Same pattern in `render_worktree_header`:

```rust
let drop_indicator_edge = match &self.drop_target {
    Some(DropTargetIndicator::Worktree {
        project_group_key: g,
        target_path,
        edge,
    }) if g == project_group_key && target_path.as_path() == worktree_path => Some(*edge),
    _ => None,
};

let header_root = div()
    .relative()
    .child(header_row)
    .when_some(drop_indicator_edge, |this, edge| {
        this.child(Self::render_drop_indicator_bar(edge, cx))
    });
```

Move the drag/drop handlers from `header_row` to `header_root` (parallel to Task 11.2).

- [ ] **Step 4: Build and run**

Run: `./script/clippy -p sidebar`
Expected: builds.

Run: `cargo run --bin zed`. Drag a header over another and confirm:
1. A 2px-tall horizontal bar appears at the top or bottom of the hovered target depending on cursor position.
2. The bar moves between top/bottom as the cursor crosses the vertical midpoint of the target.
3. On drop, the bar disappears immediately and the headers are in the expected new order.
4. On drag-cancel (Esc / out of window), the bar disappears.

- [ ] **Step 5: Commit**

```bash
git add crates/sidebar/src/sidebar.rs
git commit -m "sidebar: Render horizontal drop indicator on active drag targets"
```

---

## Task 13: Test — project header reorder persists

**Files:**
- Modify: `crates/sidebar/src/sidebar_tests.rs`

- [ ] **Step 1: Add a helper for collecting project header ordering**

If a helper like `visible_entries_as_strings` exists (Plan A's tests added one), extend it to mark project headers. Add a focused helper to assert the project order:

```rust
fn project_header_keys(sidebar: &Entity<Sidebar>, cx: &mut VisualTestContext) -> Vec<ProjectGroupKey> {
    sidebar.update(cx, |sidebar, _cx| {
        sidebar
            .contents
            .entries
            .iter()
            .filter_map(|entry| match entry {
                ListEntry::ProjectHeader { key, .. } => Some(key.clone()),
                _ => None,
            })
            .collect()
    })
}
```

(`contents` and `entries` are `pub(crate)`; if not, change visibility or add a test accessor.)

- [ ] **Step 2: Write the test**

```rust
#[gpui::test]
async fn test_project_header_reorder_persists(cx: &mut TestAppContext) {
    init_test(cx);

    let project_a = init_test_project("/proj-a", cx).await;
    let project_b = init_test_project("/proj-b", cx).await;

    // Open both in a single MultiWorkspace.
    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project_a.clone(), window, cx));
    multi_workspace.update(cx, |mw, cx| {
        mw.add_project(project_b.clone(), cx);
    });
    let sidebar = setup_sidebar(&multi_workspace, cx);

    save_thread_metadata(
        acp::SessionId::new(Arc::from("a")),
        Some("A".into()),
        Utc::now(),
        None,
        None,
        &project_a,
        cx,
    );
    save_thread_metadata(
        acp::SessionId::new(Arc::from("b")),
        Some("B".into()),
        Utc::now(),
        None,
        None,
        &project_b,
        cx,
    );
    cx.run_until_parked();

    let initial = project_header_keys(&sidebar, cx);
    assert_eq!(initial.len(), 2, "expected two project headers");
    let (a_key, b_key) = (initial[0].clone(), initial[1].clone());

    // Simulate the user dropping a above b (i.e., a is already above; verify
    // we can move it below).
    multi_workspace.update(cx, |mw, cx| {
        mw.reorder_project_groups(&a_key, &b_key, DropEdge::Below);
        mw.serialize(cx);
    });
    sidebar.update(cx, |s, cx| s.update_entries(cx));
    cx.run_until_parked();

    let after = project_header_keys(&sidebar, cx);
    assert_eq!(after, vec![b_key.clone(), a_key.clone()]);
}
```

If `MultiWorkspace::test_new` does not exist for adding multiple projects, fall back to creating the second project via the existing test factories (search `sidebar_tests.rs` for how Plan A's tests handle multi-project setup, if any). If none exists, write a minimal `multi_workspace_with_two_projects` helper in this file modeled after `init_test_project`.

- [ ] **Step 3: Run the test**

Run: `cargo test -p sidebar test_project_header_reorder_persists`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/sidebar/src/sidebar_tests.rs
git commit -m "sidebar: Test project header reorder via MultiWorkspace"
```

---

## Task 14: Test — worktree header reorder within a group

**Files:**
- Modify: `crates/sidebar/src/sidebar_tests.rs`

- [ ] **Step 1: Add a helper for collecting worktree header order**

Append:

```rust
fn worktree_header_paths(
    sidebar: &Entity<Sidebar>,
    cx: &mut VisualTestContext,
    group_key: &ProjectGroupKey,
) -> Vec<PathBuf> {
    sidebar.update(cx, |sidebar, _cx| {
        sidebar
            .contents
            .entries
            .iter()
            .filter_map(|entry| match entry {
                ListEntry::WorktreeHeader { project_group_key, worktree_path, .. }
                    if project_group_key == group_key =>
                {
                    Some(worktree_path.clone())
                }
                _ => None,
            })
            .collect()
    })
}
```

- [ ] **Step 2: Write the test**

```rust
#[gpui::test]
async fn test_worktree_reorder_within_group(cx: &mut TestAppContext) {
    init_test(cx);
    // Two main worktrees in one project group.
    let project = init_test_project_multi(&["/group-root/a", "/group-root/b"], cx).await;
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

    let group_key = sidebar.update(cx, |sidebar, _cx| {
        sidebar
            .contents
            .entries
            .iter()
            .find_map(|entry| match entry {
                ListEntry::ProjectHeader { key, .. } => Some(key.clone()),
                _ => None,
            })
            .expect("project header should exist")
    });

    let initial = worktree_header_paths(&sidebar, cx, &group_key);
    assert_eq!(
        initial,
        vec![PathBuf::from("/group-root/a"), PathBuf::from("/group-root/b")]
    );

    // Reorder: put /group-root/b first.
    let identity = remote_connection_identity_for_group(&group_key);
    let canonical = canonical_group_path_list(&group_key);
    cx.update(|cx| {
        ThreadMetadataStore::global(cx).update(cx, |store, cx| {
            store.set_worktree_order_for_group(
                identity,
                canonical,
                vec![PathBuf::from("/group-root/b"), PathBuf::from("/group-root/a")],
                cx,
            );
        });
    });
    sidebar.update(cx, |s, cx| s.update_entries(cx));
    cx.run_until_parked();

    let after = worktree_header_paths(&sidebar, cx, &group_key);
    assert_eq!(
        after,
        vec![PathBuf::from("/group-root/b"), PathBuf::from("/group-root/a")]
    );
}
```

Both `remote_connection_identity_for_group` and `canonical_group_path_list` are the helpers added in Task 6; either make them `pub(crate)` for tests or add `#[cfg(test)] pub(crate)` exports.

- [ ] **Step 3: Run the test**

Run: `cargo test -p sidebar test_worktree_reorder_within_group`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/sidebar/src/sidebar_tests.rs
git commit -m "sidebar: Test worktree reorder within project group"
```

---

## Task 15: Test — worktree drop rejected across project groups

**Files:**
- Modify: `crates/sidebar/src/sidebar_tests.rs`

- [ ] **Step 1: Write the test**

```rust
#[gpui::test]
async fn test_worktree_drop_rejected_across_groups(cx: &mut TestAppContext) {
    init_test(cx);
    let project_a = init_test_project_multi(&["/group-a/x", "/group-a/y"], cx).await;
    let project_b = init_test_project_multi(&["/group-b/p"], cx).await;
    let (multi_workspace, cx) =
        cx.add_window_view(|window, cx| MultiWorkspace::test_new(project_a.clone(), window, cx));
    multi_workspace.update(cx, |mw, cx| {
        mw.add_project(project_b.clone(), cx);
    });
    let sidebar = setup_sidebar(&multi_workspace, cx);

    save_thread_metadata(
        acp::SessionId::new(Arc::from("a")),
        Some("Thread A".into()),
        Utc::now(),
        None,
        None,
        &project_a,
        cx,
    );
    save_thread_metadata(
        acp::SessionId::new(Arc::from("b")),
        Some("Thread B".into()),
        Utc::now(),
        None,
        None,
        &project_b,
        cx,
    );
    cx.run_until_parked();

    let group_keys: Vec<ProjectGroupKey> = sidebar.update(cx, |sidebar, _cx| {
        sidebar
            .contents
            .entries
            .iter()
            .filter_map(|entry| match entry {
                ListEntry::ProjectHeader { key, .. } => Some(key.clone()),
                _ => None,
            })
            .collect()
    });
    assert_eq!(group_keys.len(), 2);
    let group_a = group_keys[0].clone();
    let group_b = group_keys[1].clone();

    let before_a = worktree_header_paths(&sidebar, cx, &group_a);

    // Simulate a worktree-on-worktree drop from group_a's first path onto a
    // path in group_b. Drive the handler directly because we don't have a
    // mouse-event simulator here.
    sidebar.update(cx, |sidebar, cx| {
        let dragged = DraggedSidebarHeader::Worktree {
            project_group_key: group_a.clone(),
            worktree_path: before_a[0].clone(),
        };
        let target_path = PathBuf::from("/group-b/p");
        sidebar.on_worktree_drop(
            &dragged,
            &group_b,
            &target_path,
            DropEdge::Above,
            cx,
        );
    });
    sidebar.update(cx, |s, cx| s.update_entries(cx));
    cx.run_until_parked();

    let after_a = worktree_header_paths(&sidebar, cx, &group_a);
    assert_eq!(
        after_a, before_a,
        "worktree order in source group should be unchanged after cross-group drop"
    );
    let after_b = worktree_header_paths(&sidebar, cx, &group_b);
    assert_eq!(
        after_b,
        vec![PathBuf::from("/group-b/p")],
        "target group should still have only its own worktrees"
    );
}
```

`on_worktree_drop` is currently private. Either:
- Change visibility to `pub(crate)`.
- Or expose a thin test-only wrapper:

  ```rust
  #[cfg(test)]
  pub(crate) fn on_worktree_drop_for_test(
      &mut self,
      dragged: &DraggedSidebarHeader,
      target_group_key: &ProjectGroupKey,
      target_path: &Path,
      edge: DropEdge,
      cx: &mut Context<Self>,
  ) {
      self.on_worktree_drop(dragged, target_group_key, target_path, edge, cx);
  }
  ```

Use whichever approach matches existing test-accessor patterns in this file (search `#[cfg(test)] pub(crate) fn` in `sidebar.rs`).

- [ ] **Step 2: Run the test**

Run: `cargo test -p sidebar test_worktree_drop_rejected_across_groups`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/sidebar/src/sidebar.rs crates/sidebar/src/sidebar_tests.rs
git commit -m "sidebar: Test cross-group worktree drops are rejected"
```

---

## Task 16: Manual smoke check + final clippy

- [ ] **Step 1: Run full clippy**

Run: `./script/clippy`
Expected: zero warnings/errors in `crates/sidebar`, `crates/agent_ui`, `crates/workspace`.

- [ ] **Step 2: Run full test suites for the affected crates**

Run: `cargo test -p sidebar -p agent_ui -p workspace`
Expected: all tests pass.

- [ ] **Step 3: Manual UI smoke**

Run: `cargo run --bin zed`. Verify:

1. **Project reorder:** open a workspace with at least 3 project groups. Drag the middle project header above the first; confirm the order changes and a 2px bar shows the insertion point during the drag.
2. **Restart persistence (project):** close Zed and reopen. The new project order should persist.
3. **Worktree reorder within a group:** open a multi-root workspace (two main worktrees in one project group). Drag the second worktree header above the first; confirm the order changes and the drop indicator appears.
4. **Restart persistence (worktree):** close Zed, reopen, confirm worktree order persists.
5. **Cross-group rejection:** with two project groups each containing multiple worktrees, drag a worktree header from group A onto a worktree header of group B. Confirm: no drop indicator appears while hovering, and on release, no change happens.
6. **Drag preview:** the floating preview under the cursor shows the header's display name and has the same width as the sidebar.
7. **DB inspection (optional):**
   ```bash
   sqlite3 ~/Library/Application\ Support/Zed/db/0-*.sqlite "SELECT * FROM worktree_group_order ORDER BY remote_connection_identity, group_path_list, position"
   ```
   Confirm rows reflect the order you set in the UI.

- [ ] **Step 4: Commit any final cleanups**

If clippy or tests required tweaks, commit them:

```bash
git add -p
git commit -m "sidebar: Cleanups for reorderable headers"
```

---

## Notes for the executing engineer

- **`on_drag_move` only fires for the matching drag-value type.** GPUI's `on_drag_move::<T>` checks `cx.active_drag.value.type_id() == TypeId::of::<T>()` before invoking the listener (see `crates/gpui/src/elements/div.rs:308-339`). This means a `DraggedSidebarHeader` drag automatically won't trigger drag-move on, say, a file-drag handler — but it's our responsibility to filter by variant (project-vs-worktree) inside the listener.
- **`DragMoveEvent::drag(cx)` is the canonical way** to access the drag's value from inside the listener. The `event.dragged_item()` returns `&dyn Any` and needs a downcast; `event.drag(cx)` already returns `&DraggedSidebarHeader`.
- **`on_drop` is called even if `on_drag_move` never fired on that target.** Always handle the "no drop_target observed for this element" case by either defaulting to a sensible edge (we use `DropEdge::Below`) or short-circuiting.
- **`drop_target_background` is a real theme color** — see `crates/project_panel/src/project_panel.rs:581` where the project panel uses it. If your theme doesn't define it, fall back to `cx.theme().colors().element_selection_background` or `border_focused` for a comparable contrast.
- **`PathList::serialize()` location:** it lives on `crates/project/src/project.rs` (or wherever `PathList` is defined). If the method is not `pub`, you have two options: (a) widen its visibility via a small PR — the persistence layer at `crates/workspace/src/persistence/model.rs:66-87` already calls it; (b) replicate the serialization inline. Prefer (a) — keep one source of truth.
- **The persisted order for project groups goes through the existing `MultiWorkspace` serialization** (`SerializedProjectGroup` `Vec` order). You don't need to write any new persistence code for project header order — it's a side-effect of `serialize(cx)` after the in-memory vec mutation.
- **Drag-cancellation:** GPUI clears `cx.active_drag` on Esc / window-leave; on the next rebuild we drop `drop_target`. If users report a "ghost" drop indicator, audit whether `update_entries` is being called on drag-cancel paths.
- **Sticky header complication:** sidebar.rs renders a sticky project header (around `sidebar.rs:2751-2846`). Confirm the sticky variant doesn't double-emit drag handlers — easiest is to check `is_sticky` in `render_project_header` and only attach the drag-and-drop wiring when `!is_sticky`. Otherwise the sticky copy may capture drops with stale state.
- **Single-worktree projects:** dragging a worktree header in a group that only contains one path is a no-op (no target to drop onto). The drop indicator should still render correctly because the UI logic doesn't depend on group size — just don't add UX polish here, since users have nothing to reorder.
- **Multi-occurrence paths:** if the same absolute path appears in two project groups' `path_list()`, our table keys position by `(group_path_list_canonical, worktree_path)`, so each group's ordering is independent. Verify by inspecting `worktree_group_order` after reordering in only one group.
- **Why the canonical group string lives in the SQL key:** if a project group's `path_list` changes (user opens a worktree, closes another), it becomes a "new" key. The order rows for the old key become orphans — Task 16's manual check + the cleanup method (`cleanup_worktree_order_not_in_groups`) handles this. Note: the cleanup method needs to be *invoked* — wire that call into `update_entries` alongside the Plan A `cleanup_worktree_overrides_not_in` call. **If you don't see worktree-order rows getting cleaned up in the DB after closing a project, that's the missing wire-up.**
