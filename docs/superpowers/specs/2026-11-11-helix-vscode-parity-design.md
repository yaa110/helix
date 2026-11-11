# Helix VSCode-Parity Features — Design

- **Date:** 2026-11-11
- **Status:** Approved
- **Scope:** Local Helix fork (based on `25.07-991-g57d864129`). Changes are committed locally; nothing is pushed.
- **Implementation order:** session persistence → auto-reload → sidebar/file tree → search & replace.

## 1. Context

This fork keeps Helix as the editor but needs four capabilities that VSCode users rely on:

1. A persistent, keyboard-driven file tree on the left, toggled with `<space>b`.
2. Session restore: reopening a directory reopens previous buffers at their saved cursor and scroll positions.
3. VSCode-style workspace search and replace.
4. Automatic reload when an open file changes on disk.

No plugin system is introduced. All four features are implemented natively in the existing Rust crates, following the current architecture (`helix-view` for editor state, `helix-term` for UI, commands, async services, and configuration).

### Decision principles

- **Behavioral ambiguity:** follow VSCode.
- **Interaction ambiguity:** translate VSCode behavior into modal, keyboard-only Vim idioms (single-key commands where possible, no mandatory mouse).
- **No hidden data loss:** no automatic action may discard unsaved buffer contents.
- **Keep phases independent:** each phase ships and commits on its own, with `cargo check` and `cargo test` green.

### Non-goals (v1)

- Hot-exit/backup of unsaved buffers and undo history.
- OS trash integration for deleted files.
- Tree mouse interaction, drag and drop, and git decorations.
- LSP-driven rename propagation when a file is renamed in the tree.
- Multi-root workspaces.
- Restoring split layouts and jumplists.
- Search include/exclude glob fields and inline replacement preview. v1 searches everything allowed by `[editor.file-explorer]` rules.
- A plugin system or scripting API.

## 2. Architecture overview

```
Application (helix-term)
 ├── Editor (helix-view)
 │    ├── workspace_root: PathBuf
 │    ├── sidebar: SidebarState        // mode, width, focus, expanded dirs, search inputs
 │    └── reload_conflicts: map<DocumentId, ReloadConflict>
 ├── FileWatcher service (helix-term)  // one OS watcher, debounced, ref-counted dirs
 ├── SessionStore (helix-view)         // owned by Editor; load on startup, save on exit/focus/doc events
 └── Compositor
      └── EditorView (helix-term)
           ├── Sidebar component        // Files | Search | Hidden
           │    ├── FileTree            // lazy flattened tree
           │    └── SearchPane          // query/replace inputs + grouped results
           └── existing view rendering // now receives the area right of the sidebar
```

Two shared pieces are built once and reused:

1. **Sidebar pane.** A persistent left region owned by `EditorView`, with state in `Editor` so commands and session persistence can reach it. It has three modes (`Hidden`, `Files`, `Search`) and two focus owners (`Editor`, `Sidebar`).
2. **File watcher service.** A single `notify` watcher in `Application`, with a debounced channel branch in the existing `tokio::select!` event loop. It serves auto-reload first and tree refresh second.

`commands::Context` exposes `editor` but not the compositor directly, so all sidebar state that commands must mutate lives in `Editor`. UI-only data (visible rows, result buffers, text-input cursor) lives in `EditorView`.

## 3. Phase 1 — Session persistence

### Goal

Launching Helix in a workspace restores the previously open buffers, selections, and scroll positions. Exiting saves them.

### Algorithm (industry standard)

Path-keyed workspace snapshot with versioned JSON and atomic replacement. VSCode stores per-workspace state in a global SQLite database; Helix stores one JSON document per workspace under its data directory. Positions are stored as **line + character column**, not byte offsets, so they survive edits between runs and can be clamped when a file shrinks.

### Storage

- Directory: `helix_loader::data_dir()/sessions/`.
- File: `<sha256(canonical_workspace_path)>.json`, mirroring the hashing scheme already used by `helix-loader/src/workspace_trust.rs`.
- Writes are atomic: write to a temporary file in the same directory, flush, then rename. A failed write logs a warning and leaves the previous session intact.
- A corrupt or unknown-version file is logged, renamed to `.bak`, and ignored.

### Schema (version 1)

```json
{
  "version": 1,
  "workspace": "/abs/project",
  "focused": "src/main.rs",
  "buffers": [
    {
      "path": "src/main.rs",
      "selections": [
        { "anchor": [10, 4], "head": [12, 9] }
      ],
      "primary": 0,
      "scroll": {
        "anchor_line": 8,
        "vertical_offset": 0,
        "horizontal_offset": 0
      }
    }
  ],
  "sidebar": {
    "mode": "files",
    "width": 30,
    "expanded": ["src"],
    "selected": "src/main.rs"
  },
  "search": {
    "query": "foo",
    "replacement": "bar",
    "match_case": false,
    "whole_word": false,
    "regex": false,
    "preserve_case": false
  }
}
```

`sidebar` and `search` use `#[serde(default)]`. This lets phases 3 and 4 extend the file without breaking sessions written by phase 1. `version` remains `1` for compatible additive changes; a breaking change increments it and older versions are ignored.

Buffer and selection paths are stored relative to the workspace root when they are inside it, and absolute otherwise. This keeps the file readable and lets a project move without losing its session.

### Workspace identity

_Workspace root_ is determined once during `Application` startup:

- First CLI argument is a directory → that canonical directory.
- `--working-dir` is set → that canonical directory.
- Otherwise → `helix_loader::find_workspace().0` (VCS root when present, current directory otherwise).

The same value is stored in `Editor::workspace_root` and used by the tree and search pane.

### Restore behavior

Session restore is active only when:

- no file arguments were given, or
- the first argument is a directory (the existing "open directory picker" path).

If explicit file arguments are given, they win: no restore occurs and the saved workspace session is not overwritten.

Startup order in `Application::new`:

1. Create `Editor` and set `workspace_root`.
2. Restore the session before the first render.
3. For each saved buffer path that still exists, call `editor.open(path, Action::Load)`. Missing or irregular files are skipped.
4. Restore selections and view offsets for every opened document.
5. Switch to the saved focused document, or the first restored document if it is gone.
6. For `hx <dir>`, suppress the existing directory picker when at least one buffer was restored. Keep the picker when the session is empty.
7. Report a status message: `restored N buffers` or a warning listing skipped files.

Session saving is suppressed while restore is in progress (opening a buffer dispatches `DocumentDidOpen`, which must not overwrite the file being read). Saving is re-enabled after step 7.

Not restored: undo history, jumplist, marks, unsaved edits. Those belong to a separate hot-exit subsystem.

### Selection and scroll encoding

- Each `helix_core::Range` stores `anchor` and `head` as `(line, char_column)`.
- Restore: `start = doc.text().line_to_char(line)`, then `start + min(char_column, line_len)`. Convert back to `Range` and call `doc.set_selection(view_id, selection)`.
- `Selection::primary_index` is persisted and restored.
- Scroll uses `ViewPosition`: `anchor` is stored as `anchor_line`, `horizontal_offset`, and `vertical_offset`. Restore with `doc.set_view_offset(view_id, ...)` after setting the selection.
- Only the focused view's per-document state is persisted. Split layouts are not restored in v1.

### Save triggers

Session saving is cheap and synchronous (small JSON), with a short debounce:

- Application exit, including normal quit and `SIGINT`/`SIGTERM` handling.
- `DocumentDidOpen` and `DocumentDidClose`.
- View switch (`DocumentFocusLost`).
- After a successful write.
- On idle timeout, at most once every five seconds while the editor is dirty.

Saving never runs when explicit file arguments were used, to avoid clobbering the workspace session.

### Commands and configuration

- `:session-save` — write the current snapshot immediately.
- `:session-clear` — remove the saved snapshot for the current workspace.
- `editor.restore-session = true` — default; set `false` to disable both restore and automatic saving.

### Files

- New: `helix-view/src/session.rs` (schema, store, hashing, atomic read/write, collect/restore). It lives in `helix-view` because typed commands reach it through `Editor`.
- `helix-view/src/editor.rs`: `workspace_root`, `session_store`, `session_active`, `session_dirty`, session-related helpers.
- `helix-view/Cargo.toml`: `sha2` (`tempfile` is already a dependency).
- `helix-term/src/application.rs`: workspace-root capture, startup restore, exit/signal save, test-injectable session store.
- `helix-term/src/events.rs`: mark `session_dirty` on open/close/focus-loss.
- `book/src/configuration.md`, `book/src/commands.md`: document the config and commands.

### Tests

- JSON round-trip of the schema.
- Unicode line/column ↔ char conversion, including combining characters.
- Clamping when a file became shorter or lines were removed.
- Missing files are skipped without aborting the restore.
- Explicit file arguments disable restore and save.
- Atomic-write failure leaves the previous file intact.
- Corrupt/unknown-version file is ignored and backed up.

### Risks

- Restoring many buffers launches language servers and parsing for each. This matches VSCode and is acceptable; skipped/failed opens are isolated per file.
- Restoring scroll position can be adjusted by `ensure_cursor_in_view` if the saved cursor is off-screen. The saved cursor and scroll are restored together, so this is rare.

## 4. Phase 2 — Auto-reload on external change

### Goal

An open file changed by another process is reloaded automatically when safe, and never silently discards unsaved edits.

### Algorithm (industry standard)

OS file watching with debounced event coalescing. On macOS/iOS this is FSEvents, on Linux inotify, on BSD kqueue, on Windows `ReadDirectoryChangesW`; the `notify` crate abstracts them. Watch **parent directories non-recursively** instead of individual files, because atomic saves (temp file + rename), which Helix itself performs, replace the watched inode and would otherwise be missed.

### Watcher service

- One `notify` watcher and one debouncer (`notify-debouncer-mini`, 300 ms default) owned by `Application`.
- `FileWatcher` exposes `watch_dir(path)` / `unwatch_dir(path)` with a ref count, and a receiver of debounced paths.
- Subscriptions:
  - Phase 2: parent directory of every open document.
  - Phase 3: every visible tree directory, used to refresh the listing.
- Add on `DocumentDidOpen`, remove on close, refresh on save-as/path change. Multiple documents in one directory share one subscription.
- Watcher start failure or watch-limit errors are logged; the editor continues without auto-reload. `:reload` and `:reload-all` remain available.

### Event handling state machine

For an event path that matches an open document:

1. **Document no longer exists on disk** → keep the buffer, set `deleted on disk` status, do not modify text.
2. **Content equals the buffer** → update `last_saved_time`, do not reload. This suppresses self-triggered events and preserves undo history.
3. **Buffer is unmodified** → call `Document::reload()` (diff-based, selection-preserving) and set status `reloaded from disk`.
4. **Buffer is modified** → record `ReloadConflict { doc_id, disk_modified, notified: false }` and show a conflict prompt.

The content/mtime check in step 2 is the primary loop breaker; the debounce is a performance measure, not the correctness mechanism.

### Conflict prompt

VSCode's conflict dialog is adapted into a small menu with a **safe default**:

- **Keep** (default) — dismiss; the buffer remains modified. A new external change re-raises the prompt.
- **Reload** — discard local buffer changes and load disk contents.
- **Overwrite** — write the buffer to disk, replacing external changes.

Navigation uses `j`/`k` or arrow keys; `Enter` confirms the highlighted item; `Esc` chooses Keep. There are no single-key destructive actions, so an accidentally captured keystroke cannot discard work. `:write` on a document with an unresolved conflict opens the same menu instead of silently overwriting.

To avoid stealing a keystroke while the user is actively typing, the menu is not shown mid-edit:

- If the conflicted document is not focused, the conflict is queued and the menu opens when that document gains focus.
- If the conflicted document is already focused, a statusline hint is set immediately and the menu opens on the next idle timeout (after `editor.idle-timeout` with no input).

A new `DocumentFocusGained` event (dispatched from `Editor::switch`) or an equivalent pending-conflict check at `EditorView` key entry provides the focus trigger.

`Compare` is intentionally omitted in v1; it requires a dedicated diff view. `:reload`/`:reload-all` and `:write` cover the same outcomes.

### Configuration

- `editor.auto-reload = true`
- `editor.auto-reload-debounce = 300` (milliseconds)

### Commands

Existing `:reload` and `:reload-all` are reused for manual recovery. No new global keybinding is required; reload is automatic. The conflict menu owns its own keys.

### Files

- New: `helix-term/src/file_watcher.rs`.
- `helix-term/src/application.rs`: create watcher, add `select!` branch, route events; re-watch on `DocumentSaved`.
- `helix-view/src/editor.rs`: `reload_conflicts`, conflict resolution helpers.
- `helix-view/src/events.rs`: `DocumentFocusGained` (if used).
- `helix-term/src/ui/`: conflict prompt component.
- `helix-term/Cargo.toml`: `notify`, `notify-debouncer-mini`.
- Docs: configuration and commands.

### Tests

- Clean buffer + external write → reloaded; selection and view offset preserved.
- Dirty buffer + external write → conflict; each menu choice (Keep, Reload, Overwrite) produces the documented state.
- Own save (`:w`) does not trigger a reload or conflict.
- Atomic rename detection (write temp, rename over target).
- Delete → buffer retained with status; recreate → event handled.
- Watcher error path does not crash or block the event loop.
- Conflict queued for a background buffer and shown on focus.

### Risks

- Network filesystems and very large watch counts can fail; failure is non-fatal.
- Self-trigger loops are prevented by content comparison, not just debounce.
- Modifying a file while a save is in flight is resolved by the same conflict path (save completion updates the saved revision).

## 5. Phase 3 — Sidebar and file tree

### Goal

A persistent, VSCode-like left sidebar with a lazy file tree, fully keyboard-controllable, toggled by `<space>b`.

### Algorithm (industry standard)

A **lazy-loading flattened tree** with an explicit expansion set and **virtualized rendering**. Only visible rows are rendered. Each directory is read one level at a time; expanding a node inserts its children; collapsing removes descendants. This is the same model used by VSCode's tree widget and by NERDTree/oil, and it avoids walking the entire workspace at startup.

Directory listing uses the `ignore` crate with `max_depth(1)`, reusing the existing `directory_content` behavior and `FileExplorerConfig` rules (`.gitignore`, `.ignore`, `.helix/ignore`, hidden files, symlinks, flattening).

### Layout and focus

- `SidebarState` in `Editor`:
  - `mode: Hidden | Files | Search`
  - `focus: Editor | Sidebar`
  - `width: u16`
  - `expanded: HashSet<PathBuf>`
  - `selected: Option<PathBuf>`
- `EditorView::render`:
  - Sidebar occupies `area.clip_bottom(1)` (the bottom row stays the commandline/statusline).
  - Editor area is `area.clip_bottom(1).clip_left(sidebar_width)` when visible.
  - `cx.editor.resize(editor_area)` receives only the editor area.
  - The bufferline is rendered within the editor area.
  - A one-column divider is drawn at the sidebar's right edge.
- Width is `editor.sidebar.width`, clamped to `[min-width, max-width]` and to leave at least 40 columns for the editor. If the terminal is too narrow, the sidebar is auto-hidden. Auto-hide is rendering-only: `mode` and `width` are retained, and if the sidebar had focus, focus returns to the editor. When the terminal widens again, the sidebar reappears with the same mode and width.
- Focus routing: when `sidebar.focus == Sidebar`, `EditorView::handle_event` routes keys to the sidebar component before the normal keymaps. Otherwise, normal Helix keymaps run.
- The sidebar header shows the workspace directory name, or `SEARCH` in search mode.

### Tree data model

```
FileTree {
    root: PathBuf,
    rows: Vec<Row>,              // flattened visible rows
    cursor: usize,               // index into rows
    scroll: usize,               // first visible row
}

Row {
    path: PathBuf,
    depth: usize,
    kind: File | Dir,
    expanded: bool,
}
```

Expansion and selection are persisted in `SidebarState` (`expanded`, `selected`) rather than in `FileTree`, so commands and session code share one source of truth. `FileTree` rebuilds `rows` from `expanded` and updates `selected`/`cursor` together when the selection moves.

- Sort: directories first, then case-insensitive path name, matching VSCode's default explorer ordering.
- Expand: list one level, insert children after the parent at `depth + 1`, recursively reinsert already-expanded descendants.
- Collapse: remove all following rows with greater depth.
- Refresh (`R`): re-list expanded directories, preserve cursor by path; if the path disappeared, move to the nearest surviving row.
- Empty and unreadable directories render an inline placeholder row rather than an error-only status.
- Symlinks are followed only per `file_explorer.follow-symlinks`; a canonicalized set prevents cycles.
- Rendering uses `▸` / `▾` markers (no icon font), indentation guides, `ui.text.directory` for directories, and `ui.selection` for the selected row. When the editor has focus, the selected row uses a muted background.

### Keymap (sidebar Files focused)

| Key | Action |
|---|---|
| `j` / `k`, `↓` / `↑` | move selection |
| `h` / `←` | collapse; if already collapsed, jump to parent |
| `l` / `→` | expand; if a file, open it |
| `Enter` / `o` | open file and focus editor |
| `s` / `v` | open in horizontal / vertical split |
| `a` / `A` | create file / directory |
| `r` | rename |
| `d` | delete (confirmation popup) |
| `y` / `Y` | copy absolute / relative path |
| `g` / `G` | first / last row |
| `Ctrl-d` / `Ctrl-u` | half-page down / up |
| `H` | collapse all |
| `R` | refresh |
| `.` | reveal current buffer |
| `q` / `Esc` | focus editor (sidebar stays visible) |
| `[` / `]` | shrink / grow sidebar width |

Global bindings:

- `<space>b` — if hidden: show Files and focus it. If visible: focus the Files pane (switching modes if the sidebar is showing Search). If Files is already focused: hide the sidebar and focus the editor.
- `<space>B` — buffer picker (moved from `<space>b`).
- `<space>/` — if hidden: show Search and focus the query input. If visible: switch to Search and focus the query input (phase 4).
- `<space>e` — existing file-explorer picker is unchanged.

When a text input is active (search fields, create/rename prompts), printable keys edit the input; `<space>b` is not available until the input is dismissed with `Esc`.

### File operations

- **Create file / directory:** prompt for a name, validate that the parent is a directory and the name is a single non-empty path component, then create. A new file opens immediately.
- **Rename:** prompt with the current name. Rename on disk, update the path of any open buffer, update watcher subscriptions, update `expanded`/`selected`, and refresh the parent. Refuse to overwrite an existing path.
- **Delete:** confirmation popup (`y`/`n`). Refuse if an open buffer has unsaved changes. Delete permanently in v1; OS trash is a documented follow-up.
- **Copy path:** writes to the system clipboard using the existing clipboard provider.
- Errors (permission denied, target exists, invalid name) are shown in the statusline without closing the sidebar.
- Renaming or deleting a file that is open in a modified buffer is refused with a clear message.

### Tree refresh on external changes

The shared watcher also subscribes to visible/expanded directories (non-recursive). Debounced events refresh the affected directory listing while preserving expansion and selection. This is the VSCode behavior; `R` remains a manual fallback. If the watcher is unavailable, only `R` refreshes.

### Session integration

Persist `mode` (if not `Hidden`), `width`, `expanded`, and `selected` in the session file. Focus is always given to the editor on startup, not restored. After buffers are restored, apply the sidebar state. If the saved sidebar mode was `Hidden`, keep it hidden. If the saved selected path no longer exists, fall back to the workspace root.

### Files

- New: `helix-term/src/ui/sidebar.rs` (container, focus, divider, header).
- New: `helix-term/src/ui/sidebar/file_tree.rs` (data model, rendering, keys, file operations).
- `helix-term/src/ui/editor.rs`: split layout, key routing, editor-area resize.
- `helix-view/src/editor.rs`: `SidebarState`, `SidebarConfig`, workspace root usage.
- `helix-term/src/commands.rs`: toggle/focus commands, `<space>B` buffer picker.
- `helix-term/src/keymap/default.rs`: bindings.
- `helix-view/src/session.rs`: sidebar fields.
- `runtime/themes` and `theme.toml`: documented theme keys with fallbacks.
- Docs: keymap, configuration.

### Theme keys

New keys, each falling back to an existing style so unmodified themes render correctly:

- `ui.sidebar` → `ui.background`
- `ui.sidebar.border` → `ui.linenr`
- `ui.sidebar.title` → `ui.text`
- `ui.tree.directory` → `ui.text.directory` (already used by the file explorer)
- `ui.tree.selected` → `ui.selection`

### Tests

- Flattening: expand/collapse, nested expansion, collapse-all.
- Sorting: directories before files, case-insensitive ordering.
- Ignore rules and hidden files.
- Refresh preserves cursor by path; missing path falls back correctly.
- Create/rename/delete with open buffers and modified buffers.
- Narrow-terminal clamp and auto-hide.
- Key routing: sidebar keys do not leak into editor normal mode and vice versa.
- Session round-trip of sidebar state.

### Risks

- This is the most invasive phase because it changes `EditorView` layout and input routing. It is isolated behind `SidebarState` and landed after the non-UI phases.
- File operations can race with external changes; operations are validated immediately before acting and errors are reported rather than forced.

## 6. Phase 4 — Global search and replace

### Goal

A VSCode-style search pane in the left sidebar: query, replace, toggles, file-grouped streaming results, open-at-match, replace one, and replace all across the workspace.

### Algorithm (industry standard)

Ripgrep's engine, already available through `ignore` + `grep-searcher` + `grep-regex`:

- Parallel directory walk with `.gitignore`, `.ignore`, `.helix/ignore`, hidden and symlink rules from `FileExplorerConfig`.
- Line-oriented searching with binary detection.
- **Debounced query** (150–250 ms) and **generation-based cancellation**: a new query drops the old receiver; the old walker's injector push fails and the walk stops.
- **Streaming results** into a bounded channel, drained by the UI with a per-frame budget. Results are grouped in a `BTreeMap<PathBuf, Vec<Match>>` and rendered sorted.
- **Result cap:** 10,000 matches; the pane reports "showing first N".
- Open buffers are searched from memory; unopened files are read from disk.
- Search work runs off the UI thread using the same parallel-walker/injector pattern as the existing `global_search` command.

### UI layout (Search pane)

```
SEARCH
[ query input                    ]  Aa ab .*
[ replace input                  ]  AB
──────────────────────────────────
▾ src/main.rs (3)
   12  fn main() { let foo = ...
   40  foo += 1;
▾ src/lib.rs (1)
   7   pub use foo::...
```

- Header `SEARCH`.
- Query input with three toggle indicators (match case, whole word, regex), highlighted when active.
- Replace input below it, with a preserve-case toggle.
- Results tree: file rows with path and match count; match rows with line number and a trimmed preview, match range highlighted. Multi-line matches show the first line with a trailing ellipsis.
- A reusable `TextInput` widget provides grapheme-aware cursor movement and editing for the search fields: printable characters insert, `Backspace`/`Delete`, `Left`/`Right`, `Home`/`End`, `Ctrl-w` and `Ctrl-Backspace` (delete word), `Ctrl-u` (clear). The focused input is effectively a mini insert mode. Tree create/rename prompts continue to use the existing one-shot `ui::Prompt` in v1.

### Matching and replacement

- Defaults (VSCode): literal query, case-insensitive, whole word off, regex off.
- Toggles:
  - Match case: `case_insensitive(!match_case)`.
  - Whole word: `word(true)`.
  - Regex: use the query as-is; otherwise literal-escape it (`regex::escape`).
  - Invalid regex: show an inline error; keep the previous results visible but marked stale.
- Replacement templates: `$1`, `${name}` expansion only in regex mode; literal replacement otherwise. Preserve-case adapts the replacement to the matched text (upper/lower/title) when enabled.
- The pane's query, replacement, and toggles persist per workspace in the session file.

### Opening results

- `Enter` on a match: ensure the document is open, switch to it, set the selection to the entire match range (byte offsets converted to char offsets), center the view, and focus the editor.
- `s` / `v`: open in horizontal / vertical split.
- `n` / `N` and `F4` / `Shift+F4`: next / previous match across the grouped results, opening each file as needed.
- `Alt-Enter`: select all matches in the current file as multiple selections.
- `Esc` from results: focus editor; the sidebar remains visible. `q` behaves the same.
- `Alt-c` / `Alt-w` / `Alt-r`: toggle case / whole word / regex. `Alt-p`: preserve case.
- `Ctrl-r` on a match: replace that single match. `Ctrl-r` on a file row: replace all matches in that file.
- `Enter` in the Replace input: replace the selected match, or the next match if none is selected.
- `Ctrl-Alt-Enter`: replace all in the workspace.
- `R`: re-run the current query.
- `Tab`: cycle Query → Replace → Results.

Terminals that do not deliver `Ctrl-Alt-Enter` can use the equivalent typed commands `:search-replace-all` and `:search-replace-current`. File groups are expanded by default as results stream in; they can be collapsed with `h`.

Results are a snapshot of the text at query time. External file changes do not live-update the list; replace operations revalidate each file before acting, and `R` re-runs the query.

### Replace semantics

- **Open buffers:** collect all matches for the file, build one `Transaction` with changes sorted by ascending range (the ordering required by `ChangeSet::from_changes`), apply it, append to history. The document is marked modified; the user still decides when to write. Undo works per buffer.
- **Unopened files:** re-read and re-search the file immediately before writing, so stale offsets never clobber changed content. Write atomically (temp file in the same directory, flush, rename) and preserve permissions. If the file is binary, non-UTF8, deleted, or no longer contains the matches, skip it and report it.
- **Line endings:** replacement text containing `\n` is normalized to the file's detected line ending (CRLF files stay CRLF).
- **Stale/edited open buffers:** re-run the search against the current buffer text at replace time; if the buffer changed since the result list was built, replace the current matches and report the difference.
- **Summary:** `Replaced X occurrences in Y files (Z skipped)`.
- No confirmation dialog for replace-all, matching VSCode.

### Performance practices

- Debounce query changes; never search on every keystroke.
- Drop stale searches immediately; do not let old threads keep injecting results.
- Drain results with a per-frame budget; render only visible rows.
- Cap results and stop the walker when the cap is reached.
- Reuse open buffer text instead of disk reads.
- Use `ignore` rules to avoid scanning build directories and vendored code.

### Files

- New: `helix-term/src/ui/sidebar/search.rs` (pane, inputs, toggles, result grouping, keys).
- New: `helix-term/src/ui/text_input.rs` (reusable single-line input for the search fields).
- `helix-term/src/commands.rs`: replace current match, replace all, next/previous, open-at-match helpers; typed commands `:search-replace-all` and `:search-replace-current`; repurpose `<space>/` from the picker to the sidebar Search pane. The old picker remains available as `:global-search`.
- `helix-term/src/keymap/default.rs`: bindings; `<space>B` buffer picker.
- `helix-view/src/session.rs`: search fields.
- `helix-term/Cargo.toml`: `regex` if not already a direct dependency.
- Docs: keymap, commands, configuration.

### Tests

- Toggle matrix: literal/regex, case sensitive/insensitive, whole word on/off.
- Regex capture expansion (`$1`, `${name}`) and literal `$` behavior.
- Preserve-case behavior for upper/lower/title matches.
- Unicode byte ↔ char mapping for opening and selecting matches.
- Multi-file replace in a temp workspace; open-buffer undo; unopened files written atomically.
- Stale-file revalidation: modify a file after results are shown, then replace all; no clobbering, correct summary.
- CRLF preservation and non-UTF8 skip.
- Query change cancels the previous search and never mixes results.
- Result cap and per-frame drain budget.

### Risks

- Replace-all on unopened files is not undoable after the fact; this matches VSCode. Atomic writes and per-file revalidation prevent corruption.
- Regex replacement must use the same syntax as the searcher; both use the `regex`/`regex-syntax` family, so semantics align.
- Large workspaces can still produce many events; the result cap and cancellation keep the UI responsive.

## 7. Configuration summary

All under `[editor]` unless noted. Names follow the existing `kebab-case` serde convention.

```toml
[editor]
restore-session = true
auto-reload = true
auto-reload-debounce = 300

[editor.sidebar]
width = 30
min-width = 20
max-width = 50
```

Existing `[editor.file-explorer]` settings (ignore rules, hidden files, flattening, symlinks) govern both the tree and search.

## 8. Keymap summary

| Binding | Action |
|---|---|
| `<space>b` | Toggle sidebar / focus Files pane |
| `<space>B` | Buffer picker (moved) |
| `<space>/` | Open/focus Search pane |
| `<space>e` | Existing file-explorer picker (unchanged) |

Sidebar-local keys are listed in the phase sections. Sidebar keys never leak into the editor and editor normal-mode keys never leak into the sidebar.

## 9. Testing strategy

- **Unit tests:** session serialization, line/column conversion, watcher event classification, tree flattening/sorting, search matcher toggles and replacement expansion.
- **Integration tests:** use the existing integration harness with temp workspaces:
  - Save session, relaunch, assert buffers/selections/scroll.
  - External file write with clean/dirty buffers and conflict choices.
  - Tree create/rename/delete with open buffers.
  - Multi-file search/replace, stale-file revalidation, undo in open buffers.
- **Manual verification:** each phase is checked against the VSCode behavior it mirrors and against at least one light and one dark Helix theme.
- **Regression gates:** `cargo check`, `cargo test` for touched crates, `cargo clippy` where practical, and `cargo fmt`.

## 10. Rollout

1. Phase 1: session persistence. Commit locally.
2. Phase 2: auto-reload. Commit locally.
3. Phase 3: sidebar + file tree. Commit locally.
4. Phase 4: search & replace. Commit locally.

No pushes. Each phase preserves existing behavior when its feature is disabled by configuration. Later phases reuse earlier infrastructure but do not alter earlier semantics.

## 11. References

- VSCode: workspace state storage, explorer tree, search view, file watcher and conflict handling, editor replace semantics.
- ripgrep: `ignore`, `grep-searcher`, `grep-regex` (the same engine VSCode uses).
- `notify` and `notify-debouncer-mini` for cross-platform file watching.
- NERDTree/oil.nvim and VSCode's tree widget for lazy tree interaction patterns.
