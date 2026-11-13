# Helix Code

![Helix Improved screenshot](assets/helix2.png)

A personal, AI-assisted fork of [Helix](https://github.com/helix-editor/helix)
that keeps modal editing while adding the VSCode-style workflow pieces this fork
author uses every day. Everything is implemented natively in Rust; there is no
plugin layer.

> **Unofficial fork — read this first.** This is **not** upstream Helix and is
> not affiliated with or endorsed by the Helix maintainers. Substantial parts of
> the fork-specific code and documentation were written or modified with AI
> assistance and have not been reviewed upstream. **Issues cannot be resolved
> and pull requests are not managed here.** This repository is published for
> personal use; do not report fork problems to upstream Helix, and do not expect
> support, reviews, merges, or timely responses. Use it at your own risk.

## How to Install

Both installers build from the `main` branch with native-CPU optimizations
(`-C target-cpu=native`), fat LTO, one codegen unit, and stripped binaries.
There are no release tags or prebuilt artifacts.

### Homebrew

```sh
brew tap yaa110/helix https://github.com/yaa110/helix
brew install yaa110/helix/helix
```

After new commits on `main`:

```sh
brew update
brew reinstall yaa110/helix/helix
```

### curl installer

```sh
curl -fsSL https://raw.githubusercontent.com/yaa110/helix/main/install.sh | sh
```

The script downloads `main` and compiles `hx` locally. It requires **Rust 1.90
or newer**, plus `git` and a C/C++ compiler for tree-sitter grammars.

It installs to `~/.local` by default:

- `~/.local/bin/hx` — wrapper that sets `HELIX_RUNTIME`
- `~/.local/libexec/helix/hx` — compiled binary
- `~/.local/share/helix/runtime` — themes, queries, grammars

If `~/.local/bin` is not already on `PATH`:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

Useful options:

```sh
# Different prefix
HELIX_INSTALL_DIR="$HOME/.helix" \
  curl -fsSL https://raw.githubusercontent.com/yaa110/helix/main/install.sh | sh

# Skip the long grammar build; hx still works without syntax grammars
HELIX_SKIP_GRAMMARS=1 \
  curl -fsSL https://raw.githubusercontent.com/yaa110/helix/main/install.sh | sh
```

Verify the installed build:

```sh
hx --version
```

## New Features

### File tree and sidebar

`<space>b` shows and focuses the tree; press it again from the editor to hide
it. Press `/` to start fuzzy search; plain typing no longer filters the tree.
Search results stay visible when you move to the code area with
`Ctrl-w Right` and come back with `Ctrl-w Left`.

| Direct key | Action |
| --- | --- |
| `/` | Start fuzzy search / edit the existing query |
| printable characters | Edit the query while search is active |
| `Enter` | Finish editing, keep filtered rows |
| `Backspace` / `Ctrl-u` | Delete a query character / clear the query |
| `Cmd/Alt-Backspace`, `Cmd/Alt-Delete` | Clear the query and keep editing |
| `j` / `k`, `Down` / `Up` | Move the selection |
| `J` / `K` | Move ten rows |
| `g` / `G`, `gg` | First / last / first row |
| `gw` | Show two-letter jump labels |
| `Ctrl-Down` / `Ctrl-Up` | Smooth-scroll by `scroll-lines` (default 3) |
| `Right` / `Left` | Expand/open, or collapse/parent |
| `Enter` | Open the selected file and focus the editor |
| `Tab` | Open the selected file, or return to the editor on a directory |
| `:` | Open the main command line |
| `Ctrl-w Left` | Move to the tree from the leftmost editor buffer |
| `Ctrl-w Right` | Move from the tree back to the editor, keeping results |
| `Esc` | Cancel search; press again to return focus to the editor |

A dedicated `/` search box sits below the project name and filters files with
the same fuzzy matcher as `<space>f`. Press `Enter` to stop editing while
keeping the filtered rows; `gw`, `<space>` actions, and navigation keep working
on those rows. Press `/` again to resume editing the same query. The search
field uses the command-line I-beam cursor and editing keys: arrows,
`Home`/`End`, `Ctrl/Alt+Left`/`Right` for word movement, and word deletion.
The command-line hint shows whether `Esc` will cancel search or leave the tree.

Press `<space>` in the tree to open a categorized which-key-style hint box:

| Space command | Action |
| --- | --- |
| `j` / `k` | Move down / up |
| `h` / `l` | Collapse / expand |
| `Enter`, `o` | Open |
| `s` / `v` | Open in a horizontal / vertical split |
| `a` / `A` | Create a file / directory |
| `r` | Rename |
| `d` | Move selected/marked items to trash |
| `u` | Undo the last filesystem operation |
| `t` / `V` | Mark an item / clear all marks |
| `c` / `m` / `p` | Copy / cut / paste selected/marked items |
| `I` | Show / hide ignored files |
| `y` / `Y` | Copy the absolute / relative path |
| `x` | Close the current buffer |
| `X` | Close all other buffers and their splits |
| `W` | Open a recently used workspace |
| `/` | Open global search |
| `b` | Hide the sidebar |
| `:` | Open the main command line |
| `g` / `G` | First / last row |
| `J` / `K` | Move ten rows |
| `H` / `R` | Collapse all / refresh |
| `D` | Toggle dirty-files-only view |
| `.` | Reveal the current buffer |
| `[` / `]` | Shrink / grow the sidebar |
| `<` / `>` | Scroll long rows left / right |
| `q` / `Esc` | Return to the editor / close the command box |

Additional tree behavior:

- Indentation guides are enabled by default and use a muted gray, so deep
  trees stay readable without competing with names or selection.
- Git status colors: modified files use the `diff.delta` color, untracked files
  use `diff.plus`, conflicts use `diff.delta.conflict`, and directories inherit
  the color of changed children; gitignored files use a muted gray. `<space>D`
  shows dirty files only; status is refreshed in the background (every 2s in
  dirty-only mode, otherwise 10s).
- `.git`/VCS directories are never listed, expanded, or searched. Fuzzy tree
  search walks the full workspace (up to 50k entries) while skipping `.git`,
  gitignored files, and build/cache trees, so collapsed folders are searchable
  without hanging. The candidate list is cached per workspace/revision and
  refreshed when the file watcher reports changes.
- Multi-select is available with `t`; marked rows show `●`. Batch copy/move
  prompts before overwriting existing paths. Delete moves items to a temporary
  trash directory instead of permanently removing them, and `u` restores the
  last filesystem operation. `I` toggles ignored files in the tree.
- Git status refreshes only paths reported by the file watcher, with periodic
  full refreshes as a fallback.
- Long names are clipped at the pane boundary; the statusline shows the full
  selected path, and `<space><` / `<space>>` scroll long rows.
- Selection is shown with the row background only; tree names stay regular
  (not bold), and the sidebar border keeps its normal color when focused.
- Opening a file from any picker expands and selects it in the tree.
- `<space>x` closes the current buffer; `<space>X` force-closes every other
  buffer and its splits.
- `<space>W` reopens a recently used workspace from a global LRU list.
- Deleting an open file closes its buffer and reveals the next/previous buffer;
  deleting a closed file selects the adjacent file instead of jumping to the
  bottom of the tree.
- `<space>e` / `<space>E` open the file explorer at the workspace / current
  working directory.

### Global search

`<space>/` opens a floating search-and-replace window with a source preview.
It keeps the previous query, replacement, options, results, cursor, and
selection when reopened. `<space>/` from the tree opens it too, and `<space>b`
from search returns to the tree.

- `Find:` and `Replace:` inputs. The window opens in results mode; plain typing
  does nothing until `/` is pressed.
- `/` edits Find without clearing the existing text. `Enter` runs the search
  and returns to results. Pressing `/` again resumes editing.
- While editing, `Tab` cycles between Find and Replace.
- Terminal paste (`Cmd-V`, `Ctrl-Shift-V`, etc.) enters edit mode and inserts
  into the focused Find/Replace input instead of the document.
- `Cmd-Backspace`/`Cmd-Delete` (or `Alt-Backspace`/`Alt-Delete`) clears the
  focused Find/Replace field.
- Both fields use the command-line I-beam cursor and editing keys: arrows,
  `Home`/`End`, `Ctrl/Alt+Left`/`Right` for word movement, and word deletion.
- Options: `[Aa]` case, `[ab]` whole word, `[.*]` regex, `[AB]` preserve case.
- `Alt-c` / `Alt-w` / `Alt-r` / `Alt-p` toggle those options directly.
- `<space>` opens a categorized options/replace hint box; `c`/`w`/`r`/`p`
  toggle the same settings, `/` starts editing, `b` opens the tree.
- `Esc` leaves edit mode, then closes the hint box, then closes the search
  window without moving the cursor.
- `Enter` opens the selected result as-is while browsing results.
- `r` replaces the selected result item; `R` replaces all results.
- `Ctrl-r` and `Ctrl-Alt-Enter` remain available as aliases.
- The active field/result area is marked with `▸` and the focused field shows a
  cursor.
- Empty search shows hints; no matches shows `No results found`.

### Mouse scrolling

`mouse-native-scroll` is enabled by default while `mouse` remains disabled.
This enables terminal mouse reporting for the wheel only: scrolling works in
the code area, file tree, search results and previews, pickers, menus, folder
picker panes, and scrollable popups. Clicks, drags, releases, middle-click
paste, and clickable popup actions are ignored.

Each wheel event scrolls by `scroll-lines` (default 3). Scrolling moves the
viewport without moving the selection; if the selection would leave the
visible area, it is clamped to the nearest edge, matching the code editor's
mouse scrolling. Changes apply immediately, without restarting `hx`.

Because terminals have no scroll-only mouse protocol, enabling this option
captures mouse reporting. Terminal-native text selection is therefore blocked
while `hx` is running. Set `mouse-native-scroll = false` for fully mouse-free
behavior, or enable `mouse = true` for full click and scroll support.

```toml
[editor]
mouse = false
mouse-native-scroll = true
```

### Folder picker

`:open-folder` (alias `:folder`) opens a two-pane folder picker and switches
the selected folder to a workspace with the same session save/restore behavior
as launching `hx .` inside it. The selected folder is also added to `<space>W`.

The picker starts at the current working directory when it is inside `$HOME`,
otherwise at `$HOME`. Browsing with the arrow keys cannot go above `$HOME`;
typing an absolute path can navigate anywhere. Both panes list only
directories, including hidden and symlinked directories. Listings are loaded
one level at a time, so choosing a folder previews its immediate children
without walking the rest of the tree.

| Key | Action |
| --- | --- |
| `/` | Edit fuzzy search over the current folder's items (non-recursive) |
| `Enter` while searching | Finish editing and keep the filtered items |
| `Cmd/Alt-Backspace`, `Cmd/Alt-Delete` | Clear the search or path field and keep editing |
| `Up` / `Down` | Move the selection |
| `Right`, `Space`, `o` | Enter the selected folder |
| `Left`, `Delete`, `Backspace` | Go to the parent folder |
| `p` | Edit the absolute path shown at the bottom |
| `Enter` while editing the path | Navigate to that directory; `Esc` cancels |
| `Enter` while browsing | Open the selected folder as a workspace |
| `Esc` | Cancel search/path editing, otherwise close the picker |

The search field is at the top and the absolute path is always shown at the
bottom. Both fields use the command-line I-beam cursor and editing keys. When
searching, the left pane is filtered and the right pane previews the direct
subdirectories of the selected item; the preview is not selectable. Entering a
folder clears the search and selects its first child, while going to the parent
re-selects the folder that was left. Invalid or unreadable paths show an error
and leave the current folder unchanged.

### Peek and code lenses

- `gP` peeks the definition; `gR` peeks references.
- Both use a searchable floating preview picker with source preview.
- `Enter` jumps; `Esc` closes without moving the cursor.
- Reference peek excludes the declaration by default.

Code lenses use the same peek-style picker:

- `<space>l` — lenses on the cursor line.
- `<space>L` — all lenses in the file.
- `Enter` executes the selected LSP command; `Esc` cancels.
- The first lens on the cursor line also appears in the statusline.
- Lenses supplied through `codeLens/resolve` are resolved automatically.
- This fork enables `gopls.config.codelenses.test = true`, because recent gopls
  versions disable `run test` lenses by default.

### Editing and workflow

- **Session restore** of buffers, split layout, per-view selections and scroll,
  sidebar state, and search inputs per workspace. Sessions are stored in a
  compact binary format. Split layout and each view's selection/scroll are
  restored; hidden buffers are reopened after the visible views. Missing or
  unreadable entries are skipped so a broken restore never blocks startup. An
  unclean-exit marker enables crash recovery by default; set
  `[editor] crash-recovery = false` to start clean instead. An untouched empty
  scratch buffer is hidden from tabs, breadcrumbs, the statusline file name,
  and the buffer picker when no file is open.
- **Automatic file reload** with silent reload for clean buffers and a
  Keep/Reload/Overwrite prompt for dirty buffers.
- **F1 action picker** (`F1` or `<space>t`): base64/hex encode/decode, case
  conversions, line sorting, minify/beautify, format document, and create
  file/folder from a relative path (parent folders are created automatically).
  The picker uses a compact centered overlay.
- **Diagnostics report** (`:diagnostics` or `<space>i`): version,
  config/theme/log paths, session state, pane layout, and per-buffer
  language/syntax/diagnostic/LSP information.
- **Current-file diff** (`:diff`): opens a read-only scratch buffer with
  `git diff HEAD` for the current file.
- **Large-file guard**: files over `large-file-size-limit` (default 5 MB) open
  without language detection, syntax highlighting, or LSP, with a status
  warning.
- **Always-visible Zellij-style buffer tabs** (`bufferline = "always"`):
  numbered compact tabs with file-type icons, muted separators, an
  accent-colored active tab (no bold/underline), and a modified dot. The active
  tab stays visible when the strip overflows.
- **Full-width powerline**: spans the tree and buffers. Left:
  `[mode][spinner][branch][file name][read-only][modified]`; right:
  `[diagnostics][selections][selection length] │ [position][total lines] │
  [encoding][line ending][indent style][file type]`.
- **Terminal title**: the window/tab title is set to `Hx | <buffer>`.
- **Layout**: `[panes][tabs]`, then `[panes][breadcrumbs]`, then
  `[panes][code]`, then a full-width command/error line, with the full-width
  powerline at the very bottom. Breadcrumbs show the current file path relative
  to the project.
- **Startup panes**: opening a file, stdin, or a git commit message starts with
  the tree and tabs hidden; opening a folder restores the last global pane
  layout. `<space>T` toggles the tab bar, and pane visibility is saved globally
  (not per workspace) for folder sessions. File launches never persist their
  temporary hidden panes, so a later `hx .` is not mistaken for a single-file
  launch. Directory startup no longer opens the slow file picker, and git
  status is collected in a background worker.
- **Binary-file guard**: images and other binary files are detected before
  decoding and reported instead of rendering terminal garbage.
- **Monokai Community (Filter Spectrum)** as the default theme, with token
  colors mirrored from VSCode's Monokai theme extension (including Markdown
  headings, code, links, and quotes).

### Defaults

- Mouse buttons and middle-click paste are disabled (`mouse = false`,
  `middle-click-paste = false`), but native wheel scrolling is enabled by
  default (`mouse-native-scroll = true`). Set it to `false` for fully mouse-free
  behavior, or enable `mouse = true` for full mouse support.
- Cursor shape: underline in Normal/Select, I-beam-style bar in Insert.
- `cursorline`, `color-modes`, `auto-save`, and `true-color` are enabled.
- Auto-save runs on focus loss and after 1 second of inactivity.
- `scrolloff = 0`, `insert-final-newline = false`, and `popup-border = "all"`.
- File picker/tree show hidden files (`hidden = false`); symlink
  deduplication is off (`deduplicate-links = false`).
- Large-file guard: `large-file-size-limit = 5242880` bytes.
- Crash recovery: `crash-recovery = true`.
- Smart tab is disabled.
- The statusline defaults to:
  - left: mode, spinner, branch, file name, read-only, modified
  - right: diagnostics, selections, selection length, position, total lines,
    encoding, line ending, indentation style, file type
  - diagnostics severity: warning, error, hint
- Soft wrap is enabled; wrapped rows count as one logical line for `j`/`k`,
  arrow keys, selections, and counts such as `5x`. The wrap marker is shown in
  the gutter with the same muted color as whitespace markers, so code and
  underlines are not shifted.
- Indent guides are enabled with `▏`, skipping the first level.
- Spaces and tabs are visible by default using `·` and `→`; NBSP, narrow
  NBSP, and newline markers are off.
- Smooth scrolling: `Ctrl-Up` / `Ctrl-Down` moves by `scroll-lines` (default 3)
  in the editor and in the focused tree.
- Line numbers default to `both`, rendering `absolute │ relative` side by side.
  Set `[editor] line-number = "absolute"` or `"relative"` to change it.
