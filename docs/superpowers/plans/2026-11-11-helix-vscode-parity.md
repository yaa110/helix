# Helix VSCode-Parity Features Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add session restore, automatic file reload, a persistent VSCode-like file tree, and VSCode-style workspace search/replace to this Helix fork.

**Architecture:** State that commands and sessions need lives in `helix-view` (`Editor`, `SidebarState`, `SessionStore`). UI and services live in `helix-term` (`EditorView`, `Sidebar`, `FileWatcher`, search engine). Phases are ordered so each lands alone: session → auto-reload → sidebar/tree → search/replace. The sidebar is a real layout pane inside `EditorView`, with its own key-routing branch. The watcher is a single debounced `notify` service in `Application`.

**Tech Stack:** Rust 2021/2024 workspace, `tokio`, `serde`/`serde_json`, `sha2`, `tempfile`, `notify` + `notify-debouncer-mini`, existing `ignore`/`grep-searcher`/`grep-regex`, `regex`, Helix's own `Compositor`/`Component`/`Editor` APIs.

**Spec:** `docs/superpowers/specs/2026-11-11-helix-vscode-parity-design.md`

## Global Constraints

- **No pushes.** Commit every task locally with the message shown in its final step.
- **VSCode is the behavioral tie-breaker.** Modal interaction translates to single-key Vim idioms.
- **Never discard unsaved buffer text** without an explicit user choice. The conflict menu defaults to **Keep**.
- **Config names are kebab-case** (`#[serde(rename_all = "kebab-case")]`), matching existing `Config`.
- **Tests run with `restore_session = false` and `auto_reload = false` by default** via `test_editor_config()`. Tests that need these features opt in and use a temp `SessionStore`.
- **No plugin or scripting layer.** No new external processes except the existing clipboard/LSP/shell providers.
- **Before every commit:** run `cargo fmt --all`, the task's test command, and `cargo check -p helix-term`.
- Integration tests require `--features integration` and live under `helix-term/tests/test/`.

## Review Focus

These are the input classes most likely to break a real user. Each has a test in the owning task.

1. **Unicode and grapheme positions** — session restore and search selections must land on the same visible character after combining marks/emoji. Owned by Tasks 2, 18, 20, 21.
2. **Atomic saves/renames** — watcher must see a temp-file rename, not just in-place writes; the editor's own save must not trigger a reload. Owned by Tasks 7, 9.
3. **Dirty-buffer conflicts** — an external write must never discard unsaved text; the menu must not swallow a destructive keystroke. Owned by Tasks 9, 10.
4. **Stale search replace** — files edited after results were shown must be revalidated before writing; CRLF and binary files must not be corrupted. Owned by Tasks 21, 22.
5. **Narrow terminals and focus leaks** — sidebar width must clamp/hide, and tree/search keys must not leak into normal mode (or vice versa). Owned by Tasks 11, 13.

---

## Phase 1 — Session persistence

### Task 1: Sidebar state, workspace root, and config scaffolding

**Files:**
- Create: `helix-view/src/sidebar.rs`
- Modify: `helix-view/src/lib.rs`, `helix-view/src/editor.rs`
- Test: `helix-view/src/sidebar.rs` (`#[cfg(test)]`), `helix-term/src/config.rs` (existing tests module)

**Interfaces:**
- Produces:
  - `helix_view::sidebar::SidebarMode { Hidden, Files, Search }` (serde kebab-case, `Default = Hidden`)
  - `SidebarFocus { Editor, Sidebar }` (`Default = Editor`)
  - `SidebarConfig { width: u16, min_width: u16, max_width: u16 }` (`Default = 30/20/50`)
  - `SearchInputs { query, replacement, match_case, whole_word, regex, preserve_case }` (`Default` all empty/false)
  - `SidebarState { mode, focus, width, expanded: HashSet<PathBuf>, selected: Option<PathBuf>, search: SearchInputs, refresh_dirs: HashSet<PathBuf>, refresh_epoch: u64 }`
  - `SidebarState::visible_width(&self, area_width: u16, config: &SidebarConfig) -> u16`
  - `Editor { workspace_root: PathBuf, sidebar: SidebarState, session_dirty: bool, session_active: bool }`
  - `Config { restore_session: bool, sidebar: SidebarConfig }`

- [ ] **Step 1: Write the failing tests**

Add to `helix-view/src/sidebar.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_and_narrow_sidebars_have_zero_width() {
        let cfg = SidebarConfig::default();
        let mut state = SidebarState::default();
        assert_eq!(state.visible_width(120, &cfg), 0);

        state.mode = SidebarMode::Files;
        assert_eq!(state.visible_width(59, &cfg), 0); // 30 + 40 editor minimum
        assert_eq!(state.visible_width(60, &cfg), 30);
    }

    #[test]
    fn width_is_clamped_to_config_bounds() {
        let cfg = SidebarConfig::default();
        let mut state = SidebarState::default();
        state.mode = SidebarMode::Files;
        state.width = 99;
        assert_eq!(state.visible_width(200, &cfg), 50);
        state.width = 1;
        assert_eq!(state.visible_width(200, &cfg), 20);
    }
}
```

Add to `helix-term/src/config.rs` tests:

```rust
#[test]
fn parses_session_and_sidebar_config() {
    let config = Config::load_test(
        r#"
        [editor]
        restore-session = false
        [editor.sidebar]
        width = 24
        min-width = 18
        max-width = 40
        "#,
    );
    assert!(!config.editor.restore_session);
    assert_eq!(config.editor.sidebar.width, 24);
    assert_eq!(config.editor.sidebar.min_width, 18);
    assert_eq!(config.editor.sidebar.max_width, 40);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p helix-view sidebar::tests -q && cargo test -p helix-term config::tests::parses_session_and_sidebar_config -q`
Expected: FAIL — `sidebar` module and config fields do not exist.

- [ ] **Step 3: Implement the scaffolding**

Create `helix-view/src/sidebar.rs` with the structs above. `visible_width` returns `0` when `mode == Hidden`; otherwise clamp `width` to `[min_width, max_width]`, then return `0` if `area_width < width + 40`, else `width`.

In `helix-view/src/lib.rs` add `pub mod sidebar;`.

In `helix-view/src/editor.rs`:
- Add `#[serde(rename_all = "kebab-case", default, deny_unknown_fields)]` `SidebarConfig` field `pub sidebar: SidebarConfig` and `pub restore_session: bool` to `Config`; default `SidebarConfig::default()` and `true`.
- Add the four `Editor` fields and initialize them in `Editor::new`: empty `PathBuf`, `SidebarState::default()`, `false`, `false`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p helix-view sidebar::tests -q && cargo test -p helix-term config::tests -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add helix-view/src/sidebar.rs helix-view/src/lib.rs helix-view/src/editor.rs helix-term/src/config.rs
git commit -m "feat(session): add sidebar/session config and editor state"
```

### Task 2: Session schema and position codec

**Files:**
- Create: `helix-view/src/session.rs`
- Modify: `helix-view/src/lib.rs`, `helix-view/src/editor.rs`
- Test: `helix-view/src/session.rs`

**Interfaces:**
- Consumes: `SidebarMode`, `SearchInputs` from Task 1.
- Produces:
  - `Point { line: usize, col: usize }` (serde as `[line, col]`)
  - `SelectionState { anchor: Point, head: Point }`
  - `ScrollState { anchor_line: usize, vertical_offset: usize, horizontal_offset: usize }`
  - `BufferState { path: PathBuf, selections: Vec<SelectionState>, primary: usize, scroll: ScrollState }`
  - `SidebarSession { mode: SidebarMode, width: u16, expanded: Vec<PathBuf>, selected: Option<PathBuf> }`
  - `Session { version: u32, workspace: PathBuf, focused: Option<PathBuf>, buffers: Vec<BufferState>, sidebar: SidebarSession, search: SearchInputs }`
  - `encode_point(text: RopeSlice<'_>, char_idx: usize) -> Point`
  - `decode_point(text: RopeSlice<'_>, point: Point) -> usize`

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use helix_core::Rope;

    #[test]
    fn point_round_trip_handles_unicode_and_clamping() {
        let rope = Rope::from_str("alpha\nβeta\n🎉 end");
        let idx = rope.line_to_char(1) + 2; // after 'β'
        assert_eq!(decode_point(rope.slice(..), encode_point(rope.slice(..), idx)), idx);

        let clamped = decode_point(rope.slice(..), Point { line: 0, col: 999 });
        assert_eq!(clamped, rope.line_to_char(1)); // end of first line

        let past_end = decode_point(rope.slice(..), Point { line: 999, col: 0 });
        assert_eq!(past_end, rope.len_chars());
    }

    #[test]
    fn session_json_round_trip() {
        let session = Session {
            version: 1,
            workspace: "/tmp/ws".into(),
            focused: Some("src/main.rs".into()),
            buffers: vec![BufferState {
                path: "src/main.rs".into(),
                selections: vec![SelectionState {
                    anchor: Point { line: 1, col: 2 },
                    head: Point { line: 3, col: 4 },
                }],
                primary: 0,
                scroll: ScrollState { anchor_line: 1, vertical_offset: 0, horizontal_offset: 0 },
            }],
            sidebar: SidebarSession::default(),
            search: SearchInputs::default(),
        };
        let json = serde_json::to_string(&session).unwrap();
        assert_eq!(serde_json::from_str::<Session>(&json).unwrap(), session);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p helix-view session::tests -q`
Expected: FAIL — `session` module missing.

- [ ] **Step 3: Implement the schema and codec**

Create `helix-view/src/session.rs`. Add `pub mod session;` to `helix-view/src/lib.rs`.

- `encode_point`: `line = text.char_to_line(char_idx.min(text.len_chars()))`, `col = char_idx - text.line_to_char(line)`.
- `decode_point`: `line = point.line.min(text.len_lines().saturating_sub(1))`; `start = text.line_to_char(line)`; `end = if line + 1 < text.len_lines() { text.line_to_char(line + 1) } else { text.len_chars() }`; return `start + point.col.min(end - start)`. Strip a trailing `\r\n`/`\n` from the line end so restoring a cursor at the newline does not place it on the next line.
- Use `#[serde(rename_all = "kebab-case", default, deny_unknown_fields)]` on config-like structs and `#[serde(default)]` on `sidebar`/`search` in `Session`.
- Implement `Default` for `SidebarSession` and derive/`Default` for the rest.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p helix-view session::tests -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add helix-view/src/session.rs helix-view/src/lib.rs
git commit -m "feat(session): add session schema and position codec"
```

### Task 3: SessionStore with atomic writes and corruption recovery

**Files:**
- Modify: `helix-view/src/session.rs`, `helix-view/src/editor.rs`, `helix-view/Cargo.toml`
- Test: `helix-view/src/session.rs`

**Interfaces:**
- Produces:
  - `#[derive(Clone, Debug)] SessionStore`
  - `SessionStore::new(dir: PathBuf) -> Self`
  - `SessionStore::default() -> Self` (`helix_loader::data_dir().join("sessions")`)
  - `SessionStore::dir(&self) -> &Path`
  - `SessionStore::path_for(&self, workspace: &Path) -> PathBuf`
  - `SessionStore::save(&self, session: &Session) -> anyhow::Result<()>`
  - `SessionStore::load(&self, workspace: &Path) -> Option<Session>`
  - `SessionStore::clear(&self, workspace: &Path) -> anyhow::Result<()>`
  - `Editor.session_store: SessionStore` initialized to `SessionStore::default()`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn store_round_trips_and_backs_up_corrupt_files() {
    let tmp = tempfile::tempdir().unwrap();
    let store = SessionStore::new(tmp.path().to_path_buf());
    let workspace = tmp.path().join("ws");
    let session = Session { workspace: workspace.clone(), ..Session::default() };

    store.save(&session).unwrap();
    assert_eq!(store.load(&workspace), Some(session));

    std::fs::write(store.path_for(&workspace), b"{not json").unwrap();
    assert_eq!(store.load(&workspace), None);
    assert!(store.path_for(&workspace).with_extension("json.bak").exists());

    store.save(&Session { workspace: workspace.clone(), ..Session::default() }).unwrap();
    store.clear(&workspace).unwrap();
    assert!(!store.path_for(&workspace).exists());
}

#[test]
fn session_filename_is_stable_for_the_same_workspace() {
    let store = SessionStore::new("/tmp/sessions".into());
    let a = store.path_for(Path::new("/tmp/project"));
    let b = store.path_for(Path::new("/tmp/project"));
    let c = store.path_for(Path::new("/tmp/other"));
    assert_eq!(a, b);
    assert_ne!(a, c);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p helix-view session::tests::store -q`
Expected: FAIL — `SessionStore` missing.

- [ ] **Step 3: Implement the store**

Add `sha2 = "0.11"` to `helix-view/Cargo.toml`. `path_for` computes `Sha256::digest(workspace.as_os_str().to_string_lossy().as_bytes())` and returns `dir/<hex>.json`.

`save`:
1. `std::fs::create_dir_all(self.dir())`.
2. Serialize pretty JSON.
3. `tempfile::NamedTempFile::new_in(self.dir())`, write/flush.
4. `persist(path)` (replace the existing file; on Windows remove first if needed).
5. On error, return `anyhow::Error`; the old file remains.

`load`: read to string; on parse error or unsupported version, rename to `<path>.bak`, log `warn!`, return `None`. On missing file, return `None` without logging.

Add `session_store: SessionStore` to `Editor` and initialize with `SessionStore::default()` in `Editor::new`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p helix-view session::tests -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add helix-view/src/session.rs helix-view/src/editor.rs helix-view/Cargo.toml Cargo.lock
git commit -m "feat(session): add atomic per-workspace session store"
```

### Task 4: Collect and restore buffers, selections, and scroll

**Files:**
- Modify: `helix-view/src/session.rs`, `helix-view/src/editor.rs`, `helix-term/src/application.rs`
- Test: `helix-term/tests/test/session.rs` (new), `helix-term/tests/integration.rs`

**Interfaces:**
- Consumes: `Session`, `SessionStore`, `Editor` from Tasks 2–3.
- Produces:
  - `collect(editor: &Editor) -> Session`
  - `RestoreOutcome { restored: usize, skipped: Vec<PathBuf> }`
  - `restore(editor: &mut Editor, session: &Session) -> RestoreOutcome`
  - `Editor::save_session(&self) -> anyhow::Result<()>`
  - `Editor::clear_session(&self) -> anyhow::Result<()>`
  - `Editor::save_session_if_active(&self) -> anyhow::Result<()>`
  - `Application::new_with_session_store(args, config, lang_loader, workspace_trust, session_store) -> Result<Self, Error>`
  - `Application::new` delegates with `SessionStore::default()`
  - `fn workspace_root_from_args(args: &Args) -> PathBuf` (private)
- Paths inside `editor.workspace_root` are stored relative; paths outside are absolute. `restore` resolves them against the current root.

- [ ] **Step 1: Write the failing integration test**

Add `mod session;` to `helix-term/tests/integration.rs`, and create `helix-term/tests/test/session.rs`:

```rust
use helix_view::editor::Action;
use helix_view::session::{self, SessionStore};
use helix_view::view::ViewPosition;
use helix_view::{doc, view};

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn restore_round_trip_preserves_buffers_and_positions() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("main.rs");
    std::fs::write(&file, "alpha\nbeta\ngamma\n")?;

    let mut app = helpers::AppBuilder::new().with_file(&file, None).build()?;
    app.editor.workspace_root = dir.path().to_path_buf();
    {
        let (view, doc) = helix_view::current_mut!(app.editor);
        doc.set_selection(view.id, helix_core::Selection::single(2, 8));
        doc.set_view_offset(view.id, ViewPosition { anchor: 1, horizontal_offset: 0, vertical_offset: 0 });
    }

    let store = SessionStore::new(dir.path().join("state"));
    let saved = session::collect(&app.editor);
    store.save(&saved)?;

    let mut restored = helpers::AppBuilder::new()
        .with_session_store(store)
        .with_workspace_dir(dir.path())
        .build()?;
    assert_eq!(restored.editor.documents().count(), 1);
    let (view, doc) = helix_view::current_mut!(restored.editor);
    assert_eq!(doc.text().to_string(), "alpha\nbeta\ngamma\n");
    assert_eq!(doc.selection(view.id), &helix_core::Selection::single(2, 8));
    assert_eq!(doc.view_offset(view.id), ViewPosition { anchor: 1, horizontal_offset: 0, vertical_offset: 0 });
    Ok(())
}
```

Add to `helpers.rs`:
- `AppBuilder { session_store: Option<SessionStore>, ... }`.
- `with_session_store(mut self, store: SessionStore) -> Self`.
- `with_workspace_dir<P: Into<PathBuf>>(mut self, dir: P) -> Self` inserts the directory as the first `args.files` entry.
- In `build()`, call `Application::new_with_session_store(..., store)` when set, else `Application::new`.
- Remove the existing `bail!` that rejects a directory in `args.files[0]`; document that session tests need it.
- `test_config()`/`test_editor_config()` set `restore_session: false`, `auto_reload: false` (the latter field arrives in Task 7; add it then).

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p helix-term --features integration --test integration session::restore_round_trip -q`
Expected: FAIL — `collect`, `restore`, `SessionStore` builder are missing.

- [ ] **Step 3: Implement collect/restore**

`collect`:
- `workspace = editor.workspace_root.clone()`.
- Focused path: `let (view, doc) = helix_view::current_ref!(editor); view.doc` path.
- For each `doc` in `editor.documents()` with `Some(path)`:
  - If `doc.selections().get(&view.id)` exists, encode every `Range` with `encode_point`; otherwise `Selection::point(0)`.
  - `primary = selection.primary_index()`.
  - `scroll = doc.view_offset(view.id)` if a selection exists, else `ViewPosition::default()`; encode `anchor_line` with `text.char_to_line(offset.anchor)`.
  - Store relative path via `strip_prefix(&workspace).unwrap_or(path)`.
- `sidebar`: mode (`Hidden` allowed), width, expanded paths relative, selected relative.
- `search`: `editor.sidebar.search.clone()`.

`restore`:
- For each buffer: `decode_path`, skip missing files; `editor.open(&path, Action::Load)`; collect `(doc_id, BufferState)`.
- `view_id = view!(editor).id`.
- For each restored doc: map `SelectionState` to `Range::new(decode_point(...), decode_point(...))`, `Selection::new(ranges.into(), primary.min(len-1))`; `doc.set_selection(view_id, selection)`; `doc.set_view_offset(view_id, ViewPosition { anchor: text.line_to_char(scroll.anchor_line), ... })`.
- Switch to the saved focused doc if restored; else the first one; `view.ensure_cursor_in_view(doc, config.scrolloff)`.
- Apply `sidebar` fields (absolute paths) and `search`.
- Return `RestoreOutcome`.

`Editor::save_session`:
```rust
if !self.session_active || self.workspace_root.as_os_str().is_empty() {
    return Ok(());
}
self.session_store.save(&session::collect(self))
```
`clear_session` calls `session_store.clear(&self.workspace_root)`.
`save_session_if_active` checks `self.config().restore_session` and then calls `save_session`; `Application` uses this for automatic saves, while `:session-save` calls `save_session` directly so it still works when restore is disabled.

Application constructor (scaffolding for Task 5):
- Add `new_with_session_store` with the signature above; make `new` delegate with `SessionStore::default()`.
- `workspace_root_from_args`:
  1. first `args.files` key that is a directory → `helix_stdx::path::canonicalize`;
  2. else `args.working_directory` → canonicalize;
  3. else `helix_loader::find_workspace().0`.
- Set `editor.workspace_root`, `editor.session_store`, and `editor.session_active` (`args.files.is_empty() || first file is a dir`) before the first render. Do **not** restore or save yet; Task 5 adds that behavior.
- `AppBuilder` gets `with_session_store` and `with_workspace_dir`; remove the test harness's `bail!` for a directory in `args.files[0]`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p helix-term --features integration --test integration session::restore_round_trip -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add helix-view/src/session.rs helix-view/src/editor.rs helix-term/src/application.rs helix-term/tests/integration.rs helix-term/tests/test/session.rs helix-term/tests/test/helpers.rs
git commit -m "feat(session): collect and restore buffers, selections, and scroll"
```

### Task 5: Wire restore and save into Application

**Files:**
- Modify: `helix-term/src/application.rs`, `helix-term/src/handlers.rs`, create `helix-term/src/handlers/session.rs`, `helix-term/tests/test/session.rs`, `helix-term/tests/test/helpers.rs`

**Interfaces:**
- Consumes: `SessionStore`, `collect`, `restore`, `Editor::save_session`, and the `Application::new_with_session_store`/`workspace_root_from_args` scaffolding from Task 4.
- Produces:
  - `Editor.session_dirty` set by hooks on `DocumentDidOpen`, `DocumentDidClose`, `DocumentFocusLost`
  - `Application::close` saves the session before/after cleanup
  - `Application` idle saving with a five-second throttle

- [ ] **Step 1: Write the failing integration tests**

Add to `helix-term/tests/test/session.rs`:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn close_writes_session_and_explicit_files_do_not() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("one.txt");
    std::fs::write(&file, "one\n")?;
    let store = SessionStore::new(dir.path().join("state"));

    let mut app = helpers::AppBuilder::new()
        .with_session_store(store.clone())
        .with_workspace_dir(dir.path())
        .with_config(session_config())
        .build()?;
    app.editor.open(&file, Action::Load)?;
    app.close().await;
    assert!(store.load(dir.path()).is_some());

    let other = tempfile::tempdir()?;
    let file = other.path().join("two.txt");
    std::fs::write(&file, "two\n")?;
    let store2 = SessionStore::new(other.path().join("state"));
    let mut app = helpers::AppBuilder::new()
        .with_session_store(store2.clone())
        .with_file(&file, None)
        .with_config(session_config())
        .build()?;
    app.close().await;
    assert!(store2.load(other.path()).is_none());
    Ok(())
}
```

Add `session_config()` to `session.rs` tests (or helpers) returning a `Config` with `restore_session = true` and `auto_reload = false`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p helix-term --features integration --test integration session::close_writes -q`
Expected: FAIL — restore and save wiring missing.

- [ ] **Step 3: Implement the wiring**

- Add `last_session_save: std::time::Instant` to `Application`.
- After `Editor` is built and before the first picker:
  - if `session_active && config.editor.restore_session`: `if let Some(session) = editor.session_store.load(&editor.workspace_root) { let outcome = helix_view::session::restore(&mut editor, &session); restored = outcome.restored; ... }`.
  - set a status message; clear `editor.session_dirty`.
- Picker suppression: in the existing `if let Some((first, _)) = files_it.next_if(...)` block, only push the picker when `restored == 0`.
- Add `handlers/session.rs`:
```rust
pub(super) fn register_hooks(_handlers: &Handlers) {
    register_hook!(move |event: &mut DocumentDidOpen<'_>| { event.editor.session_dirty = true; Ok(()) });
    register_hook!(move |event: &mut DocumentDidClose<'_>| { event.editor.session_dirty = true; Ok(()) });
    register_hook!(move |event: &mut DocumentFocusLost<'_>| { event.editor.session_dirty = true; Ok(()) });
}
```
Register it in `handlers.rs`.
- `Application::close`: after cleanup, `if let Err(err) = self.editor.save_session_if_active() { log::warn!(...) }`.
- Idle save: in `handle_idle_timeout`, if `editor.session_dirty` and `last_session_save.elapsed() >= 5s`, call `save_session_if_active` and reset both.
- `Application::run` exits through `close`, so SIGINT/SIGTERM paths are covered.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p helix-term --features integration --test integration session:: -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add helix-term/src/application.rs helix-term/src/handlers.rs helix-term/src/handlers/session.rs helix-term/tests/test/session.rs helix-term/tests/test/helpers.rs
git commit -m "feat(session): restore on startup and save on exit"
```

### Task 6: Session commands and documentation

**Files:**
- Modify: `helix-term/src/commands/typed.rs`, `book/src/commands.md`, `book/src/configuration.md`
- Test: `helix-term/tests/test/session.rs`

**Interfaces:**
- Produces: `:session-save`, `:session-clear`.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test(flavor = "multi_thread")]
async fn session_commands_save_and_clear() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let store = SessionStore::new(dir.path().join("state"));
    let mut app = helpers::AppBuilder::new()
        .with_session_store(store.clone())
        .with_workspace_dir(dir.path())
        .with_config(session_config())
        .build()?;

    test_key_sequence(&mut app, Some(":session-save<ret>"), None, false).await?;
    assert!(store.load(dir.path()).is_some());

    test_key_sequence(&mut app, Some(":session-clear<ret>"), None, false).await?;
    assert!(store.load(dir.path()).is_none());
    Ok(())
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p helix-term --features integration --test integration session::session_commands -q`
Expected: FAIL — unknown commands.

- [ ] **Step 3: Implement**

Add typed command functions to `commands/typed.rs` that call `cx.editor.save_session()?` / `cx.editor.clear_session()?` and set a status message. Register in the `define_commands!` list. Document both in `book/src/commands.md` and `restore-session` in `book/src/configuration.md`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p helix-term --features integration --test integration session:: -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add helix-term/src/commands/typed.rs book/src/commands.md book/src/configuration.md helix-term/tests/test/session.rs
git commit -m "feat(session): add session-save and session-clear commands"
```

---

## Phase 2 — Auto-reload

### Task 7: FileWatcher service and config

**Files:**
- Create: `helix-term/src/file_watcher.rs`
- Modify: `helix-term/src/lib.rs`, `helix-term/Cargo.toml`, `helix-view/src/editor.rs`, `helix-term/tests/test/helpers.rs`
- Test: `helix-term/src/file_watcher.rs`

**Interfaces:**
- Produces:
  - `FileWatcher::new(debounce: Duration) -> anyhow::Result<(FileWatcher, tokio::sync::mpsc::UnboundedReceiver<Vec<PathBuf>>)>`
  - `FileWatcher::set_watch_set(&mut self, doc_dirs: HashSet<PathBuf>, sidebar_dirs: HashSet<PathBuf>)`
  - `FileWatcher::sync_documents(&mut self, editor: &Editor)`
  - `FileWatcher::sync_sidebar(&mut self, editor: &Editor)`
  - Config `auto_reload: bool` (default `true`), `auto_reload_debounce: u64` (default `300`)
  - `test_editor_config()` sets both `restore_session: false` and `auto_reload: false`

- [ ] **Step 1: Write the failing watcher test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn reports_writes_in_a_watched_directory() {
        let dir = tempfile::tempdir().unwrap();
        let (mut watcher, mut rx) = FileWatcher::new(Duration::from_millis(50)).unwrap();
        watcher.set_watch_set(HashSet::from([dir.path().to_path_buf()]), HashSet::new());

        let file = dir.path().join("a.txt");
        let tmp = dir.path().join("a.txt.tmp");
        std::fs::write(&tmp, "hello").unwrap();
        std::fs::rename(&tmp, &file).unwrap();
        let paths = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("watcher timed out")
            .expect("watcher channel closed");
        assert!(paths.iter().any(|p| p.ends_with("a.txt")));
    }
}
```

The test uses a temp-file rename (not an in-place write) so it pins the atomic-save case.

`set_watch_set` is the pure subscription diff; `sync_documents`/`sync_sidebar` only compute the two sets from `Editor` and call it. This keeps the watcher unit-testable without constructing an `Editor`.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p helix-term --lib file_watcher::tests -q`
Expected: FAIL — module and dependencies missing.

- [ ] **Step 3: Implement**

Add dependencies to `helix-term/Cargo.toml`:
```toml
notify = "8"
notify-debouncer-mini = "0.6"
```
(Use the current compatible versions; if the API differs, adapt to `new_debouncer(Duration, callback)`.)

Add `pub mod file_watcher;` to `helix-term/src/lib.rs`.

`FileWatcher` keeps:
- `debouncer: Debouncer<RecommendedWatcher>`
- `tx: UnboundedSender<Vec<PathBuf>>`
- `doc_dirs: HashSet<PathBuf>`, `sidebar_dirs: HashSet<PathBuf>`, `refcounts: HashMap<PathBuf, usize>`

`new` creates the debouncer with a callback that sends the event paths; `sync_documents` computes parent dirs of all documents with paths, diffs against `doc_dirs`, and calls `watch`/`unwatch`; `sync_sidebar` does the same for `workspace_root` plus `sidebar.expanded` when mode is `Files` (and no dirs when hidden/search). `watch` increments refcount and calls `debouncer.watcher().watch(dir, RecursiveMode::NonRecursive)` on first reference; `unwatch` decrements and calls `unwatch` at zero. Errors are logged, not fatal.

Add `auto_reload`/`auto_reload_debounce` to `helix-view` `Config` with defaults.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p helix-term --lib file_watcher::tests -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add helix-term/src/file_watcher.rs helix-term/src/lib.rs helix-term/Cargo.toml helix-view/src/editor.rs helix-term/tests/test/helpers.rs Cargo.lock
git commit -m "feat(reload): add debounced file watcher service"
```

### Task 8: Reload helper, saved-time getter, and conflict state

**Files:**
- Modify: `helix-view/src/editor.rs`, `helix-view/src/document.rs`, `helix-term/src/commands/typed.rs`
- Test: `helix-view/src/document.rs` (`#[cfg(test)]`) and `helix-term/tests/test/reload.rs` (new)

**Interfaces:**
- Produces:
  - `Document::last_saved_time(&self) -> SystemTime`
  - `ReloadConflict { disk_modified: SystemTime }`
  - `Editor.reload_conflicts: BTreeMap<DocumentId, ReloadConflict>`
  - `Editor::mark_reload_conflict(&mut self, doc_id, disk_modified)`
  - `Editor::clear_reload_conflict(&mut self, doc_id)`
  - `Editor::reload_conflict(&self, doc_id) -> Option<&ReloadConflict>`
  - `Editor::reload_document(&mut self, doc_id) -> anyhow::Result<()>`
  - `Editor::reload_all_documents(&mut self) -> Vec<anyhow::Error>`
- `typed::reload` and `typed::reload_all` delegate to these methods.

- [ ] **Step 1: Write the failing test**

Add `mod reload;` to `helix-term/tests/integration.rs` and create `helix-term/tests/test/reload.rs`:

```rust
use helix_view::editor::Action;
use helix_view::doc;

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn reload_document_refreshes_text_and_clears_modified() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "old\n")?;
    let mut app = helpers::AppBuilder::new().with_file(&file, None).build()?;
    test_key_sequence(&mut app, Some("inew<esc>"), None, false).await?;
    assert!(doc!(app.editor).is_modified());

    std::fs::write(&file, "disk\n")?;
    let doc_id = doc!(app.editor).id();
    app.editor.reload_document(doc_id)?;
    assert_eq!(doc!(app.editor).text().to_string(), "disk\n");
    assert!(!doc!(app.editor).is_modified());
    Ok(())
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p helix-term --features integration --test integration reload::reload_document -q`
Expected: FAIL — `reload_document` missing.

- [ ] **Step 3: Implement**

- Add `last_saved_time()` getter.
- Add `ReloadConflict` and the `reload_conflicts` map to `Editor`; initialize empty.
- Add mark/clear/get methods.
- Implement `reload_document` using the multi-view pattern from `typed::reload_all`: ensure at least the current view is initialized, reload via the first view, sync the remaining views with `view.sync_changes(doc)`, ensure cursor visibility, and notify `editor.language_servers.file_event_handler.file_changed(path)`.
- Implement `reload_all_documents` by looping over document ids.
- Replace the bodies of `typed::reload`/`typed::reload_all` with calls to these methods. Keep `PromptEvent::Validate` gating.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p helix-term --features integration --test integration reload::reload_document -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add helix-view/src/editor.rs helix-view/src/document.rs helix-term/src/commands/typed.rs helix-term/tests/integration.rs helix-term/tests/test/reload.rs
git commit -m "feat(reload): add document reload helper and conflict state"
```

### Task 9: Application watcher wiring and silent clean reload

**Files:**
- Modify: `helix-term/src/application.rs`, `helix-term/src/file_watcher.rs`
- Test: `helix-term/tests/test/reload.rs`

**Interfaces:**
- Consumes: `FileWatcher`, `Editor::reload_document`, conflict state.
- Produces:
  - `FileWatcher::apply_events(editor: &mut Editor, paths: &[PathBuf])`
  - Application field `file_watcher: Option<FileWatcher>` and `fs_events: UnboundedReceiver<Vec<PathBuf>>`
  - `select!` branch that drains filesystem events
  - Tree refresh event marking (`sidebar.refresh_dirs`, `sidebar.refresh_epoch`)

- [ ] **Step 1: Write the failing tests**

Add to `reload.rs`:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn clean_buffer_reloads_and_dirty_buffer_conflicts() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "old\n")?;
    let mut app = helpers::AppBuilder::new().with_file(&file, None).build()?;

    std::fs::write(&file, "new\n")?;
    file_watcher::apply_events(&mut app.editor, &[file.clone()]);
    assert_eq!(doc!(app.editor).text().to_string(), "new\n");

    test_key_sequence(&mut app, Some("ityped<esc>"), None, false).await?;
    let local = doc!(app.editor).text().to_string();
    std::fs::write(&file, "external\n")?;
    file_watcher::apply_events(&mut app.editor, &[file.clone()]);
    assert_eq!(doc!(app.editor).text().to_string(), local);
    assert!(app.editor.reload_conflict(doc!(app.editor).id()).is_some());
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn own_save_does_not_reload_or_conflict() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "text\n")?;
    let mut app = helpers::AppBuilder::new().with_file(&file, None).build()?;
    test_key_sequence(&mut app, Some(":w<ret>"), None, false).await?;
    file_watcher::apply_events(&mut app.editor, &[file]);
    assert!(!doc!(app.editor).is_modified());
    assert!(app.editor.reload_conflict(doc!(app.editor).id()).is_none());
    Ok(())
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p helix-term --features integration --test integration reload:: -q`
Expected: FAIL — `apply_events` and wiring missing.

- [ ] **Step 3: Implement**

`apply_events`:
- Canonicalize each path.
- If a document exists for the path:
  - missing file → status `deleted on disk`, continue;
  - `doc.is_modified()` → `mark_reload_conflict(doc_id, mtime)`, status hint, continue;
  - if `disk_content_matches(doc)` → `doc.pickup_last_saved_time()`, continue;
  - else `editor.reload_document(doc_id)`, status `reloaded from disk`.
- Add the canonical parent directory to `editor.sidebar.refresh_dirs`, increment `refresh_epoch`, and set `editor.needs_redraw = true`.
- `disk_content_matches`: use `std::fs::read_to_string`. If the file is not valid UTF-8 (or read fails), return `false` so the document is reloaded through its own encoding path; otherwise compare the string to `doc.text().to_string()`.

Application:
- Create the watcher in `new_with_session_store` when `config.editor.auto_reload`; store `(watcher, fs_events)`.
- Add `Some(paths) = self.fs_events.recv()` to the `select!` loop; call `sync_documents`, `sync_sidebar`, `apply_events`, then `render`.
- Call `sync_documents`/`sync_sidebar` after `DocumentSaved` and after terminal events, so save-as/rename subscriptions update.
- Expose a test-only way to inject events? Tests call `apply_events` directly; no OS-event flakiness.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p helix-term --features integration --test integration reload:: -q && cargo test -p helix-term --lib file_watcher::tests -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add helix-term/src/application.rs helix-term/src/file_watcher.rs helix-term/tests/test/reload.rs
git commit -m "feat(reload): auto-reload clean buffers on filesystem events"
```

### Task 10: Conflict prompt and save interception

**Files:**
- Create: `helix-term/src/ui/reload_conflict.rs`
- Modify: `helix-term/src/ui/mod.rs`, `helix-term/src/ui/editor.rs`, `helix-term/src/commands/typed.rs`
- Test: `helix-term/tests/test/reload.rs`

**Interfaces:**
- Produces:
  - `ReloadConflictPrompt::new(doc_id: DocumentId) -> Self` (a `Component`)
  - `commands::typed::write_document(cx, doc_id, force) -> anyhow::Result<()>` (pub(crate))
  - `EditorView::handle_idle_timeout` returns a callback that pushes the prompt when the focused doc has a pending conflict.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test(flavor = "multi_thread")]
async fn conflict_menu_reload_overwrite_and_keep() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "old\n")?;
    let mut app = helpers::AppBuilder::new().with_file(&file, None).build()?;
    test_key_sequence(&mut app, Some("ilocal<esc>"), None, false).await?;
    std::fs::write(&file, "disk\n")?;
    file_watcher::apply_events(&mut app.editor, &[file.clone()]);

    app.handle_idle_timeout().await; // pushes the menu
    test_key_sequence(&mut app, Some("j<ret>"), None, false).await?; // Keep -> Reload
    assert_eq!(doc!(app.editor).text().to_string(), "disk\n");
    assert!(!doc!(app.editor).is_modified());

    // Overwrite path: local edit wins and is written to disk.
    test_key_sequence(&mut app, Some("ilocal<esc>"), None, false).await?;
    let local_text = doc!(app.editor).text().to_string();
    std::fs::write(&file, "external\n")?;
    file_watcher::apply_events(&mut app.editor, &[file.clone()]);
    app.handle_idle_timeout().await;
    test_key_sequence(&mut app, Some("jj<ret>"), None, false).await?;
    assert_eq!(std::fs::read_to_string(&file)?, local_text);
    Ok(())
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p helix-term --features integration --test integration reload::conflict_menu -q`
Expected: FAIL — prompt missing.

- [ ] **Step 3: Implement**

- Create `ReloadConflictPrompt`:
  - `choice: usize = 0`; options `["Keep", "Reload", "Overwrite"]`.
  - `handle_event`: `j`/down, `k`/up, `Enter` confirm, `Esc` Keep.
  - On confirm: Keep → clear conflict; Reload → `editor.reload_document(doc_id)` + clear; Overwrite → `commands::typed::write_document(cx, doc_id, true)` + clear on success.
  - Return `EventResult::Consumed(Some(Box::new(|compositor, _| { compositor.pop(); })))`.
  - Render a centered bordered popup using theme `ui.popup`/`ui.menu.selected`.
- `EditorView::handle_idle_timeout`: if the focused doc has a conflict, return `EventResult::Consumed(Some(callback pushing the prompt))`; otherwise keep the existing inlay-hint behavior.
- `commands::typed::write_document`: switch to `doc_id` (`Action::Load`) if needed, then call the existing `write_impl` with `force`. Make `write_impl` check `editor.reload_conflict(doc_id)` and, unless `force`, bail with `file changed on disk; resolve the reload conflict first`.
- Register `ui::reload_conflict` in `ui/mod.rs`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p helix-term --features integration --test integration reload:: -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add helix-term/src/ui/reload_conflict.rs helix-term/src/ui/mod.rs helix-term/src/ui/editor.rs helix-term/src/commands/typed.rs helix-term/tests/test/reload.rs
git commit -m "feat(reload): add safe conflict prompt and save interception"
```

---

## Phase 3 — Sidebar and file tree

### Task 11: Sidebar pane, layout split, and focus routing

**Files:**
- Create: `helix-term/src/ui/sidebar.rs`
- Modify: `helix-term/src/ui/mod.rs`, `helix-term/src/ui/editor.rs`, `helix-term/src/commands.rs`, `helix-term/src/keymap/default.rs`, `helix-view/src/editor.rs`
- Test: `helix-term/tests/test/sidebar.rs` (new), `helix-term/tests/integration.rs`, `helix-view/src/sidebar.rs`

**Interfaces:**
- Consumes: `SidebarState`, `SidebarMode`, `SidebarFocus`, `SidebarConfig` from Task 1.
- Produces:
  - `Sidebar::new() -> Self`
  - `Sidebar::ensure_init(&mut self, editor: &mut Editor)`
  - `Sidebar::render(&mut self, area, surface, cx: &mut Context)`
  - `Sidebar::handle_event(&mut self, event, cx: &mut commands::Context) -> EventResult`
  - `EditorView.sidebar: Sidebar`
  - `commands::toggle_file_tree`, `commands::focus_search`
  - Keymap: `<space>b` → `toggle_file_tree`, `<space>B` → `buffer_picker`, `<space>/` → `focus_search`

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test(flavor = "multi_thread")]
async fn space_b_opens_files_pane_and_escape_returns_focus() -> anyhow::Result<()> {
    let mut app = helpers::AppBuilder::new().build()?;
    test_key_sequence(&mut app, Some("<space>b"), None, false).await?;
    assert_eq!(app.editor.sidebar.mode, SidebarMode::Files);
    assert_eq!(app.editor.sidebar.focus, SidebarFocus::Sidebar);

    test_key_sequence(&mut app, Some("<esc>"), None, false).await?;
    assert_eq!(app.editor.sidebar.focus, SidebarFocus::Editor);
    assert_eq!(app.editor.sidebar.mode, SidebarMode::Files);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn sidebar_keys_do_not_move_editor_cursor() -> anyhow::Result<()> {
    let mut app = helpers::AppBuilder::new().with_input_text("one\ntwo\n").build()?;
    let before = { let (v, d) = helix_view::current_ref!(app.editor); d.selection(v.id).clone() };
    test_key_sequence(&mut app, Some("<space>bjj"), None, false).await?;
    let after = { let (v, d) = helix_view::current_ref!(app.editor); d.selection(v.id).clone() };
    assert_eq!(before, after);
    Ok(())
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p helix-term --features integration --test integration sidebar:: -q`
Expected: FAIL — sidebar/commands missing.

- [ ] **Step 3: Implement**

- Create a placeholder `Sidebar` that renders a header and consumes `Esc`/`q` (focus editor), `<space>b` (hide), and `<space>/` (switch to Search). It does **not** own `FileTree` or `SearchPane` yet; Tasks 12 and 19 add those fields and extend this component.
- `EditorView`: add `sidebar` field; in `render`, compute `width = cx.editor.sidebar.visible_width(area.width, &cx.editor.config().sidebar)`; when `> 0`, render sidebar at `area.clip_bottom(1).with_width(width)`, set `editor_area = area.clip_bottom(1).clip_left(width)`, draw a divider at `width - 1`; otherwise use the current area. Bufferline uses `editor_area`.
- Theme styles use `try_get_exact` first, then explicit fallbacks: `ui.sidebar` → `ui.background`, `ui.sidebar.border` → `ui.linenr`, `ui.sidebar.title` → `ui.text`. This avoids the generic `ui` style masking the intended fallback.
- In `handle_event`, before normal keymaps, if `sidebar.focus == Sidebar && sidebar.mode != Hidden`, return `self.sidebar.handle_event(event, &mut cx)`. The sidebar consumes `j/k/h/l/...`; `Esc`/`q` set focus to Editor; `<space>b` hides; `<space>/` switches to Search.
- Commands `toggle_file_tree` and `focus_search` implement the state machine from the spec. Register in the `commands!` list.
- Keymap changes in `keymap/default.rs`.
- `Editor` should expose a helper `SidebarState::is_visible()` for tests and commands.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p helix-view sidebar::tests -q && cargo test -p helix-term --features integration --test integration sidebar:: -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add helix-term/src/ui/sidebar.rs helix-term/src/ui/mod.rs helix-term/src/ui/editor.rs helix-term/src/commands.rs helix-term/src/keymap/default.rs helix-view/src/editor.rs helix-view/src/sidebar.rs helix-term/tests/integration.rs helix-term/tests/test/sidebar.rs
git commit -m "feat(sidebar): add persistent pane, layout split, and focus routing"
```

### Task 12: FileTree model, directory listing, and rendering

**Files:**
- Create: `helix-term/src/ui/sidebar/file_tree.rs`
- Modify: `helix-term/src/ui/sidebar.rs`, `helix-term/src/ui/mod.rs`
- Test: `helix-term/src/ui/sidebar/file_tree.rs`

**Interfaces:**
- Consumes: `SidebarState`, `FileExplorerConfig`, `ignore::WalkBuilder`.
- Produces:
  - `EntryKind { File, Dir }`
  - `Row { path: PathBuf, depth: usize, kind: EntryKind, expanded: bool }`
  - `FileTree { root: PathBuf, rows: Vec<Row>, cursor: usize, scroll: usize }`
  - `FileTree::new(root: PathBuf) -> Self`
  - `FileTree::rebuild(&mut self, editor: &Editor, state: &SidebarState)`
  - `FileTree::rebuild_with(&mut self, root: &Path, expanded: &HashSet<PathBuf>, list: impl Fn(&Path) -> io::Result<Vec<(PathBuf, bool)>>)`
  - `FileTree::refresh(&mut self, editor: &Editor, state: &SidebarState)`
  - `FileTree::render(&mut self, area, surface, editor: &Editor, focused: bool)`
  - `ui::directory_entries(root: &Path, editor: &Editor) -> io::Result<Vec<(PathBuf, bool)>>`

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn list_dir(path: &Path) -> std::io::Result<Vec<(PathBuf, bool)>> {
        let mut entries: Vec<_> = std::fs::read_dir(path)?
            .map(|e| {
                let e = e?;
                Ok((e.path(), e.path().is_dir()))
            })
            .collect::<std::io::Result<_>>()?;
        entries.sort_by(|(a, ad), (b, bd)| (!ad, a.to_string_lossy().to_lowercase()).cmp(&(!bd, b.to_string_lossy().to_lowercase())));
        Ok(entries)
    }

    #[test]
    fn lists_directories_first_and_respects_expansion() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "").unwrap();
        std::fs::write(dir.path().join("README.md"), "").unwrap();

        let mut tree = FileTree::new(dir.path().to_path_buf());
        let mut expanded = HashSet::new();
        tree.rebuild_with(dir.path(), &expanded, list_dir);
        assert_eq!(tree.rows[0].kind, EntryKind::Dir);
        assert_eq!(tree.rows[0].path.file_name().unwrap(), "src");
        assert_eq!(tree.rows[1].path.file_name().unwrap(), "README.md");

        expanded.insert(dir.path().join("src"));
        tree.rebuild_with(dir.path(), &expanded, list_dir);
        assert_eq!(tree.rows.len(), 3);
        assert_eq!(tree.rows[1].depth, 1);
        assert_eq!(tree.rows[1].path.file_name().unwrap(), "main.rs");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p helix-term --lib ui::sidebar::file_tree -q`
Expected: FAIL — module missing.

- [ ] **Step 3: Implement**

- Add `pub(crate) fn directory_entries(root: &Path, editor: &Editor) -> io::Result<Vec<(PathBuf, bool)>>` in `ui/mod.rs`, reusing the existing `directory_content` walker configuration (hidden, ignore, gitignore, symlinks, `.helix/ignore`) with `max_depth(1)` and no `..` entry. Sort directories first, then case-insensitive file name.
- `FileTree::rebuild_with` walks `[root]` and, for each expanded path that is a descendant of a visible directory, appends its children at `depth + 1`. Keep it O(visible rows): only list directories present in `expanded`. `FileTree::rebuild(editor, state)` is a thin wrapper that calls `rebuild_with(root, &state.expanded, |p| directory_entries(p, editor))`.
- `refresh` remembers `rows[cursor].path`, rebuilds, then finds the same path; if missing, select the nearest row at the same or shallower depth.
- `render`: draw the header from `root.file_name()`, then `rows[scroll..]` clipped to `area.height - 1`; indent with `"  ".repeat(depth)` and `▸`/`▾` for dirs; use `ui.tree.directory`/`ui.tree.selected` with `try_get_exact` fallbacks to `ui.text.directory`/`ui.selection`.
- `Sidebar::render` calls `tree.render` in Files mode.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p helix-term --lib ui::sidebar::file_tree -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add helix-term/src/ui/sidebar/file_tree.rs helix-term/src/ui/sidebar.rs helix-term/src/ui/mod.rs
git commit -m "feat(tree): add lazy flattened file tree and rendering"
```

### Task 13: Tree navigation, open, splits, and reveal

**Files:**
- Modify: `helix-term/src/ui/sidebar/file_tree.rs`, `helix-term/src/ui/sidebar.rs`
- Test: `helix-term/tests/test/sidebar.rs`

**Interfaces:**
- Produces: `FileTree::handle_key(&mut self, key: KeyEvent, cx: &mut commands::Context) -> bool` handling the Files keymap from the spec.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test(flavor = "multi_thread")]
async fn tree_navigation_opens_files_and_moves_focus() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("a.txt"), "alpha\n")?;
    std::fs::write(dir.path().join("b.txt"), "beta\n")?;
    let mut app = helpers::AppBuilder::new()
        .with_workspace_dir(dir.path())
        .build()?;

    test_key_sequence(&mut app, Some("<space>b"), None, false).await?;
    test_key_sequence(&mut app, Some("<ret>"), None, false).await?;
    assert_eq!(app.editor.sidebar.focus, SidebarFocus::Editor);
    assert!(doc!(app.editor).path().unwrap().ends_with("a.txt"));
    Ok(())
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p helix-term --features integration --test integration sidebar::tree_navigation -q`
Expected: FAIL — navigation not implemented.

- [ ] **Step 3: Implement**

- `j/k`, arrows: move cursor with scroll bounds; update `state.selected` to the new row path.
- `h`: collapse if expanded; else select parent row (`depth - 1`).
- `l`: expand a directory (insert to `state.expanded`, rebuild) or open a file.
- `Enter`/`o`: `cx.editor.open(path, Action::Load)`, focus editor, ensure cursor visible.
- `s`/`v`: `Action::HorizontalSplit` / `Action::VerticalSplit`, focus editor.
- `g/G`, `Ctrl-d/Ctrl-u`: first/last/half-page.
- `H`: clear `state.expanded` and rebuild.
- `R`: refresh.
- `.`: reveal current buffer: expand ancestors, rebuild, select the row.
- `[`/`]`: adjust `state.width` by 2, clamp to config bounds.
- `q`/`Esc`: focus editor. `<space>b`: hide. `<space>/`: switch to Search.
- For a text input prompt, printable keys edit; this is handled by the prompt component, not the tree.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p helix-term --features integration --test integration sidebar:: -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add helix-term/src/ui/sidebar/file_tree.rs helix-term/src/ui/sidebar.rs helix-term/tests/test/sidebar.rs
git commit -m "feat(tree): add navigation, open, splits, and reveal"
```

### Task 14: Tree file operations

**Files:**
- Modify: `helix-term/src/ui/sidebar/file_tree.rs`
- Test: `helix-term/tests/test/sidebar.rs`

**Interfaces:**
- Produces: keys `a`, `A`, `r`, `d`, `y`, `Y` with prompts and confirmations.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test(flavor = "multi_thread")]
async fn create_rename_and_delete_from_tree() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let mut app = helpers::AppBuilder::new().with_workspace_dir(dir.path()).build()?;

    test_key_sequence(&mut app, Some("<space>banew<ret>"), None, false).await?;
    assert!(dir.path().join("new").is_file());

    test_key_sequence(&mut app, Some("r<C-u>renamed<ret>"), None, false).await?;
    assert!(dir.path().join("renamed").is_file());
    assert!(!dir.path().join("new").exists());

    // The delete confirmation defaults to No; `j` selects Yes.
    test_key_sequence(&mut app, Some("dj<ret>"), None, false).await?;
    assert!(!dir.path().join("renamed").exists());
    Ok(())
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p helix-term --features integration --test integration sidebar::create_rename -q`
Expected: FAIL — operations missing.

- [ ] **Step 3: Implement**

- `a`: `ui::prompt` "New file:" with parent = selected dir or selected file's parent; validate a single non-empty component; create file; open it; rebuild.
- `A`: same for directory.
- `r`: `ui::prompt_with_input` prefilled with the file name; rename; if the path is open in a buffer, call `editor.set_doc_path(doc_id, &new_path)`; update `state.selected` and parent; refresh.
- `d`: use `ui::Select` with options `[No, Yes]` (default No); on Yes delete the file/dir (`remove_dir_all` for dirs). Refuse if an open buffer is modified. Report errors in the statusline.
- `y`/`Y`: `editor.registers.write('+', vec![absolute_or_relative_path])` and set status.
- After each operation, update `session_dirty = true` and rebuild.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p helix-term --features integration --test integration sidebar:: -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add helix-term/src/ui/sidebar/file_tree.rs helix-term/tests/test/sidebar.rs
git commit -m "feat(tree): add create, rename, delete, and copy-path operations"
```

### Task 15: Tree refresh from filesystem events

**Files:**
- Modify: `helix-term/src/ui/sidebar.rs`, `helix-term/src/ui/sidebar/file_tree.rs`, `helix-term/src/application.rs`
- Test: `helix-term/tests/test/sidebar.rs`

**Interfaces:**
- Consumes: `sidebar.refresh_dirs`, `sidebar.refresh_epoch` populated by Task 9.
- Produces:
  - Sidebar refresh when the epoch changes; no full-tree walk.
  - `#[cfg(feature = "integration")] Application::sidebar_row_paths(&mut self) -> Vec<PathBuf>` (test hook delegating to `EditorView::sidebar_row_paths`).

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test(flavor = "multi_thread")]
async fn tree_refreshes_after_external_create() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let mut app = helpers::AppBuilder::new().with_workspace_dir(dir.path()).build()?;
    test_key_sequence(&mut app, Some("<space>b"), None, false).await?;

    let before = app.editor.sidebar.refresh_epoch;
    std::fs::write(dir.path().join("appeared.txt"), "x")?;
    file_watcher::apply_events(&mut app.editor, &[dir.path().join("appeared.txt")]);
    assert!(app.editor.sidebar.refresh_epoch > before);

    run_event_loop_until_idle(&mut app).await;
    assert!(app.sidebar_row_paths().iter().any(|p| p.ends_with("appeared.txt")));
    Ok(())
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p helix-term --features integration --test integration sidebar::tree_refreshes -q`
Expected: FAIL — refresh not wired.

- [ ] **Step 3: Implement**

- `Sidebar` keeps `last_refresh_epoch: u64`; in `render` (or `handle_idle_timeout`) when `editor.sidebar.refresh_epoch != last_refresh_epoch`, call `tree.refresh(editor, &editor.sidebar)` and clear `refresh_dirs`.
- Application calls `file_watcher.sync_sidebar(&editor)` after document/save events and after filesystem events, so visible/expanded dirs are watched.
- `apply_events` already increments `refresh_epoch` and inserts parent dirs.
- Add `EditorView::sidebar_row_paths(&self) -> Vec<PathBuf>` (returns `self.sidebar.tree.rows.iter().map(|r| r.path.clone())`) and the `#[cfg(feature = "integration")] Application::sidebar_row_paths(&mut self)` hook used by the test.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p helix-term --features integration --test integration sidebar:: -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add helix-term/src/ui/sidebar.rs helix-term/src/ui/sidebar/file_tree.rs helix-term/src/application.rs helix-term/tests/test/sidebar.rs
git commit -m "feat(tree): refresh visible directories on filesystem events"
```

### Task 16: Persist sidebar state and document the keymap

**Files:**
- Modify: `helix-term/tests/test/session.rs`, `book/src/keymap.md`, `book/src/configuration.md`, `helix-view/src/session.rs` (only if a field is missing)
- Test: `helix-term/tests/test/session.rs`

**Interfaces:**
- Consumes: sidebar fields in `collect`/`restore` from Task 4.
- Produces: round-trip coverage and docs for `<space>b`, `<space>B`, `<space>/`, and the tree keymap.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test(flavor = "multi_thread")]
async fn sidebar_state_round_trips() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("a.txt"), "a\n")?;
    let mut app = helpers::AppBuilder::new()
        .with_workspace_dir(dir.path())
        .with_config(session_config())
        .build()?;
    app.editor.sidebar.mode = SidebarMode::Files;
    app.editor.sidebar.width = 34;
    app.editor.sidebar.expanded.insert(dir.path().to_path_buf());
    app.editor.sidebar.selected = Some(dir.path().join("a.txt"));

    let saved = helix_view::session::collect(&app.editor);
    assert_eq!(saved.sidebar.width, 34);
    assert_eq!(saved.sidebar.mode, SidebarMode::Files);
    assert!(saved.sidebar.selected.is_some());
    Ok(())
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p helix-term --features integration --test integration session::sidebar_state -q`
Expected: FAIL if collect does not yet persist all fields.

- [ ] **Step 3: Implement**

- Ensure `collect`/`restore` encode/decode all sidebar fields; if Task 4 left any out, add them now.
- Document the keys in `book/src/keymap.md` and `[editor.sidebar]` in `book/src/configuration.md`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p helix-term --features integration --test integration session:: -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add helix-view/src/session.rs helix-term/tests/test/session.rs book/src/keymap.md book/src/configuration.md
git commit -m "feat(session): persist sidebar state and document tree keymap"
```

---

## Phase 4 — Global search and replace

### Task 17: Search engine module

**Files:**
- Create: `helix-term/src/search.rs`
- Modify: `helix-term/src/lib.rs`, `helix-term/Cargo.toml`
- Test: `helix-term/src/search.rs`

**Interfaces:**
- Produces:
  - `#[derive(Clone, Copy, Debug, Default)] SearchOptions { match_case: bool, whole_word: bool, regex: bool, preserve_case: bool }`
  - `Match { path: PathBuf, line: usize, col: usize, start: usize, end: usize, text: String, preview: String }`
  - `SearchEvent { Match(Match), Done { capped: bool }, Error(String) }`
  - `build_regex(query: &str, options: SearchOptions) -> Result<regex::Regex, regex::Error>`
  - `find_matches(text: &str, regex: &regex::Regex) -> Vec<(usize, usize, String)>`
  - `spawn_search(root: PathBuf, query: String, options: SearchOptions, open_docs: Vec<(PathBuf, String)>, tx: std::sync::mpsc::Sender<SearchEvent>, max_results: usize)`
  - `replacement_for(caps: Option<&regex::Captures<'_>>, matched: &str, template: &str, options: SearchOptions) -> String`
  - `apply_replacements_to_text(text: &str, regex: &regex::Regex, template: &str, options: SearchOptions) -> (String, usize)`
  - `write_atomic(path: &Path, contents: &str) -> anyhow::Result<()>`

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn opts(match_case: bool, whole_word: bool, regex: bool) -> SearchOptions {
        SearchOptions { match_case, whole_word, regex, preserve_case: false }
    }

    #[test]
    fn literal_case_whole_word_and_regex_modes() {
        let literal = build_regex("foo", opts(false, false, false)).unwrap();
        assert!(literal.is_match("FOO"));
        let word = build_regex("foo", opts(false, true, false)).unwrap();
        assert!(word.is_match("a foo b"));
        assert!(!word.is_match("foobar"));
        let re = build_regex(r"f(o+)", opts(true, false, true)).unwrap();
        assert_eq!(find_matches("foo FOo", &re).len(), 1);
    }

    #[test]
    fn replacement_expands_captures_and_preserves_case() {
        let re = build_regex(r"(\w+)", opts(false, false, true)).unwrap();
        let caps = re.captures("hello").unwrap();
        assert_eq!(replacement_for(Some(&caps), "hello", "[$1]", opts(false, false, true)), "[hello]");
        assert_eq!(
            replacement_for(Some(&caps), "HELLO", "world", SearchOptions { preserve_case: true, ..opts(false, false, false) }),
            "WORLD"
        );
    }

    #[test]
    fn replacement_keeps_crlf_line_endings() {
        let re = build_regex("a", opts(false, false, false)).unwrap();
        let (out, count) = apply_replacements_to_text("a\r\nb\r\n", &re, "x\ny", opts(false, false, false));
        assert_eq!(count, 1);
        assert_eq!(out, "x\r\ny\r\nb\r\n");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p helix-term --lib search::tests -q`
Expected: FAIL — module missing.

- [ ] **Step 3: Implement**

- Add `regex = "1"` to `helix-term/Cargo.toml`; add `pub mod search;` to `lib.rs`.
- `build_regex`: literal mode uses `regex::escape`; whole-word wraps in `\b(?:...)\b`; `RegexBuilder::new(...).case_insensitive(!match_case).multi_line(true).build()`.
- `find_matches`: `regex.find_iter(text)`, return `(start, end, text[start..end].to_string())`.
- `replacement_for`: in regex mode, `caps.expand(template, &mut out)`; literal mode returns `template` with `$` untouched. Preserve-case: if matched is all uppercase → uppercase replacement; if first char uppercase and rest lowercase → capitalize; else leave.
- `apply_replacements_to_text`: collect matches, replace from the end (or build ascending with `String` slices), count. Normalize `\n` in the replacement to `\r\n` when the source contains `\r\n`.
- `write_atomic`: `tempfile::NamedTempFile::new_in(parent)`, write, flush, preserve permissions from the existing file, `persist(path)`.
- `spawn_search`: search `open_docs` text first, then walk the workspace with `ignore::WalkBuilder::build_parallel` + `grep_searcher::Searcher`, skipping any path already present in `open_docs` so results are not duplicated. For each matched line, run `find_matches` on the line content and emit one `Match` per submatch with absolute byte offset (`line_start_byte + match.start`). Enforce `max_results`; when reached, send `Done { capped: true }` and stop injecting. Send `Done { capped: false }` at the end. Call `helix_event::request_redraw()` after each batch.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p helix-term --lib search::tests -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add helix-term/src/search.rs helix-term/src/lib.rs helix-term/Cargo.toml Cargo.lock
git commit -m "feat(search): add ripgrep-backed search and replacement engine"
```

### Task 18: TextInput widget

**Files:**
- Create: `helix-term/src/ui/text_input.rs`
- Modify: `helix-term/src/ui/mod.rs`
- Test: `helix-term/src/ui/text_input.rs`

**Interfaces:**
- Produces:
  - `TextInput::new(value: impl Into<String>) -> Self`
  - `TextInput::value(&self) -> &str`
  - `TextInput::set_value(&mut self, value: impl Into<String>)`
  - `TextInput::cursor(&self) -> usize` (byte index at a grapheme boundary)
  - `TextInput::handle_key(&mut self, key: KeyEvent) -> bool`
  - `TextInput::render(&self, area, surface, theme: &Theme, focused: bool)`

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use helix_view::input::KeyEvent;

    fn key(code: KeyCode) -> KeyEvent { KeyEvent { code, modifiers: KeyModifiers::NONE } }
    fn ctrl(c: char) -> KeyEvent { KeyEvent { code: KeyCode::Char(c), modifiers: KeyModifiers::CONTROL } }

    #[test]
    fn inserts_and_moves_by_grapheme() {
        let mut input = TextInput::new("");
        for c in "aβ🎉".chars() {
            assert!(input.handle_key(key(KeyCode::Char(c))));
        }
        input.handle_key(key(KeyCode::Home));
        input.handle_key(key(KeyCode::Right));
        input.handle_key(key(KeyCode::Char('x')));
        assert_eq!(input.value(), "axβ🎉");
        assert_eq!(input.cursor(), "ax".len());
    }

    #[test]
    fn delete_word_and_clear() {
        let mut input = TextInput::new("one two three");
        input.handle_key(ctrl('w'));
        assert_eq!(input.value(), "one two ");
        input.handle_key(ctrl('u'));
        assert_eq!(input.value(), "");
    }
}
```

Adjust the test constructors to the actual `helix_view::input::KeyEvent` API.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p helix-term --lib ui::text_input -q`
Expected: FAIL — widget missing.

- [ ] **Step 3: Implement**

- Store `value: String`, `cursor: usize` (byte index).
- Use `helix_core::graphemes::{next_grapheme_boundary, prev_grapheme_boundary}` for movement/deletion.
- Handle printable chars, Backspace, Delete, Left/Right, Home/End, `Ctrl-w`/`Ctrl-Backspace`, `Ctrl-u`.
- `render` draws the value and a cursor cell when focused.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p helix-term --lib ui::text_input -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add helix-term/src/ui/text_input.rs helix-term/src/ui/mod.rs
git commit -m "feat(search): add grapheme-aware single-line input widget"
```

### Task 19: Search pane UI and streaming results

**Files:**
- Create: `helix-term/src/ui/sidebar/search.rs`
- Modify: `helix-term/src/ui/sidebar.rs`, `helix-term/src/ui/mod.rs`
- Test: `helix-term/tests/test/search.rs` (new), `helix-term/tests/integration.rs`

**Interfaces:**
- Consumes: `search` engine (Task 17), `TextInput` (Task 18), `SearchInputs`.
- Produces:
  - `SearchFocus { Query, Replace, Results }`
  - `SearchPane { query: TextInput, replacement: TextInput, focus: SearchFocus, results: BTreeMap<PathBuf, FileResults>, rows: Vec<ResultRow>, cursor: usize, scroll: usize, ... }`
  - `SearchPane::new(inputs: &SearchInputs) -> Self`
  - `SearchPane::render(&mut self, area, surface, editor: &Editor)`
  - `SearchPane::handle_event(&mut self, event, cx: &mut commands::Context) -> EventResult`
  - `SearchPane::handle_idle_timeout(&mut self, editor: &Editor)`
  - `#[cfg(feature = "integration")] Application::search_result_count(&mut self) -> usize` (test hook delegating to `EditorView::search_result_count`)

- [ ] **Step 1: Write the failing integration test**

Add `mod search;` to `integration.rs`; create `helix-term/tests/test/search.rs`:

```rust
async fn wait_for_search_results(app: &mut Application, expected: usize) {
    for _ in 0..40 {
        run_event_loop_until_idle(app).await;
        if app.search_result_count() >= expected {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("search results did not arrive");
}

#[tokio::test(flavor = "multi_thread")]
async fn search_pane_finds_and_opens_a_match() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("a.txt"), "zero\nneedle\n")?;
    let mut app = helpers::AppBuilder::new().with_workspace_dir(dir.path()).build()?;

    test_key_sequence(&mut app, Some("<space>/needle"), None, false).await?;
    wait_for_search_results(&mut app, 1).await; // idle starts the debounced search
    test_key_sequence(&mut app, Some("<ret>"), None, false).await?; // query -> results
    test_key_sequence(&mut app, Some("<ret>"), None, false).await?; // open first match

    let (view, doc) = helix_view::current_ref!(app.editor);
    assert!(doc.path().unwrap().ends_with("a.txt"));
    assert_eq!(doc.selection(view.id).primary().from(), 5); // start of "needle"
    Ok(())
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p helix-term --features integration --test integration search::search_pane -q`
Expected: FAIL — pane missing.

- [ ] **Step 3: Implement**

- `SearchPane::new` seeds inputs from `SearchInputs`; focus starts at Query.
- Input editing: `TextInput::handle_key`; on change, update `editor.sidebar.search` and set `pending = true`.
- `handle_idle_timeout`: if `pending`, build options, snapshot open docs (`editor.documents()` with paths and text), spawn `search::spawn_search` on a `std::thread` with `max_results = 10_000`, replace `receiver` with the new channel; old receiver drop cancels the old search. 150–250 ms debounce is provided by Helix's idle timer. If `build_regex` fails, store an inline error and keep the previous results marked stale instead of starting a search.
- `render`: drain `try_recv` with a budget (500 events/frame), insert matches into `BTreeMap<PathBuf, FileResults>`, rebuild `rows`, handle `Done { capped }`, then draw query/replace inputs, toggles, and visible rows. File groups are expanded by default.
- The search thread calls `helix_event::request_redraw()` after pushing each batch. `Editor::wait_event` already listens for `helix_event::redraw_requested()`, so the event loop wakes and `render` drains the channel.
- `handle_event`: route keys by focus; `Tab` cycles Query → Replace → Results; `Esc` from Query → Results, from Results → Editor; `Alt-c/w/r/p` toggles and sets `pending = true`; `j/k/h/l` navigate results; `<space>b` from Results hides the sidebar.
- `Sidebar::render`/`handle_event` dispatch to `SearchPane` in Search mode.
- `<space>/` command (`focus_search`) already switches mode; `Sidebar::ensure_init` focuses Query when entering Search.
- Add `EditorView::search_result_count(&self) -> usize` and the `#[cfg(feature = "integration")] Application::search_result_count(&mut self)` hook for deterministic tests.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p helix-term --features integration --test integration search::search_pane -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add helix-term/src/ui/sidebar/search.rs helix-term/src/ui/sidebar.rs helix-term/src/ui/mod.rs helix-term/tests/integration.rs helix-term/tests/test/search.rs
git commit -m "feat(search): add streaming search pane with toggles"
```

### Task 20: Search navigation, open-at-match, and select-all

**Files:**
- Modify: `helix-term/src/ui/sidebar/search.rs`, `helix-term/src/commands.rs`
- Test: `helix-term/tests/test/search.rs`

**Interfaces:**
- Produces:
  - `SearchPane::open_match(&mut self, editor: &mut Editor, file: &Path, start: usize, end: usize) -> anyhow::Result<()>`
  - `SearchPane::select_all_in_file(&mut self, editor: &mut Editor) -> anyhow::Result<()>`
  - keys `Enter`, `s`, `v`, `n`/`N`, `F4`/`Shift-F4`, `Alt-Enter`.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test(flavor = "multi_thread")]
async fn select_all_matches_in_file_creates_multiple_selections() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("a.txt"), "x\nx\nx\n")?;
    let mut app = helpers::AppBuilder::new().with_workspace_dir(dir.path()).build()?;
    test_key_sequence(&mut app, Some("<space>/x"), None, false).await?;
    wait_for_search_results(&mut app, 3).await;
    test_key_sequence(&mut app, Some("<ret>"), None, false).await?;
    test_key_sequence(&mut app, Some("<A-ret>"), None, false).await?;

    let (view, doc) = helix_view::current_ref!(app.editor);
    assert_eq!(doc.selection(view.id).ranges().len(), 3);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn unicode_match_opens_at_char_offsets() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("u.txt"), "αβγ\n")?;
    let mut app = helpers::AppBuilder::new().with_workspace_dir(dir.path()).build()?;

    test_key_sequence(&mut app, Some("<space>/β"), None, false).await?;
    wait_for_search_results(&mut app, 1).await;
    test_key_sequence(&mut app, Some("<ret><ret>"), None, false).await?;

    let (view, doc) = helix_view::current_ref!(app.editor);
    assert_eq!(doc.selection(view.id).primary().from(), 1);
    assert_eq!(doc.selection(view.id).primary().to(), 2);
    Ok(())
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p helix-term --features integration --test integration search::select_all -q`
Expected: FAIL.

- [ ] **Step 3: Implement**

- `open_match`: resolve path, `editor.open(path, Action::Load)`, convert byte offsets to chars (`doc.text().byte_to_char`), `doc.set_selection(view_id, Selection::single(start, end))`, `view.ensure_cursor_in_view_center(doc, scrolloff)`, focus editor.
- Splits: use `Action::HorizontalSplit`/`VerticalSplit`.
- `n`/`N`/`F4`/`Shift-F4`: move to next/previous result row, call `open_match`.
- `Alt-Enter`: gather all matches for the selected file, build a `Selection` with one `Range` per match (`Selection::new(ranges.into(), 0)`), set it, focus editor.
- Ensure a match row highlights the exact match range in the preview (render with a highlight span).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p helix-term --features integration --test integration search:: -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add helix-term/src/ui/sidebar/search.rs helix-term/src/commands.rs helix-term/tests/test/search.rs
git commit -m "feat(search): add navigation, open-at-match, and select-all"
```

### Task 21: Replace engine for open and unopened files

**Files:**
- Modify: `helix-term/src/search.rs`, `helix-view/src/editor.rs` (only if a helper is needed)
- Test: `helix-term/src/search.rs`, `helix-term/tests/test/search.rs`

**Interfaces:**
- Produces:
  - `ReplaceOutcome { replaced: usize, files: usize, skipped: Vec<PathBuf> }`
  - `replace_in_open_document(editor: &mut Editor, doc_id: DocumentId, regex: &regex::Regex, template: &str, options: SearchOptions) -> anyhow::Result<usize>`
  - `replace_in_file(path: &Path, regex: &regex::Regex, template: &str, options: SearchOptions) -> anyhow::Result<usize>`

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn replace_in_file_rereads_before_writing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    std::fs::write(&path, "foo\n").unwrap();
    let re = build_regex("foo", SearchOptions { match_case: true, ..SearchOptions::default() }).unwrap();

    // The file changes after the query was built; replacement must use the current text.
    std::fs::write(&path, "xfoo\n").unwrap();
    assert_eq!(replace_in_file(&path, &re, "bar", SearchOptions::default()).unwrap(), 1);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "xbar\n");
}

#[test]
fn replace_in_file_skips_binary_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bin");
    let original = [0xff_u8, 0xfe, b'f', b'o', b'o'];
    std::fs::write(&path, original).unwrap();
    let re = build_regex("foo", SearchOptions::default()).unwrap();
    assert_eq!(replace_in_file(&path, &re, "bar", SearchOptions::default()).unwrap(), 0);
    assert_eq!(std::fs::read(&path).unwrap(), original);
}
```

Add an integration test for open buffers:

```rust
#[tokio::test(flavor = "multi_thread")]
async fn replace_all_updates_open_buffer_in_memory_and_disk_files() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let open = dir.path().join("open.txt");
    let closed = dir.path().join("closed.txt");
    std::fs::write(&open, "foo\n")?;
    std::fs::write(&closed, "foo\n")?;
    let mut app = helpers::AppBuilder::new().with_file(&open, None).build()?;
    app.editor.workspace_root = dir.path().to_path_buf();

    let re = search::build_regex("foo", search::SearchOptions::default())?;
    let doc_id = doc!(app.editor).id();
    let n = search::replace_in_open_document(&mut app.editor, doc_id, &re, "bar", search::SearchOptions::default())?;
    assert_eq!(n, 1);
    assert!(doc!(app.editor).is_modified());
    search::replace_in_file(&closed, &re, "bar", search::SearchOptions::default())?;
    assert_eq!(std::fs::read_to_string(&closed)?, "bar\n");
    Ok(())
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p helix-term --lib search::tests::replace -q`
Expected: FAIL — replace functions missing.

- [ ] **Step 3: Implement**

- `replace_in_open_document`: re-run `find_matches` against the current `doc.text()`, build `Change`s ascending `(from_char, to_char, Some(replacement))` via `Transaction::change`, `doc.apply(&tx, view_id)`, `doc.append_changes_to_history(view)`. Return count.
- `replace_in_file`: read to string (skip non-UTF8/binary), re-run `find_matches` on the current text, apply with `apply_replacements_to_text`, and write with `write_atomic`. Return count; skip if zero.
- Preserve line endings and permissions; `write_atomic` handles both.
- Stale results: the UI passes only the file path; the function always re-reads and re-searches, so offsets cannot be stale.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p helix-term --lib search::tests -q && cargo test -p helix-term --features integration --test integration search::replace -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add helix-term/src/search.rs helix-view/src/editor.rs helix-term/tests/test/search.rs
git commit -m "feat(search): apply replacements to open buffers and disk files atomically"
```

### Task 22: Search replace keys, commands, persistence, and docs

**Files:**
- Modify: `helix-term/src/ui/sidebar/search.rs`, `helix-term/src/commands.rs`, `helix-term/src/commands/typed.rs`, `helix-view/src/session.rs` (if needed), `book/src/keymap.md`, `book/src/commands.md`, `book/src/configuration.md`
- Test: `helix-term/tests/test/search.rs`, `helix-term/tests/test/session.rs`

**Interfaces:**
- Produces:
  - `Ctrl-r` on a match → replace one; on a file row → replace all in that file.
  - `Ctrl-Alt-Enter` → replace all in the workspace.
  - `:search-replace-all`, `:search-replace-current`.
  - Search inputs persist in the session.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test(flavor = "multi_thread")]
async fn replace_all_from_search_pane_updates_disk_and_reports() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    std::fs::write(&a, "foo\n")?;
    std::fs::write(&b, "foo\n")?;
    let mut app = helpers::AppBuilder::new().with_workspace_dir(dir.path()).build()?;

    test_key_sequence(&mut app, Some("<space>/foo"), None, false).await?;
    run_event_loop_until_idle(&mut app).await;
    test_key_sequence(&mut app, Some("<ret><tab><tab>bar"), None, false).await?;
    test_key_sequence(&mut app, Some("<C-A-ret>"), None, false).await?;

    assert_eq!(std::fs::read_to_string(&a)?, "bar\n");
    assert_eq!(std::fs::read_to_string(&b)?, "bar\n");
    assert!(app.editor.get_status().is_some());
    Ok(())
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p helix-term --features integration --test integration search::replace_all -q`
Expected: FAIL — keys/commands missing.

- [ ] **Step 3: Implement**

- In `SearchPane`:
  - `Ctrl-r` on a match: open/revalidate and replace that one match in the open buffer.
  - `Ctrl-r` on a file header: replace all matches in that file (open buffer or disk).
  - `Enter` in Replace input: replace selected/next match.
  - `Ctrl-Alt-Enter`: iterate all result files, call `replace_in_open_document` or `replace_in_file`; count; set summary status.
- Add typed commands `search-replace-all` and `search-replace-current` in `commands/typed.rs` that read `cx.editor.sidebar.search` and call the same helpers. Register them.
- Persist `editor.sidebar.search` in `collect`/`restore`; on pane input/toggle changes, write back to it.
- Document all search keys and commands.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p helix-term --features integration --test integration search:: -q && cargo test -p helix-term --features integration --test integration session:: -q`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add helix-term/src/ui/sidebar/search.rs helix-term/src/commands.rs helix-term/src/commands/typed.rs helix-view/src/session.rs helix-term/tests/test/search.rs helix-term/tests/test/session.rs book/src/keymap.md book/src/commands.md book/src/configuration.md
git commit -m "feat(search): add replace keys, commands, and session persistence"
```

---

## Final Verification

- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo check -p helix-term`.
- [ ] Run `cargo test -p helix-view -q`.
- [ ] Run `cargo test -p helix-term --lib -q`.
- [ ] Run `cargo test -p helix-term --features integration --test integration -q`.
- [ ] Manually verify, with a light and a dark theme:
  - `<space>b` opens the tree; `j/k/h/l/Enter` navigate/open; `a/A/r/d` work; `<space>b` hides.
  - Reopen a workspace and confirm buffers/cursors/scroll restore.
  - Edit a file in another process: clean buffer reloads; dirty buffer shows the Keep/Reload/Overwrite menu.
  - `<space>/` searches; `Enter` opens at the match; `Tab` to replace; `Ctrl-r` and `Ctrl-Alt-Enter` replace; `:w` is still required for open buffers.
- [ ] `git log --oneline` shows one commit per task; no pushes.
