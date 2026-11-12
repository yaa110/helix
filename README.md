# Helix — personal VSCode-parity fork

> ## Unofficial, AI-assisted fork — read this first
>
> This repository is a **personal fork of
> [Helix](https://github.com/helix-editor/helix)**. It is not upstream Helix,
> it is not affiliated with or endorsed by the Helix maintainers, and it is not
> an official Helix release.
>
> **Substantial parts of the fork-specific code and documentation were written
> or modified with the help of AI LLM models.** The changes were made to support
> this fork author's own workflow and have not been reviewed by the upstream
> Helix maintainers.
>
> **Issues cannot be resolved and pull requests are not managed here.** This
> repository is published for personal use and reference. Do not report problems
> caused by this fork to the upstream Helix project, and do not expect support,
> reviews, merges, or timely responses here. Use it at your own risk.

The goal is to keep Helix's modal editing model while adding the workflow
pieces a VSCode user misses.

Everything is implemented natively in Rust. There is no plugin layer.

> Upstream Helix remains the authority for everything not described here.
> See the upstream book and `CHANGELOG.md` for the base editor.

## Fork additions at a glance

- **File tree sidebar** on the left, toggled with `<space>b`, with lazy
  directory loading, create/rename/delete, splits, path copying, and
  VSCode-like auto-reveal of the current buffer.
- **Automatic tree tracking**: changing the current buffer expands and selects
  it in the tree; opening a file through any picker also selects it.
- **Two-letter tree jumps** with `gw` + label.
- **Fast focus switching**: `<space>b` from the editor lands on the current
  buffer; `Enter`/`Tab`/`Esc` returns to the editor.
- **Global shortcuts from the sidebar**: unhandled `Ctrl`/`Alt` chords pass
  through, so bindings such as `Ctrl+E` for a picker still work while the tree
  is focused.
- **Session restore** of buffers, selections, cursor/scroll, sidebar state,
  and search inputs per workspace.
- **Automatic file reload** with silent reload for clean buffers and a safe
  Keep/Reload/Overwrite prompt for dirty buffers.
- **Workspace search and replace** in the sidebar, with case/word/regex/
  preserve-case toggles, grouped streaming results, open-at-match,
  select-all, replace-one, and replace-all.
- **LSP code lenses**: `<space>l` shows actions for the current line (for
  example gopls `run test`), `<space>L` shows all lenses in the file, and the
  first available lens title appears in the statusline.
- **Searchable text transforms** (`F1` or `<space>t`): base64/hex encode and
  decode, case conversions, and line sorting on selections or the whole buffer.
- **Always-visible buffer tabs** (`bufferline = "always"`).
- **Monokai Community (Filter Spectrum)** as the default theme.
- **Native implementation** in `helix-view` and `helix-term`; no plugin system.

## Install

This fork is published at
[github.com/yaa110/helix](https://github.com/yaa110/helix).

### Homebrew

```sh
brew tap yaa110/helix https://github.com/yaa110/helix
brew install yaa110/helix/helix
```

The formula builds from the `master` branch and installs `hx` plus this fork's
runtime (themes, queries, grammars) under Homebrew's prefix.

```sh
# After new commits on master:
brew reinstall yaa110/helix/helix
```

### curl installer

```sh
curl -fsSL https://raw.githubusercontent.com/yaa110/helix/master/install.sh | sh
```

This installs to `~/.local` by default:

- `~/.local/bin/hx` — wrapper that sets `HELIX_RUNTIME`
- `~/.local/libexec/helix/hx` — the actual binary
- `~/.local/share/helix/runtime` — themes, queries, grammars

Make sure `~/.local/bin` is on your `PATH`:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

Override the prefix or version:

```sh
HELIX_INSTALL_DIR="$HOME/.helix" \
HELIX_VERSION=v0.1.0 \
  curl -fsSL https://raw.githubusercontent.com/yaa110/helix/master/install.sh | sh
```

Prebuilt releases are provided for:

- `x86_64-unknown-linux-gnu`
- `aarch64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`

The release workflow only runs when a `v*` tag is pushed.

## Build and run

Local builds use your `rustup` default toolchain. CI builds with Rust 1.90.0.

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

## Default theme

The default theme is **Monokai Community (Filter Spectrum)**, matching the VSCode
theme of the same name (`monokai_pro_filter_spectrum.toml`). It is used
unconditionally when `theme` is not set in `config.toml`. This fork targets
modern true-color terminals (Ghostty, kitty, WezTerm, iTerm2); the fallback is
only used if the theme file itself cannot be loaded.

```toml
# Optional: pin it explicitly, or choose another theme.
theme = "monokai_pro_filter_spectrum"
```

`:theme monokai_pro_filter_spectrum` switches to it at runtime.

## Changing the fork defaults

Everything the fork adds is configurable. A complete example that changes every
fork-specific default:

```toml
# Top-level. Omit to use Monokai Pro (Filter Spectrum); set another name to
# replace the default theme.
theme = "monokai_pro_filter_spectrum"

[editor]
# Session restore and automatic saving. Default: true
restore-session = true

# Reload clean buffers when files change on disk. Default: true
auto-reload = true

# Filesystem debounce in milliseconds. Default: 300
auto-reload-debounce = 300

# Always show the buffer tabs/bufferline. Default: "always"
bufferline = "always"

# Ghostty and other modern terminals support this automatically; set it only
# if Helix does not detect it.
true-color = true

# Alphabet used by tree `gw` jump labels. Default: a-z
# jump-label-alphabet = "abcdefghijklmnopqrstuvwxyz"

[editor.sidebar]
# Preferred/limits for the left pane, in columns. Defaults: 30/20/50
width = 30
min-width = 20
max-width = 50

[editor.file-explorer]
# The tree and search use these ignore rules. Defaults shown.
hidden = false
follow-symlinks = false
parents = false
ignore = false
git-ignore = false
git-global = false
git-exclude = false
flatten-dirs = true

[keys.normal]
# Fork bindings. Defaults:
b = "toggle_file_tree"
B = "buffer_picker"
# C-e = "file_picker"   # example popular VSCode-style binding
```

Notes:

- `theme` is top-level; all other settings are under `[editor]` or a child
  table.
- `:config-reload` applies most changes without restarting.
- The tree/search ignore behavior comes from `[editor.file-explorer]`; changing
  those values affects both the tree and the search walker.
- `C-b = "toggle_file_tree"` is recommended if you want the VSCode muscle
  memory; `C-b` is otherwise `page_up`.
- Unhandled `Ctrl`/`Alt` chords pass through from the sidebar, so custom global
  bindings keep working while the tree or search pane is focused.

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

**Global shortcuts still work:** while the tree (or search pane) is focused,
plain keys belong to the sidebar, but unhandled `Ctrl`/`Alt` chords fall through
to the normal keymap. So a custom `Ctrl+e = "file_picker"` (or any other global
binding) works from the tree, and `<space>b` / a configured `Ctrl+b` still
toggles it.

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

## Text transforms (`F1` / `<space>t`)

A searchable VSCode-style action modal for selected text. If there is no
selection, the transform applies to the whole buffer.

Available actions:

- Base64 encode / decode
- Hex encode / decode
- `UPPERCASE`, `lowercase`, `Title Case`
- `kebab-case`, `snake_case`, `camelCase`, `PascalCase`,
  `SCREAMING_SNAKE_CASE`, `dot.case`
- Sort lines ascending / descending

Open with `F1` or `<space>t`; type to filter, `j`/`k` or arrows to move,
`Enter` to apply, `Esc` to cancel.

## LSP code lenses (run tests, runnable actions)

Code lenses are the LSP mechanism VSCode uses for gopls’s `run test` /
`run benchmark` above Go functions and rust-analyzer’s `▶ Run` lens. This fork
adds generic code-lens support — no Go-specific or debug-adapter code.

- `<space>l` — open a menu of code lenses on the cursor line
- `<space>L` — open a menu of all code lenses in the current file
- The first lens on the cursor line is shown in the statusline (`CodeLens`
  statusline element, enabled by default)
- Selecting a menu item executes the LSP command via
  `workspace/executeCommand`; server-initiated `workspace/applyEdit` is handled
  by the existing LSP integration

The menu is keyboard only: `j`/`k` or arrows to move, `Enter` to execute, `Esc`
to cancel. Lenses are refreshed on idle after edits. Code actions remain on
`<space>a`; they are a different protocol.

MVP limitation: lenses whose command is only supplied through
`codeLens/resolve` are not shown yet, and no inline virtual-text button is
rendered. This is deliberate: the menu + statusline is less intrusive for a
modal editor.

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

# Always show the buffer tabs/bufferline. Default: "always"
bufferline = "always"

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
