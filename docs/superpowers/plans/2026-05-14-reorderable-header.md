# Reorderable Header Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add drag-and-drop reordering for project headers and worktree subheaders in the sidebar, with order persisted across restarts and visible drop indicators during a drag.

**Architecture:** Two new SQLite tables in the `ThreadMetadataStore` database (`project_group_orders` and `worktree_orders`) store a per-entry `sort_index INTEGER`. The store loads them into in-memory caches at startup, like the existing `worktree_group_overrides`. The sidebar's `rebuild_contents()` consults these caches and stably sorts project groups and worktrees by sort_index before pushing entries. Drag-and-drop wiring on each header uses GPUI's `on_drag` / `drag_over` / `on_drop` APIs (already used by `collab_panel`). A `Sidebar::active_drag: Option<ActiveDrag>` field tracks the in-flight drag and the prospective drop slot, so a 2px colored indicator can render reactively between entries.

**Tech Stack:** Rust, GPUI (entities, drag/drop, async tasks, list rendering), SQLite via Zed's `Statement` helper, the existing `ThreadMetadataStore` global.

---

## File Structure

This plan modifies two existing files. No new files are introduced — the changes fit naturally into existing modules and following the convention in the project of preferring fewer, focused files.

- `crates/agent_ui/src/thread_metadata_store.rs` — schema migrations, two new in-memory caches, public APIs for reading/writing sort indices, async DB persistence following the existing `worktree_group_overrides` pattern.
- `crates/sidebar/src/sidebar.rs` — sort project iteration and worktree iteration in `rebuild_contents()` by persisted order, define drag payload types, track in-flight drag state, render drop indicators between entries, attach drag/drop handlers to `render_project_header()` and `render_worktree_header()`.
- `crates/sidebar/src/sidebar_tests.rs` — integration tests verifying that reordering is applied to rendered entries and persists across a fresh sidebar.

---

## Conventions used throughout this plan

- **Test command (whole workspace touched here):** `cargo test -p agent_ui` and `cargo test -p sidebar`.
- **Lint command:** `./script/clippy` (per `CLAUDE.md` — do not use `cargo clippy`).
- **Commit policy:** Per project `CLAUDE.md`, only commit when explicitly authorized. Each step labeled "Commit" represents a logical commit boundary; the executing agent must surface them to the user but not commit unprompted. Treat the commit *messages* as the executing agent's suggestion to the user.
- **Sort policy:** Entries with an explicit `sort_index` come first, ascending. Entries without one keep their natural rebuild order and follow afterward. Stability is preserved via Rust's `sort_by_key` (which is stable).
- **Sort index normalization:** On every reorder we rewrite the full ordered list with sort_index = `0, 1, 2, ...`. This keeps reasoning trivial. Projects and worktrees per project are typically under ~50 entries, so the I/O cost is negligible.
- **Orphan rows:** A worktree or project that disappears from disk leaves its row in the DB. We do not garbage-collect in this plan; documented as a follow-up.

---

## Task 1: Persistence layer for worktree sort order

### Files

- Modify: `crates/agent_ui/src/thread_metadata_store.rs`
- Test: `crates/agent_ui/src/thread_metadata_store.rs` (in the existing `mod tests` at line 2193)

### Steps

- [ ] **Step 1.1: Write failing test for round-trip of worktree sort indices**

Append this test inside `mod tests { ... }` near the other `worktree_overrides` tests (around line 2520, after `test_store_initializes_cache_from_database`):

```rust
#[gpui::test]
async fn test_worktree_orders_round_trip_through_database(cx: &mut TestAppContext) {
    init_test(cx);
    let db_path = tempfile::NamedTempFile::new().unwrap().into_temp_path();
    let store = cx.update(|cx| {
        ThreadMetadataStore::new_with_database_path(db_path.to_path_buf(), cx)
    });
    cx.run_until_parked();

    let path_a: PathBuf = "/projects/repo/worktree_a".into();
    let path_b: PathBuf = "/projects/repo/worktree_b".into();
    let path_c: PathBuf = "/projects/repo/worktree_c".into();
    let ordered = vec![path_c.clone(), path_a.clone(), path_b.clone()];

    store.update(cx, |store, cx| {
        store.set_worktree_sort_order(None, ordered.clone(), cx);
    });
    cx.run_until_parked();

    // Drop and reload from the same DB file.
    drop(store);
    let store = cx.update(|cx| {
        ThreadMetadataStore::new_with_database_path(db_path.to_path_buf(), cx)
    });
    cx.run_until_parked();

    store.read_with(cx, |store, _| {
        assert_eq!(store.worktree_sort_index(None, &path_c), Some(0));
        assert_eq!(store.worktree_sort_index(None, &path_a), Some(1));
        assert_eq!(store.worktree_sort_index(None, &path_b), Some(2));
        assert_eq!(
            store.worktree_sort_index(None, &PathBuf::from("/nope")),
            None
        );
    });
}
```

- [ ] **Step 1.2: Run test, confirm it fails**

Run: `cargo test -p agent_ui test_worktree_orders_round_trip_through_database`

Expected: compile error — `set_worktree_sort_order` and `worktree_sort_index` do not exist yet.

- [ ] **Step 1.3: Add schema for the new `worktree_orders` table**

In `thread_metadata_store.rs`, find the existing `CREATE TABLE IF NOT EXISTS worktree_group_overrides(...)` statement (around line 1686). Immediately after it, in the same migration block, add:

```rust
conn.exec(
    "CREATE TABLE IF NOT EXISTS worktree_orders(
        remote_connection_identity TEXT NOT NULL,
        worktree_path TEXT NOT NULL,
        sort_index INTEGER NOT NULL,
        PRIMARY KEY(remote_connection_identity, worktree_path)
    ) STRICT",
)?()?;
```

Migrations run on every store init, so `IF NOT EXISTS` is idempotent and safe — no version bump required for this additive table.

- [ ] **Step 1.4: Add the in-memory cache field on `ThreadMetadataStore`**

Find the struct declaration of `ThreadMetadataStore` (around line 522, where `worktree_overrides: HashMap<WorktreeGroupOverrideKey, WorktreeGroupOverride>` is). Add two new fields directly below it:

```rust
worktree_sort_indices: HashMap<WorktreeGroupOverrideKey, i64>,
worktree_sort_indices_loaded: bool,
dirty_worktree_sort_keys: HashSet<WorktreeGroupOverrideKey>,
```

In the constructor (search for `worktree_overrides: HashMap::new()`), initialize them analogously:

```rust
worktree_sort_indices: HashMap::new(),
worktree_sort_indices_loaded: false,
dirty_worktree_sort_keys: HashSet::new(),
```

Reuse the existing `WorktreeGroupOverrideKey` type — its `(path, remote_connection_identity)` shape is exactly what we need.

- [ ] **Step 1.5: Add the DB load function**

In the same file near `load_worktree_overrides` (around line 1735), add:

```rust
async fn load_worktree_orders(&self) -> anyhow::Result<Vec<WorktreeOrderRow>> {
    self.write(move |conn| {
        conn.select::<WorktreeOrderRow>(
            "SELECT remote_connection_identity, worktree_path, sort_index \
             FROM worktree_orders \
             ORDER BY remote_connection_identity, worktree_path",
        )?()
    })
    .await
}
```

And the row struct near where `WorktreeOverrideRow` is declared:

```rust
#[derive(Debug, Clone)]
struct WorktreeOrderRow {
    key: WorktreeGroupOverrideKey,
    sort_index: i64,
}

impl sqlez::bindable::Column for WorktreeOrderRow {
    fn column(statement: &mut Statement, start_index: i32) -> anyhow::Result<(Self, i32)> {
        let (remote, next) = String::column(statement, start_index)?;
        let (path, next) = String::column(statement, next)?;
        let (sort_index, next) = i64::column(statement, next)?;
        let key = WorktreeGroupOverrideKey::from_db_strings(remote, path)?;
        Ok((Self { key, sort_index }, next))
    }
}
```

(If a `from_db_strings` constructor doesn't already exist on `WorktreeGroupOverrideKey`, mirror the pattern used by `WorktreeOverrideRow::column` — typically `WorktreeGroupOverrideKey { path: PathBuf::from(path), remote_connection_identity: parse(...) }`.)

- [ ] **Step 1.6: Add the DB write function**

Below `upsert_worktree_override` (around line 1746), add the persisting upsert:

```rust
async fn upsert_worktree_sort_index(
    &self,
    key: WorktreeGroupOverrideKey,
    sort_index: i64,
) -> anyhow::Result<()> {
    let remote_connection_identity =
        Self::remote_connection_identity_to_db_string(key.remote_connection_identity.as_ref())?;
    let path = Self::path_to_db_string(key.path);
    self.write(move |conn| {
        let mut stmt = Statement::prepare(
            conn,
            "INSERT INTO worktree_orders(remote_connection_identity, worktree_path, sort_index) \
             VALUES (?1, ?2, ?3) \
             ON CONFLICT(remote_connection_identity, worktree_path) DO UPDATE SET \
                 sort_index = excluded.sort_index",
        )?;
        let mut i = stmt.bind(&remote_connection_identity, 1)?;
        i = stmt.bind(&path, i)?;
        stmt.bind(&sort_index, i)?;
        stmt.exec()
    })
    .await
}

async fn delete_worktree_sort_index(
    &self,
    key: WorktreeGroupOverrideKey,
) -> anyhow::Result<()> {
    let remote_connection_identity =
        Self::remote_connection_identity_to_db_string(key.remote_connection_identity.as_ref())?;
    let path = Self::path_to_db_string(key.path);
    self.write(move |conn| {
        let mut stmt = Statement::prepare(
            conn,
            "DELETE FROM worktree_orders \
             WHERE remote_connection_identity = ?1 AND worktree_path = ?2",
        )?;
        let mut i = stmt.bind(&remote_connection_identity, 1)?;
        stmt.bind(&path, i)?;
        stmt.exec()
    })
    .await
}
```

- [ ] **Step 1.7: Wire up load-at-startup**

Find the existing startup loader pattern (around line 1421, `load_worktree_overrides_task`). Directly below the block that processes worktree overrides, add the parallel task for sort indices:

```rust
let load_worktree_orders_task = cx.background_spawn({
    let db = db.clone();
    async move {
        db.load_worktree_orders()
            .await
            .context("Failed to fetch worktree sort orders")
    }
});
cx.spawn(async move |this, cx| {
    let Some(rows) = load_worktree_orders_task.await.log_err() else {
        return;
    };
    this.update(cx, |this, cx| {
        for row in rows {
            if this.dirty_worktree_sort_keys.contains(&row.key) {
                continue;
            }
            this.worktree_sort_indices.insert(row.key, row.sort_index);
        }
        this.worktree_sort_indices_loaded = true;
        cx.notify();
    })
    .ok();
})
.detach();
```

- [ ] **Step 1.8: Add the public `worktree_sort_index` read API**

Add this method to `impl ThreadMetadataStore` near the other read accessors (around the existing `is_worktree_collapsed` method, line ~830):

```rust
pub fn worktree_sort_index(
    &self,
    remote_connection: Option<&RemoteConnectionOptions>,
    path: &Path,
) -> Option<i64> {
    let key = WorktreeGroupOverrideKey::new(path.to_path_buf(), remote_connection);
    self.worktree_sort_indices.get(&key).copied()
}
```

- [ ] **Step 1.9: Add the public `set_worktree_sort_order` write API**

Add this method directly below `set_worktree_collapsed` (around line 814). It accepts the ordered list of paths and renumbers all of them, *and* removes any rows previously present for that remote that are no longer in the list (we will rewrite the whole order block for the affected scope each time):

```rust
pub fn set_worktree_sort_order(
    &mut self,
    remote_connection: Option<&RemoteConnectionOptions>,
    ordered_paths: Vec<PathBuf>,
    cx: &mut Context<Self>,
) {
    let remote_identity =
        WorktreeGroupOverrideKey::remote_identity(remote_connection);

    // Collect existing keys for this remote so we can delete ones that are no
    // longer present in the new order. Worktrees outside the affected scope
    // are untouched.
    let stale_keys: Vec<WorktreeGroupOverrideKey> = self
        .worktree_sort_indices
        .keys()
        .filter(|key| key.remote_connection_identity == remote_identity
            && !ordered_paths.iter().any(|p| p == &key.path))
        .cloned()
        .collect();

    for key in stale_keys {
        self.worktree_sort_indices.remove(&key);
        self.dirty_worktree_sort_keys.insert(key.clone());
        self.persist_worktree_sort_delete(key);
    }

    for (i, path) in ordered_paths.into_iter().enumerate() {
        let key = WorktreeGroupOverrideKey::new(path, remote_connection);
        let new_index = i as i64;
        if self.worktree_sort_indices_loaded
            && self.worktree_sort_indices.get(&key) == Some(&new_index)
        {
            continue;
        }
        self.worktree_sort_indices.insert(key.clone(), new_index);
        self.dirty_worktree_sort_keys.insert(key.clone());
        self.persist_worktree_sort_upsert(key, new_index);
    }

    cx.notify();
}
```

Note: the helper `WorktreeGroupOverrideKey::remote_identity` does not exist yet — extract whatever logic the existing `WorktreeGroupOverrideKey::new` uses to compute its `remote_connection_identity` field into a free associated function on the key type. If awkward, inline the snippet here instead.

The two `persist_worktree_sort_*` methods below dispatch a background task that calls the DB. They follow the pattern of `persist_worktree_override` (around line 1880):

```rust
fn persist_worktree_sort_upsert(
    &self,
    key: WorktreeGroupOverrideKey,
    sort_index: i64,
) {
    let db = self.db.clone();
    self.executor
        .spawn(async move {
            db.upsert_worktree_sort_index(key, sort_index)
                .await
                .log_err();
        })
        .detach();
}

fn persist_worktree_sort_delete(&self, key: WorktreeGroupOverrideKey) {
    let db = self.db.clone();
    self.executor
        .spawn(async move {
            db.delete_worktree_sort_index(key).await.log_err();
        })
        .detach();
}
```

If the existing `persist_worktree_override` uses a different pattern (e.g. routes through a shared dedup queue with `enqueue_db_op`), match that pattern instead — read the existing implementation and mirror it precisely. The point is the same write deduplication semantics that `worktree_group_overrides` already has.

- [ ] **Step 1.10: Run the test, confirm it passes**

Run: `cargo test -p agent_ui test_worktree_orders_round_trip_through_database`

Expected: PASS. The test writes three paths in an explicit order, drops the store, creates a new one against the same DB file, and verifies the sort indices come back as 0/1/2.

- [ ] **Step 1.11: Add a second test — partial overlap and stale-row cleanup**

Append:

```rust
#[gpui::test]
async fn test_set_worktree_sort_order_clears_paths_not_in_new_order(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let db_path = tempfile::NamedTempFile::new().unwrap().into_temp_path();
    let store = cx.update(|cx| {
        ThreadMetadataStore::new_with_database_path(db_path.to_path_buf(), cx)
    });
    cx.run_until_parked();

    let a: PathBuf = "/p/a".into();
    let b: PathBuf = "/p/b".into();
    let c: PathBuf = "/p/c".into();

    store.update(cx, |store, cx| {
        store.set_worktree_sort_order(None, vec![a.clone(), b.clone(), c.clone()], cx);
    });
    cx.run_until_parked();

    // Now reorder to only [c, a] — b should drop from the order entirely.
    store.update(cx, |store, cx| {
        store.set_worktree_sort_order(None, vec![c.clone(), a.clone()], cx);
    });
    cx.run_until_parked();

    store.read_with(cx, |store, _| {
        assert_eq!(store.worktree_sort_index(None, &c), Some(0));
        assert_eq!(store.worktree_sort_index(None, &a), Some(1));
        assert_eq!(store.worktree_sort_index(None, &b), None);
    });
}
```

- [ ] **Step 1.12: Run all worktree-order tests**

Run: `cargo test -p agent_ui worktree_order`

Expected: both tests PASS.

- [ ] **Step 1.13: Run clippy**

Run: `./script/clippy -p agent_ui`

Expected: no warnings introduced by the new code.

- [ ] **Step 1.14: Commit**

Suggested message (prompt the user to authorize):

```
agent_ui: Persist worktree sort order in thread_metadata_store

Adds a new worktree_orders SQLite table and matching in-memory cache on
ThreadMetadataStore. Mirrors the existing worktree_group_overrides
pattern: async load at startup, write-through on update, dedupe of
in-flight writes against the dirty key set.
```

---

## Task 2: Persistence layer for project group sort order

### Files

- Modify: `crates/agent_ui/src/thread_metadata_store.rs`
- Test: `crates/agent_ui/src/thread_metadata_store.rs` (in the existing `mod tests`)

### Steps

- [ ] **Step 2.1: Write failing test for round-trip of project group sort indices**

Append in `mod tests`:

```rust
#[gpui::test]
async fn test_project_group_orders_round_trip_through_database(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let db_path = tempfile::NamedTempFile::new().unwrap().into_temp_path();
    let store = cx.update(|cx| {
        ThreadMetadataStore::new_with_database_path(db_path.to_path_buf(), cx)
    });
    cx.run_until_parked();

    let key_a = ProjectGroupKey::new(None, PathList::new(&[Path::new("/a")]));
    let key_b = ProjectGroupKey::new(None, PathList::new(&[Path::new("/b")]));
    let key_c = ProjectGroupKey::new(None, PathList::new(&[Path::new("/c")]));

    store.update(cx, |store, cx| {
        store.set_project_group_sort_order(
            vec![key_c.clone(), key_a.clone(), key_b.clone()],
            cx,
        );
    });
    cx.run_until_parked();

    drop(store);
    let store = cx.update(|cx| {
        ThreadMetadataStore::new_with_database_path(db_path.to_path_buf(), cx)
    });
    cx.run_until_parked();

    store.read_with(cx, |store, _| {
        assert_eq!(store.project_group_sort_index(&key_c), Some(0));
        assert_eq!(store.project_group_sort_index(&key_a), Some(1));
        assert_eq!(store.project_group_sort_index(&key_b), Some(2));
    });
}
```

- [ ] **Step 2.2: Run test, confirm it fails**

Run: `cargo test -p agent_ui test_project_group_orders_round_trip_through_database`

Expected: compile error — `set_project_group_sort_order` and `project_group_sort_index` do not exist.

- [ ] **Step 2.3: Add schema for the `project_group_orders` table**

In the same migration block as Step 1.3, add:

```rust
conn.exec(
    "CREATE TABLE IF NOT EXISTS project_group_orders(
        remote_connection_identity TEXT NOT NULL,
        project_paths TEXT NOT NULL,
        sort_index INTEGER NOT NULL,
        PRIMARY KEY(remote_connection_identity, project_paths)
    ) STRICT",
)?()?;
```

`project_paths` is the serialized form of `ProjectGroupKey::paths` (a `PathList`). We reuse `PathList`'s existing string serialization — see Step 2.4 for the helper.

- [ ] **Step 2.4: Add a key type for project group order rows**

Near `WorktreeGroupOverrideKey` (search for its definition), add:

```rust
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct ProjectGroupOrderKey {
    pub remote_connection_identity: Option<String>,
    pub project_paths: String,
}

impl ProjectGroupOrderKey {
    pub fn from_group_key(key: &ProjectGroupKey) -> Self {
        let remote_connection_identity =
            WorktreeGroupOverrideKey::remote_identity(key.host().as_ref());
        Self {
            remote_connection_identity,
            project_paths: key.path_list().serialize(),
        }
    }
}
```

If `PathList::serialize()` doesn't exist with that exact name, use whatever method the existing `folder_paths_order TEXT` column relies on (line 1602/1655 — referenced in research). The point is a canonical, deterministic string round-trip.

- [ ] **Step 2.5: Add the row struct and DB load function**

Below `WorktreeOrderRow` (added in Step 1.5), add:

```rust
#[derive(Debug, Clone)]
struct ProjectGroupOrderRow {
    key: ProjectGroupOrderKey,
    sort_index: i64,
}

impl sqlez::bindable::Column for ProjectGroupOrderRow {
    fn column(statement: &mut Statement, start_index: i32) -> anyhow::Result<(Self, i32)> {
        let (remote, next) = String::column(statement, start_index)?;
        let (paths, next) = String::column(statement, next)?;
        let (sort_index, next) = i64::column(statement, next)?;
        let remote_connection_identity = if remote.is_empty() { None } else { Some(remote) };
        Ok((
            Self {
                key: ProjectGroupOrderKey {
                    remote_connection_identity,
                    project_paths: paths,
                },
                sort_index,
            },
            next,
        ))
    }
}

async fn load_project_group_orders(
    &self,
) -> anyhow::Result<Vec<ProjectGroupOrderRow>> {
    self.write(move |conn| {
        conn.select::<ProjectGroupOrderRow>(
            "SELECT remote_connection_identity, project_paths, sort_index \
             FROM project_group_orders \
             ORDER BY remote_connection_identity, project_paths",
        )?()
    })
    .await
}
```

- [ ] **Step 2.6: Add the DB upsert + delete functions**

Below `upsert_worktree_sort_index` (added in Step 1.6), add:

```rust
async fn upsert_project_group_sort_index(
    &self,
    key: ProjectGroupOrderKey,
    sort_index: i64,
) -> anyhow::Result<()> {
    let remote = key.remote_connection_identity.unwrap_or_default();
    let paths = key.project_paths;
    self.write(move |conn| {
        let mut stmt = Statement::prepare(
            conn,
            "INSERT INTO project_group_orders(remote_connection_identity, project_paths, sort_index) \
             VALUES (?1, ?2, ?3) \
             ON CONFLICT(remote_connection_identity, project_paths) DO UPDATE SET \
                 sort_index = excluded.sort_index",
        )?;
        let mut i = stmt.bind(&remote, 1)?;
        i = stmt.bind(&paths, i)?;
        stmt.bind(&sort_index, i)?;
        stmt.exec()
    })
    .await
}

async fn delete_project_group_sort_index(
    &self,
    key: ProjectGroupOrderKey,
) -> anyhow::Result<()> {
    let remote = key.remote_connection_identity.unwrap_or_default();
    let paths = key.project_paths;
    self.write(move |conn| {
        let mut stmt = Statement::prepare(
            conn,
            "DELETE FROM project_group_orders \
             WHERE remote_connection_identity = ?1 AND project_paths = ?2",
        )?;
        let mut i = stmt.bind(&remote, 1)?;
        stmt.bind(&paths, i)?;
        stmt.exec()
    })
    .await
}
```

- [ ] **Step 2.7: Add the in-memory cache fields and the startup loader**

In the struct (next to `worktree_sort_indices` added in Step 1.4):

```rust
project_group_sort_indices: HashMap<ProjectGroupOrderKey, i64>,
project_group_sort_indices_loaded: bool,
dirty_project_group_sort_keys: HashSet<ProjectGroupOrderKey>,
```

Initialize them in the constructor next to `worktree_sort_indices`:

```rust
project_group_sort_indices: HashMap::new(),
project_group_sort_indices_loaded: false,
dirty_project_group_sort_keys: HashSet::new(),
```

In the startup loader area (next to `load_worktree_orders_task` from Step 1.7):

```rust
let load_project_group_orders_task = cx.background_spawn({
    let db = db.clone();
    async move {
        db.load_project_group_orders()
            .await
            .context("Failed to fetch project group sort orders")
    }
});
cx.spawn(async move |this, cx| {
    let Some(rows) = load_project_group_orders_task.await.log_err() else {
        return;
    };
    this.update(cx, |this, cx| {
        for row in rows {
            if this.dirty_project_group_sort_keys.contains(&row.key) {
                continue;
            }
            this.project_group_sort_indices.insert(row.key, row.sort_index);
        }
        this.project_group_sort_indices_loaded = true;
        cx.notify();
    })
    .ok();
})
.detach();
```

- [ ] **Step 2.8: Add the public read API**

Near `worktree_sort_index` (added in Step 1.8):

```rust
pub fn project_group_sort_index(&self, key: &ProjectGroupKey) -> Option<i64> {
    let key = ProjectGroupOrderKey::from_group_key(key);
    self.project_group_sort_indices.get(&key).copied()
}
```

- [ ] **Step 2.9: Add the public write API**

Near `set_worktree_sort_order` (added in Step 1.9):

```rust
pub fn set_project_group_sort_order(
    &mut self,
    ordered_keys: Vec<ProjectGroupKey>,
    cx: &mut Context<Self>,
) {
    let new_keys: Vec<ProjectGroupOrderKey> = ordered_keys
        .iter()
        .map(ProjectGroupOrderKey::from_group_key)
        .collect();

    // Drop any cached row not present in the new order.
    let stale_keys: Vec<ProjectGroupOrderKey> = self
        .project_group_sort_indices
        .keys()
        .filter(|key| !new_keys.contains(key))
        .cloned()
        .collect();
    for key in stale_keys {
        self.project_group_sort_indices.remove(&key);
        self.dirty_project_group_sort_keys.insert(key.clone());
        let db = self.db.clone();
        self.executor
            .spawn(async move {
                db.delete_project_group_sort_index(key).await.log_err();
            })
            .detach();
    }

    for (i, key) in new_keys.into_iter().enumerate() {
        let new_index = i as i64;
        if self.project_group_sort_indices_loaded
            && self.project_group_sort_indices.get(&key) == Some(&new_index)
        {
            continue;
        }
        self.project_group_sort_indices.insert(key.clone(), new_index);
        self.dirty_project_group_sort_keys.insert(key.clone());
        let db = self.db.clone();
        let persisted_key = key.clone();
        self.executor
            .spawn(async move {
                db.upsert_project_group_sort_index(persisted_key, new_index)
                    .await
                    .log_err();
            })
            .detach();
    }
    cx.notify();
}
```

- [ ] **Step 2.10: Run test, confirm it passes**

Run: `cargo test -p agent_ui test_project_group_orders_round_trip_through_database`

Expected: PASS.

- [ ] **Step 2.11: Run clippy**

Run: `./script/clippy -p agent_ui`

Expected: no warnings introduced.

- [ ] **Step 2.12: Commit**

Suggested message:

```
agent_ui: Persist project group sort order

Adds a new project_group_orders table and matching cache, public APIs
(set_project_group_sort_order, project_group_sort_index), and startup
loader. Mirrors the worktree_orders shape from the previous change.
```

---

## Task 3: Apply persisted order in `rebuild_contents`

### Files

- Modify: `crates/sidebar/src/sidebar.rs:1203-1842` (the `rebuild_contents` function — specifically the project iteration around line 1258 and the worktree iteration around line 1767)
- Test: `crates/sidebar/src/sidebar_tests.rs`

### Steps

- [ ] **Step 3.1: Write a failing test for worktree ordering**

Append in `sidebar_tests.rs` near the other multi-worktree tests:

```rust
#[gpui::test]
async fn test_sidebar_renders_worktrees_in_persisted_order(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    let project = init_test_project_multi(cx).await;
    let (workspace, sidebar) = setup_sidebar(project.clone(), cx).await;

    // The project as set up by `init_test_project_multi` has worktrees in
    // discovery order: ["/a", "/b", "/c"]. Persist the explicit order
    // ["/c", "/a", "/b"].
    cx.update(|_, cx| {
        ThreadMetadataStore::global(cx).update(cx, |store, cx| {
            store.set_worktree_sort_order(
                None,
                vec!["/c".into(), "/a".into(), "/b".into()],
                cx,
            );
        });
    });
    sidebar.update_in(cx, |sidebar, _, cx| sidebar.update_entries(cx));
    cx.run_until_parked();

    let observed = sidebar.read_with(cx, |sidebar, _| {
        worktree_headers(&sidebar.contents)
            .iter()
            .map(|(_, path)| path.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    });
    assert_eq!(observed, vec!["/c", "/a", "/b"]);
}
```

(If `init_test_project_multi` does not already create a project with three known worktree paths, adapt the test to use whatever fixture exists — but the assertion shape stays: build with discovery order, persist a different order, rebuild, observe the persisted order.)

- [ ] **Step 3.2: Run test, confirm it fails**

Run: `cargo test -p sidebar test_sidebar_renders_worktrees_in_persisted_order`

Expected: FAIL — entries still appear in discovery order because `rebuild_contents` doesn't consult `worktree_sort_index` yet.

- [ ] **Step 3.3: Sort `group_folder_paths` before pushing worktree entries**

In `crates/sidebar/src/sidebar.rs`, locate the worktree iteration in `rebuild_contents` (around line 1767):

```rust
let store = ThreadMetadataStore::global(cx).read(cx);
for worktree_path in &group_folder_paths {
    // ...
```

Replace it with a sorted iteration. Insert a sort step using a stable sort by `(Option<i64>, original_position)`:

```rust
let store = ThreadMetadataStore::global(cx).read(cx);
let host = group_host.as_ref();

let mut sorted_worktree_paths: Vec<(usize, &PathBuf)> = group_folder_paths
    .iter()
    .enumerate()
    .collect();
sorted_worktree_paths.sort_by_key(|(natural_ix, path)| {
    // (Some(idx), 0) for explicitly ordered worktrees, sorted by idx ascending.
    // (None, natural_ix) for un-ordered worktrees, preserving discovery order
    // and pushed after the explicitly ordered ones.
    match store.worktree_sort_index(host, path) {
        Some(idx) => (0u8, idx, *natural_ix),
        None => (1u8, 0, *natural_ix as i64),
    }
});

for (_, worktree_path) in sorted_worktree_paths {
    let override_entry = store
        .worktree_override(host, &worktree_path)
        .cloned();
    // ... rest of the loop body unchanged
}
```

The tuple `(0, idx, natural_ix)` vs `(1, 0, natural_ix)` means: anyone explicitly ordered sorts before anyone without an index, and ties within either bucket break by discovery order (stable).

- [ ] **Step 3.4: Run worktree test, confirm it passes**

Run: `cargo test -p sidebar test_sidebar_renders_worktrees_in_persisted_order`

Expected: PASS.

- [ ] **Step 3.5: Write a failing test for project group ordering**

Append in `sidebar_tests.rs`:

```rust
#[gpui::test]
async fn test_sidebar_renders_project_groups_in_persisted_order(
    cx: &mut TestAppContext,
) {
    init_test(cx);
    // init_test_project_multi sets up two project groups, "p1" and "p2",
    // in that natural order. Use whichever multi-project fixture exists.
    let project = init_test_project_multi(cx).await;
    let (workspace, sidebar) = setup_sidebar(project.clone(), cx).await;

    let groups = sidebar.read_with(cx, |sidebar, cx| {
        sidebar.multi_workspace.upgrade().unwrap()
            .read(cx)
            .project_groups(cx)
            .iter()
            .map(|g| g.key().clone())
            .collect::<Vec<_>>()
    });
    assert!(groups.len() >= 2, "fixture must have ≥2 project groups");

    let reversed = groups.iter().cloned().rev().collect::<Vec<_>>();
    cx.update(|_, cx| {
        ThreadMetadataStore::global(cx).update(cx, |store, cx| {
            store.set_project_group_sort_order(reversed.clone(), cx);
        });
    });
    sidebar.update_in(cx, |sidebar, _, cx| sidebar.update_entries(cx));
    cx.run_until_parked();

    let observed = sidebar.read_with(cx, |sidebar, _| {
        sidebar
            .contents
            .entries
            .iter()
            .filter_map(|e| match e {
                ListEntry::ProjectHeader { key, .. } => Some(key.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
    });
    assert_eq!(observed, reversed);
}
```

- [ ] **Step 3.6: Run test, confirm it fails**

Run: `cargo test -p sidebar test_sidebar_renders_project_groups_in_persisted_order`

Expected: FAIL.

- [ ] **Step 3.7: Sort project groups before iterating**

In `rebuild_contents`, find the iteration over project groups (around line 1258, `for group in mw.project_groups(cx)` or similar). Wrap it in a sort step:

```rust
let store = ThreadMetadataStore::global(cx).read(cx);
let mut groups: Vec<_> = mw.project_groups(cx).iter().enumerate().collect();
groups.sort_by_key(|(natural_ix, group)| {
    match store.project_group_sort_index(group.key()) {
        Some(idx) => (0u8, idx, *natural_ix),
        None => (1u8, 0, *natural_ix as i64),
    }
});
drop(store); // release the borrow before mutating self below

for (_, group) in groups {
    // ... rest of the rebuild_contents loop body unchanged
}
```

Adapt to the actual variable names — the key invariant is: collect groups into a Vec, sort stably by `(bucket, idx, natural_ix)`, then iterate.

- [ ] **Step 3.8: Run project test, confirm it passes**

Run: `cargo test -p sidebar test_sidebar_renders_project_groups_in_persisted_order`

Expected: PASS.

- [ ] **Step 3.9: Run the full sidebar test suite for regressions**

Run: `cargo test -p sidebar`

Expected: all existing tests still pass — the sorts are stable and degenerate to natural order when no `sort_index` rows exist.

- [ ] **Step 3.10: Run clippy**

Run: `./script/clippy -p sidebar`

- [ ] **Step 3.11: Commit**

Suggested message:

```
sidebar: Apply persisted sort order in rebuild_contents

Project groups and worktrees within each group now consult the
ThreadMetadataStore sort indices and are rendered in the persisted
order. Entries without an explicit sort index keep discovery order
and follow the explicitly ordered ones.
```

---

## Task 4: Drag payload types, drag state, and drop indicator rendering

This task only adds data + visual scaffolding. The actual `on_drag` / `on_drop` wiring is in Tasks 5 and 6.

### Files

- Modify: `crates/sidebar/src/sidebar.rs`

### Steps

- [ ] **Step 4.1: Define drag payload types**

Near the `ListEntry` enum (around line 271), add two new structs:

```rust
#[derive(Debug, Clone)]
pub struct DraggedProjectHeader {
    pub key: ProjectGroupKey,
    pub source_index: usize,
}

#[derive(Debug, Clone)]
pub struct DraggedWorktreeHeader {
    pub project_key: ProjectGroupKey,
    pub worktree_path: Arc<Path>,
    pub source_index: usize,
}
```

These are the values passed to `.on_drag(value, ...)` and matched on in `.drag_over::<T>(...)` and `.on_drop(...)`. GPUI uses the concrete type as the drag-type discriminator.

- [ ] **Step 4.2: Define drop-slot state**

Below those structs:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropSide {
    Above,
    Below,
}

#[derive(Debug, Clone)]
pub enum ActiveDrag {
    Project {
        source_index: usize,
        hovered_index: Option<usize>,
        hovered_side: DropSide,
    },
    Worktree {
        project_key: ProjectGroupKey,
        source_index: usize,
        hovered_index: Option<usize>,
        hovered_side: DropSide,
    },
}
```

`hovered_index` is the index in `self.contents.entries` of the entry the pointer is currently over. `hovered_side` is which half of that entry the pointer is in, determining whether we'd drop above or below.

- [ ] **Step 4.3: Add the state field on `Sidebar`**

In the `Sidebar` struct (search for `pub struct Sidebar`), add:

```rust
active_drag: Option<ActiveDrag>,
```

In every `Sidebar::new(...)` constructor (use Grep to find them), initialize it to `None`:

```rust
active_drag: None,
```

- [ ] **Step 4.4: Helper — clear drag state**

Add a method on `impl Sidebar`:

```rust
fn clear_active_drag(&mut self, cx: &mut Context<Self>) {
    if self.active_drag.take().is_some() {
        cx.notify();
    }
}
```

- [ ] **Step 4.5: Helper — render a drop indicator**

Add a method on `impl Sidebar` near the other render helpers (around `render_list_entry`):

```rust
fn render_drop_indicator(&self, cx: &Context<Self>) -> AnyElement {
    h_flex()
        .h(px(2.))
        .w_full()
        .bg(cx.theme().colors().drop_target_background)
        .into_any_element()
}
```

(If `drop_target_background` isn't a defined theme color, substitute the most appropriate accent — search the theme for `drop_target` / `border_focused` and pick whichever already exists.)

- [ ] **Step 4.6: Render indicators conditionally around each entry**

This is the trickiest UI step. Update `render_list_entry` (line 1921) so that, when `self.active_drag` is `Some` and the indicator should appear above or below this index, it returns a wrapper containing the indicator + the entry. Replace the body's final assembly with:

```rust
let (above_indicator, below_indicator) = match self.active_drag.as_ref() {
    Some(ActiveDrag::Project { hovered_index: Some(h), hovered_side, .. })
        if matches!(&entry, ListEntry::ProjectHeader { .. }) && *h == ix =>
    {
        match hovered_side {
            DropSide::Above => (Some(self.render_drop_indicator(cx)), None),
            DropSide::Below => (None, Some(self.render_drop_indicator(cx))),
        }
    }
    Some(ActiveDrag::Worktree {
        project_key,
        hovered_index: Some(h),
        hovered_side,
        ..
    }) => {
        let same_target = match &entry {
            ListEntry::WorktreeHeader { project_group_key, .. } => {
                project_group_key == project_key && *h == ix
            }
            _ => false,
        };
        if same_target {
            match hovered_side {
                DropSide::Above => (Some(self.render_drop_indicator(cx)), None),
                DropSide::Below => (None, Some(self.render_drop_indicator(cx))),
            }
        } else {
            (None, None)
        }
    }
    _ => (None, None),
};

v_flex()
    .when_some(above_indicator, |this, indicator| this.child(indicator))
    .child(rendered)
    .when_some(below_indicator, |this, indicator| this.child(indicator))
    .into_any_element()
```

(`rendered` is the existing variable name from the match block. Keep the previous logic that builds `rendered`; only the *return* statement of the function changes.)

- [ ] **Step 4.7: Confirm the project compiles**

Run: `cargo check -p sidebar`

Expected: success. Nothing renders an indicator yet because nothing sets `active_drag`, but the function paths compile.

- [ ] **Step 4.8: Run clippy**

Run: `./script/clippy -p sidebar`

- [ ] **Step 4.9: Commit**

Suggested message:

```
sidebar: Add drag payload types and drop indicator rendering

Defines DraggedProjectHeader and DraggedWorktreeHeader, an ActiveDrag
state machine on the Sidebar, and a 2px drop indicator that renders
above or below the hovered entry. No drag/drop is wired yet — Tasks
5 and 6 add the handlers.
```

---

## Task 5: Wire drag/drop on worktree headers

### Files

- Modify: `crates/sidebar/src/sidebar.rs` — `render_worktree_header` (line 2068)
- Test: `crates/sidebar/src/sidebar_tests.rs`

### Steps

- [ ] **Step 5.1: Write a failing test that simulates a drop**

Direct UI-driven drag testing is hard in GPUI tests. Instead, test the *side-effect entry point* — a method we'll add that the drop handler will call. Add this test:

```rust
#[gpui::test]
async fn test_drop_worktree_updates_sort_order(cx: &mut TestAppContext) {
    init_test(cx);
    let project = init_test_project_multi(cx).await;
    let (workspace, sidebar) = setup_sidebar(project.clone(), cx).await;

    let (project_key, original) = sidebar.read_with(cx, |sidebar, _| {
        // Use whichever helper returns the worktrees of the first project.
        let entries = worktree_headers(&sidebar.contents);
        assert!(entries.len() >= 2, "fixture must have ≥2 worktrees");
        let project_key = match &sidebar.contents.entries[0] {
            ListEntry::ProjectHeader { key, .. } => key.clone(),
            _ => panic!("expected ProjectHeader at index 0"),
        };
        let paths: Vec<PathBuf> = entries
            .iter()
            .map(|(_, p)| p.to_path_buf())
            .collect();
        (project_key, paths)
    });

    let drag = DraggedWorktreeHeader {
        project_key: project_key.clone(),
        worktree_path: Arc::from(original[1].as_path()),
        source_index: 0, // value doesn't matter for the drop call
    };
    sidebar.update_in(cx, |sidebar, _, cx| {
        // Drop worktree[1] just above worktree[0].
        sidebar.handle_worktree_drop(&drag, /*target_index_in_project=*/ 0, DropSide::Above, cx);
    });
    cx.run_until_parked();

    let observed: Vec<PathBuf> = sidebar.read_with(cx, |sidebar, _| {
        worktree_headers(&sidebar.contents)
            .iter()
            .map(|(_, p)| p.to_path_buf())
            .collect()
    });

    let mut expected = original.clone();
    let moved = expected.remove(1);
    expected.insert(0, moved);
    assert_eq!(observed, expected);
}
```

- [ ] **Step 5.2: Run test, confirm it fails**

Run: `cargo test -p sidebar test_drop_worktree_updates_sort_order`

Expected: compile error — `handle_worktree_drop` does not exist.

- [ ] **Step 5.3: Implement `handle_worktree_drop`**

Add to `impl Sidebar`:

```rust
fn handle_worktree_drop(
    &mut self,
    drag: &DraggedWorktreeHeader,
    target_index_in_project: usize,
    side: DropSide,
    cx: &mut Context<Self>,
) {
    let project_key = drag.project_key.clone();

    // Collect the worktrees currently rendered for this project in their
    // current visual order — this is the source of truth we are mutating.
    let mut current_paths: Vec<PathBuf> = self
        .contents
        .entries
        .iter()
        .filter_map(|entry| match entry {
            ListEntry::WorktreeHeader { project_group_key, worktree_path, .. }
                if project_group_key == &project_key =>
            {
                Some(worktree_path.clone())
            }
            _ => None,
        })
        .collect();

    let dragged_path: PathBuf = drag.worktree_path.as_ref().to_path_buf();
    let Some(source_pos) =
        current_paths.iter().position(|p| p == &dragged_path)
    else {
        return;
    };
    let moved = current_paths.remove(source_pos);

    // Adjust target index now that source was removed.
    let mut insert_at = target_index_in_project;
    if source_pos < target_index_in_project {
        insert_at = insert_at.saturating_sub(1);
    }
    if matches!(side, DropSide::Below) {
        insert_at = insert_at.saturating_add(1);
    }
    let insert_at = insert_at.min(current_paths.len());
    current_paths.insert(insert_at, moved);

    let remote = project_key.host();
    ThreadMetadataStore::global(cx).update(cx, |store, cx| {
        store.set_worktree_sort_order(remote.as_ref(), current_paths, cx);
    });
    self.clear_active_drag(cx);
    self.update_entries(cx);
}
```

- [ ] **Step 5.4: Run test, confirm it passes**

Run: `cargo test -p sidebar test_drop_worktree_updates_sort_order`

Expected: PASS.

- [ ] **Step 5.5: Wire `on_drag` on worktree headers**

In `render_worktree_header` (line 2068), find the element chain that builds the header (`let id = ...; ... let header = h_flex().id(id)...`). At the *very end* of the chain before `.into_any_element()`, add:

```rust
.on_drag(
    DraggedWorktreeHeader {
        project_key: project_group_key.clone(),
        worktree_path: Arc::from(worktree_path.as_path()),
        source_index: ix,
    },
    move |dragged, _, _, cx| {
        cx.new(|_| DraggedWorktreeView {
            display_name: dragged.worktree_path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| dragged.worktree_path.display().to_string()),
        })
    },
)
```

Then add the minimal ghost view (near other small `Render` impls in the file):

```rust
struct DraggedWorktreeView {
    display_name: String,
}

impl Render for DraggedWorktreeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .px_2()
            .py_1()
            .bg(cx.theme().colors().element_background)
            .border_1()
            .border_color(cx.theme().colors().border)
            .rounded_md()
            .child(self.display_name.clone())
    }
}
```

- [ ] **Step 5.6: Wire `drag_over` on worktree headers**

Continue the chain on `render_worktree_header`:

```rust
.drag_over::<DraggedWorktreeHeader>({
    let project_group_key = project_group_key.clone();
    cx.listener(move |this, dragged: &DraggedWorktreeHeader, _, cx| {
        if dragged.project_key != project_group_key {
            return; // refuse cross-project drops
        }
        // Default to "above" on hover — Step 5.7 refines this with pointer Y.
        this.active_drag = Some(ActiveDrag::Worktree {
            project_key: dragged.project_key.clone(),
            source_index: dragged.source_index,
            hovered_index: Some(ix),
            hovered_side: DropSide::Above,
        });
        cx.notify();
    })
})
```

- [ ] **Step 5.7: Refine drop side using pointer position**

GPUI's `drag_over` callback gives a `DragMoveEvent`-style argument depending on the version. If pointer Y is exposed, use it; otherwise approximate with a separate `.on_mouse_move`. Search the codebase for `drag_move_event` or `DragMoveEvent` to see what's available:

```bash
grep -rn "DragMoveEvent\|drag_move_event\|on_drag_move" crates/
```

If `on_drag_move::<T>(|event, window, cx| ...)` exists, use it instead of plain `drag_over`:

```rust
.on_drag_move::<DraggedWorktreeHeader>(cx.listener({
    let project_group_key = project_group_key.clone();
    move |this, event: &DragMoveEvent<DraggedWorktreeHeader>, _, cx| {
        if event.drag.project_key != project_group_key {
            return;
        }
        let bounds = event.bounds;
        let midpoint_y = bounds.origin.y + bounds.size.height / 2.;
        let side = if event.event.position.y < midpoint_y {
            DropSide::Above
        } else {
            DropSide::Below
        };
        this.active_drag = Some(ActiveDrag::Worktree {
            project_key: event.drag.project_key.clone(),
            source_index: event.drag.source_index,
            hovered_index: Some(ix),
            hovered_side: side,
        });
        cx.notify();
    }
}))
```

If no equivalent API is found, leave Step 5.6's `Above`-only default in place and document the follow-up. The plan still works visually; only the half-detection is degraded.

- [ ] **Step 5.8: Wire `on_drop` on worktree headers**

Continue the chain:

```rust
.on_drop(cx.listener({
    let project_group_key = project_group_key.clone();
    move |this, dragged: &DraggedWorktreeHeader, _, cx| {
        if dragged.project_key != project_group_key {
            this.clear_active_drag(cx);
            return;
        }
        // Compute the index of this worktree within its project.
        let target_within_project = this
            .contents
            .entries
            .iter()
            .filter(|e| matches!(e, ListEntry::WorktreeHeader { project_group_key: k, .. } if k == &project_group_key))
            .position(|e| matches!(e, ListEntry::WorktreeHeader { worktree_path, .. } if worktree_path == &worktree_path_clone))
            .unwrap_or(0);
        let side = match &this.active_drag {
            Some(ActiveDrag::Worktree { hovered_side, .. }) => *hovered_side,
            _ => DropSide::Above,
        };
        this.handle_worktree_drop(dragged, target_within_project, side, cx);
    }
}))
```

Note: `worktree_path_clone` is a clone you'll need to add above the closure (`let worktree_path_clone = worktree_path.clone();` near where `id` is computed). Capturing the borrowed `worktree_path` directly across a `'static` closure won't compile.

- [ ] **Step 5.9: Clear drag state on `on_drag_end` / when drop misses**

GPUI's drag system calls callbacks based on where the drop happened. To ensure stale `active_drag` state doesn't linger if the user releases outside any worktree, register a window-level mouse-up handler in `render`, or use a fallback `on_drag_end` if the API supports it. Search for `on_drag_end` / `cancel_drag` in the codebase first. If neither exists, add a lazy clear at the start of `update_entries`:

```rust
// At the top of update_entries:
if self.active_drag.is_some() && !cx.has_active_drag() {
    self.active_drag = None;
}
```

(Where `cx.has_active_drag()` is whatever GPUI exposes; check `App` for `active_drag()` / `has_active_drag()` — search Grep for `active_drag`.)

- [ ] **Step 5.10: Run the drop test and the full sidebar suite**

Run: `cargo test -p sidebar test_drop_worktree_updates_sort_order`
Run: `cargo test -p sidebar`

Expected: all pass.

- [ ] **Step 5.11: Run clippy**

Run: `./script/clippy -p sidebar`

- [ ] **Step 5.12: Commit**

Suggested message:

```
sidebar: Drag-and-drop reorder for worktree headers

Adds DraggedWorktreeHeader as the drag payload, drag_over / on_drop
handlers on render_worktree_header, and a handle_worktree_drop method
that computes the new visual order, persists it via
set_worktree_sort_order, and rebuilds. Cross-project drops are refused.
```

---

## Task 6: Wire drag/drop on project headers

### Files

- Modify: `crates/sidebar/src/sidebar.rs` — `render_project_header` (line 2208)
- Test: `crates/sidebar/src/sidebar_tests.rs`

### Steps

- [ ] **Step 6.1: Write failing test for project drop**

```rust
#[gpui::test]
async fn test_drop_project_updates_sort_order(cx: &mut TestAppContext) {
    init_test(cx);
    let project = init_test_project_multi(cx).await;
    let (workspace, sidebar) = setup_sidebar(project.clone(), cx).await;

    let original_project_keys: Vec<ProjectGroupKey> = sidebar.read_with(cx, |sidebar, _| {
        sidebar
            .contents
            .entries
            .iter()
            .filter_map(|e| match e {
                ListEntry::ProjectHeader { key, .. } => Some(key.clone()),
                _ => None,
            })
            .collect()
    });
    assert!(original_project_keys.len() >= 2);

    let drag = DraggedProjectHeader {
        key: original_project_keys[1].clone(),
        source_index: 0,
    };
    sidebar.update_in(cx, |sidebar, _, cx| {
        sidebar.handle_project_drop(&drag, /*target_project_index=*/ 0, DropSide::Above, cx);
    });
    cx.run_until_parked();

    let observed: Vec<ProjectGroupKey> = sidebar.read_with(cx, |sidebar, _| {
        sidebar
            .contents
            .entries
            .iter()
            .filter_map(|e| match e {
                ListEntry::ProjectHeader { key, .. } => Some(key.clone()),
                _ => None,
            })
            .collect()
    });
    let mut expected = original_project_keys.clone();
    let moved = expected.remove(1);
    expected.insert(0, moved);
    assert_eq!(observed, expected);
}
```

- [ ] **Step 6.2: Run test, confirm it fails**

Run: `cargo test -p sidebar test_drop_project_updates_sort_order`

Expected: compile error — `handle_project_drop` does not exist.

- [ ] **Step 6.3: Implement `handle_project_drop`**

```rust
fn handle_project_drop(
    &mut self,
    drag: &DraggedProjectHeader,
    target_project_index: usize,
    side: DropSide,
    cx: &mut Context<Self>,
) {
    let mut current_keys: Vec<ProjectGroupKey> = self
        .contents
        .entries
        .iter()
        .filter_map(|e| match e {
            ListEntry::ProjectHeader { key, .. } => Some(key.clone()),
            _ => None,
        })
        .collect();

    let Some(source_pos) = current_keys.iter().position(|k| k == &drag.key) else {
        return;
    };
    let moved = current_keys.remove(source_pos);

    let mut insert_at = target_project_index;
    if source_pos < target_project_index {
        insert_at = insert_at.saturating_sub(1);
    }
    if matches!(side, DropSide::Below) {
        insert_at = insert_at.saturating_add(1);
    }
    let insert_at = insert_at.min(current_keys.len());
    current_keys.insert(insert_at, moved);

    ThreadMetadataStore::global(cx).update(cx, |store, cx| {
        store.set_project_group_sort_order(current_keys, cx);
    });
    self.clear_active_drag(cx);
    self.update_entries(cx);
}
```

- [ ] **Step 6.4: Run test, confirm it passes**

Run: `cargo test -p sidebar test_drop_project_updates_sort_order`

Expected: PASS.

- [ ] **Step 6.5: Wire `on_drag` on project headers**

In `render_project_header` (line 2208), at the end of the `header` element chain (just before `.into_any_element()`):

```rust
.on_drag(
    DraggedProjectHeader { key: key.clone(), source_index: ix },
    move |dragged, _, _, cx| {
        cx.new(|_| DraggedProjectView { label: dragged.key.path_list().to_string().into() })
    },
)
```

And the ghost view:

```rust
struct DraggedProjectView {
    label: SharedString,
}

impl Render for DraggedProjectView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .px_2()
            .py_1()
            .font_weight(FontWeight::SEMIBOLD)
            .bg(cx.theme().colors().element_background)
            .border_1()
            .border_color(cx.theme().colors().border)
            .rounded_md()
            .child(self.label.clone())
    }
}
```

(If `PathList::to_string()` doesn't exist, format the first path or use a debug label.)

- [ ] **Step 6.6: Wire `drag_over` / `on_drag_move` on project headers**

```rust
.on_drag_move::<DraggedProjectHeader>(cx.listener(move |this, event: &DragMoveEvent<DraggedProjectHeader>, _, cx| {
    let bounds = event.bounds;
    let midpoint_y = bounds.origin.y + bounds.size.height / 2.;
    let side = if event.event.position.y < midpoint_y {
        DropSide::Above
    } else {
        DropSide::Below
    };
    this.active_drag = Some(ActiveDrag::Project {
        source_index: event.drag.source_index,
        hovered_index: Some(ix),
        hovered_side: side,
    });
    cx.notify();
}))
```

(Fall back to the simpler `.drag_over::<DraggedProjectHeader>(...)` with a fixed `DropSide::Above` if `on_drag_move` isn't available — see Task 5.7's note.)

- [ ] **Step 6.7: Wire `on_drop` on project headers**

```rust
.on_drop(cx.listener(move |this, dragged: &DraggedProjectHeader, _, cx| {
    // Compute the index of THIS project header within the project-only ordering.
    let target_index_among_projects = this
        .contents
        .entries
        .iter()
        .filter(|e| matches!(e, ListEntry::ProjectHeader { .. }))
        .position(|e| matches!(e, ListEntry::ProjectHeader { key: k, .. } if k == &key_clone))
        .unwrap_or(0);
    let side = match &this.active_drag {
        Some(ActiveDrag::Project { hovered_side, .. }) => *hovered_side,
        _ => DropSide::Above,
    };
    this.handle_project_drop(dragged, target_index_among_projects, side, cx);
}))
```

Add `let key_clone = key.clone();` near where `id` is built, since `key` is borrowed for `'a` but the listener needs `'static`.

- [ ] **Step 6.8: Run drop tests + full suite**

Run: `cargo test -p sidebar test_drop_project_updates_sort_order`
Run: `cargo test -p sidebar`

Expected: all pass.

- [ ] **Step 6.9: Run clippy**

Run: `./script/clippy -p sidebar`

- [ ] **Step 6.10: Commit**

Suggested message:

```
sidebar: Drag-and-drop reorder for project headers

Adds DraggedProjectHeader as the drag payload, drag_over / on_drop
handlers on render_project_header, and handle_project_drop which
computes the new order across project groups, persists it via
set_project_group_sort_order, and rebuilds.
```

---

## Task 7: End-to-end manual verification

Automated tests in Tasks 1-6 cover the persistence layer, the rebuild ordering, and the drop side effects. They do *not* cover the actual pointer-driven drag interaction in a running window. This task is a manual checklist the executing agent must walk through before declaring the feature complete.

### Files

- None — this is a behavioral verification step. If a bug surfaces, return to the relevant earlier task.

### Steps

- [ ] **Step 7.1: Launch the app**

Run: `cargo run` (or whatever invocation the repo uses for the dev binary — `script/run` if present).

Expected: the app launches and the sidebar shows at least one project with multiple worktrees.

- [ ] **Step 7.2: Reorder worktrees within a project**

1. Open a project with ≥ 2 worktrees.
2. Click and hold on a worktree subheader.
3. Drag it over another worktree subheader within the same project.
4. Verify a horizontal indicator line appears above or below the target depending on pointer Y.
5. Release.

Expected: the worktrees swap visually. The dragged worktree is now in the new position; the dragged ghost disappears.

- [ ] **Step 7.3: Confirm worktree order persists across restart**

1. Quit the app.
2. Relaunch.

Expected: the reordered worktrees come back in the new order, not the original discovery order.

- [ ] **Step 7.4: Reorder project headers**

1. With ≥ 2 projects open in the sidebar, drag one project header over another.
2. Verify the indicator appears at the correct side.
3. Release.

Expected: projects swap visually.

- [ ] **Step 7.5: Confirm project order persists across restart**

1. Quit and relaunch.

Expected: the new project order is preserved.

- [ ] **Step 7.6: Try invalid drag targets**

1. Drag a worktree subheader over a project header from a *different* project.

Expected: no drop indicator appears, and releasing has no effect.

2. Drag a worktree onto a thread/terminal entry within its own project.

Expected: no drop occurs; releasing has no effect.

- [ ] **Step 7.7: Smoke-check that other interactions still work**

- Clicking a worktree subheader still toggles its collapse state.
- Right-click context menus on headers still open.
- Thread entries beneath worktrees still highlight on hover.

If any of these regress, the drag handlers are intercepting events they shouldn't. Add `stop_propagation` on the *mouse-down inside drag handle* path, not on the whole header, and re-test.

- [ ] **Step 7.8: Final sign-off**

Run the full test sweep:

```
cargo test -p agent_ui
cargo test -p sidebar
./script/clippy
```

Expected: all green. If so, the feature is ready to PR.

---

## Known limitations / follow-ups (out of scope for this plan)

- **Orphan rows.** If a worktree or project is deleted from disk while the user wasn't running Zed, its `sort_index` row remains in the DB indefinitely. Cheap to ignore; a periodic cleanup pass mirroring `cleanup_worktree_overrides_removes_unreferenced_paths` (tested at line 2420) can be added later.
- **Keyboard reorder.** Drag-and-drop is mouse-only here. A future addition could bind actions like `ProjectPanel: MoveProjectUp` / `MoveProjectDown` / `MoveWorktreeUp` / `MoveWorktreeDown` and dispatch them via the existing keymap, using the same persistence APIs.
- **Sort_index drift / renumbering on every reorder.** We rewrite every row in scope on every reorder. With typical project counts (single digits) and worktree counts (single to low-double digits), this is unmeasurable. A migration to gap-based ordering (e.g. multiples of 1024 with periodic rebalance) is only worth it if the entry count grows substantially.

---

**Plan complete.**
