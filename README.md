# Helix — personal VSCode-parity fork

This repository is a **local fork of [Helix](https://github.com/helix-editor/helix)**.
It is not upstream Helix and is not intended to be pushed to the upstream
project. The goal is to keep Helix's modal editing model while adding the
workflow pieces a VSCode user misses:

- a persistent, keyboard-driven file tree;
- session restore for buffers and cursor/scroll positions;
- VSCode-style workspace search and replace;
- automatic reload when an open file changes on disk.

Everything is implemented natively in Rust. There is no plugin layer.

> Upstream Helix remains the authority for everything not described here.
> See the upstream book and `CHANGELOG.md` for the base editor.

## Build and run

The toolchain is pinned in `rust-toolchain.toml` (Rust 1.90.0).

```sh
# Run once from the repository root. HELIX_RUNTIME makes Helix use this fork's
# runtime/ (themes, queries, and the prebuilt grammars).
HELIX_RUNTIME="$PWD/runtime" cargo run --release -- .

# Or build once and run the binary:
cargo build --release
export HELIX_RUNTIME="$PWD/runtime"
./target/release/hx .
```

Open a file or a workspace:

```sh
HELIX_RUNTIME="$PWD/runtime" ./target/release/hx src/main.rs
HELIX_RUNTIME="$PWD/runtime" ./target/release/hx .
```

To install the binary:

```sh
cargo install --path helix-term --locked
export HELIX_RUNTIME="$PWD/runtime"   # add to ~/.zshrc / ~/.bashrc if desired
hx .
```

`runtime/grammars` is already populated in this checkout. If you move the
binary elsewhere, either set `HELIX_RUNTIME` or symlink `runtime/` into
`~/.config/helix/runtime`.

## What is new in this fork

### 1. Persistent file tree sidebar

A VSCode-like left sidebar with three modes: **Files**, **Search**, and hidden.
It reserves editor space rather than floating over the code, and it is fully
keyboard driven.

- `<space>b` — show/focus the Files pane, or hide it when Files is already focused.
- `<space>/` — show/focus the Search pane.
- `<space>B` — buffer picker (moved from `<space>b`).
- The sidebar auto-hides when the terminal cannot leave 40 columns for the editor.

The tree is lazily loaded (one directory level at a time), respects the same
ignore/hidden/symlink rules as `[editor.file-explorer]`, and refreshes when the
filesystem changes.

#### Tree keymap

| Key | Action |
| --- | --- |
| `j` / `k`, `Down` / `Up` | Move the selection |
| `h` / `Left` | Collapse the directory, or move to its parent |
| `l` / `Right` | Expand the directory, or open the file |
| `Enter` / `o` | Open the selected file and focus the editor |
| `Tab` | Open the selected file and jump to the editor; on a directory, just jump back |
| `s` / `v` | Open in a horizontal / vertical split |
| `gw` | Show two-letter jump labels; type two letters to jump to a row |
| `gg` / `G` | First / last row |
| `Ctrl-d` / `Ctrl-u` | Move ten rows down / up |
| `a` / `A` | Create a file / directory |
| `r` | Rename |
| `d` | Delete (confirmation defaults to No) |
| `y` / `Y` | Copy the absolute / relative path |
| `H` | Collapse all |
| `R` | Refresh |
| `.` | Reveal the current buffer in the tree |
| `[` / `]` | Shrink / grow the sidebar |
| `q` / `Esc` | Return focus to the editor |

**VSCode-like reveal:** while the editor has focus, the tree automatically
selects the current buffer, expanding its ancestor directories. This also
happens when a file is opened through a picker, `:open`, or any other command.
Manual tree browsing is not interrupted while the tree itself is focused.
`Tab` or `Enter` jumps back to the editor.

**Fast jumping:** `<space>b` from the editor lands on the current buffer in
the tree (because of auto-reveal); `Enter`, `Tab`, or `Esc` goes back to the
editor. From the tree, `Enter` opens the selected row, and `gw` + two letters
jumps to a row without opening it.

### 2. Session restore

When Helix starts in a workspace, it restores the previously open buffers,
selections, cursor positions, viewport, sidebar mode/width/expanded directories/
selected row, and search inputs.

- Restore is active when launched with no file arguments, or with a directory
  as the first argument.
- Explicit file arguments win and do not overwrite the saved session.
- State is stored under `helix_loader::data_dir()/sessions/<sha256>.json`.
- Writes are atomic; corrupt files are backed up as `.bak`.
- Unsaved-buffer hot exit and undo history are not restored.

Commands:

- `:session-save`
- `:session-clear` (also disables automatic saving for this run so quitting
  does not recreate the file)

### 3. Automatic file reload

A debounced OS file watcher (`notify`) watches the parent directories of open
documents, so atomic saves (temp file + rename) are detected.

- Clean buffers reload silently and keep their selection.
- Dirty buffers never lose text. The next idle/focus shows a menu:
  **Keep** (default), **Reload**, **Overwrite**.
- Saves performed by Helix are ignored via content comparison.
- Deleted files keep their buffer and show a status message.
- Existing `:reload` / `:reload-all` still work and clear conflict state.

### 4. Workspace search and replace

`<space>/` opens a VSCode-style search pane in the sidebar.

- Query, replacement, and toggles: `Aa` match case, `ab` whole word, `.*` regex,
  `AB` preserve case.
- Results are grouped by file, streamed in, debounced, cancellable, and capped
  at 10,000 matches.
- Open buffers are searched from memory; unopened files are read from disk.
- Results revalidate against current text before replacing.

Search pane keys:

| Key | Action |
| --- | --- |
| `Tab` | Query → Replace → Results |
| `Alt-c` / `Alt-w` / `Alt-r` / `Alt-p` | Toggle case / word / regex / preserve case |
| `j` / `k` | Move through results |
| `h` / `l` | Collapse / expand a file group |
| `Enter` | Open the selected match and select it |
| `s` / `v` | Open the selected match in a split |
| `n` / `N`, `F4` / `Shift-F4` | Next / previous match |
| `Alt-Enter` | Select all matches in the current file |
| `Ctrl-r` | Replace the selected match (on a file row: all in that file) |
| `Ctrl-Alt-Enter` | Replace all matches in the result set |
| `R` | Re-run the query |
| `Esc` | Results → editor |

Replace semantics:

- Open buffers get one in-memory, undoable transaction; they stay modified
  until you `:w`.
- Unopened files are re-read, re-searched, and written atomically, preserving
  permissions and CRLF line endings. Binary/non-UTF-8 files are skipped.
- `:search-replace-all` and `:search-replace-current` expose the same
  operations from command mode.

## Configuration

Configuration lives at `~/.config/helix/config.toml` (or the path passed with
`-c`). New settings:

```toml
[editor]
# Restore buffers/selections/scroll and save automatically.
restore-session = true

# Reload clean buffers when files change on disk.
auto-reload = true
auto-reload-debounce = 300 # milliseconds

[editor.sidebar]
width = 30
min-width = 20
max-width = 50
```

### Binding Ctrl+B instead of <space>b

`Ctrl-b` is normally `page_up`. Override it in your config:

```toml
[keys.normal]
C-b = "toggle_file_tree"
```

`PageUp` remains available for paging. If you use tmux, remap its prefix or
`Ctrl-b` will not reach Helix. The sidebar/search inputs own the keyboard while
focused, so `C-b` is a normal-mode binding; `Esc` returns to the editor first.

## Verification

The implementation is covered by unit and integration tests:

```sh
cargo test -p helix-view
cargo test -p helix-term --lib
cargo test -p helix-term --features integration --test integration
```

Manual verification still matters for theme rendering and OS-specific watcher
behavior.

## Known limitations / deferred work

These are deliberately out of scope for the current fork state:

- No hot-exit of unsaved buffers, undo history, or split-layout restore.
- `Compare` is not offered in the reload-conflict menu; use `:reload`/`:w!`.
- `:w` on an unresolved conflict errors rather than opening the menu.
- Search currently uses `ignore`'s default filters rather than the full
  `[editor.file-explorer]` configuration, and reads whole files sequentially.
- Bracketed paste while the sidebar is focused is not routed into the search
  inputs.
- BOM/non-UTF-8 file comparison in the watcher is best-effort.
- No OS trash integration; tree delete is permanent after confirmation.
- No mouse interaction, git decorations, or LSP rename propagation from tree
  renames.
- Windows session replacement is not fully atomic.
- Tree empty/unreadable placeholders and symlink-cycle deduplication are not
  implemented.

## Upstream and license

This is a fork of Helix. Upstream: <https://github.com/helix-editor/helix>.
Helix is licensed under MPL-2.0; see `LICENSE`. The new fork-specific code in
this repository is provided under the same license.
