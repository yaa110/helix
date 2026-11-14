# Helix Code

![Helix Improved screenshot](assets/helix2.png)

A personal, AI-assisted fork of [Helix](https://github.com/helix-editor/helix)
that keeps modal editing while adding VSCode-style workflow pieces. Everything
is implemented natively in Rust with no plugin layer.

> **Unofficial fork — read this first.** This is **not** upstream Helix and is
> not affiliated with or endorsed by the Helix maintainers. Substantial parts
> of the fork-specific code and documentation were written or modified with AI
> assistance and have not been reviewed upstream. **Issues cannot be resolved
> and pull requests are not managed here.** Use it at your own risk and do not
> report fork problems to upstream Helix.

## How to Install

Installers build from the `main` branch with native-CPU optimizations
(`-C target-cpu=native`), fat LTO, one codegen unit, and stripped binaries.
There are no release tags or prebuilt artifacts.

### Homebrew

```sh
brew tap yaa110/helix https://github.com/yaa110/helix
brew install yaa110/helix/helix
# update later
brew update && brew reinstall yaa110/helix/helix
```

### curl installer

```sh
curl -fsSL https://raw.githubusercontent.com/yaa110/helix/main/install.sh | sh
```

Requires **Rust 1.90+**, plus `git` and a C/C++ compiler for tree-sitter
grammars. Installs to `~/.local` by default (`~/.local/bin/hx` wrapper,
binary, and runtime). Add `~/.local/bin` to `PATH` if needed.

Useful options: `HELIX_INSTALL_DIR=/path` to change the prefix,
`HELIX_SKIP_GRAMMARS=1` to skip the grammar build. Verify with `hx --version`.

For keybindings and usage of fork-specific features, open the in-editor tutor
(`:tutor`) and see its forked-features chapter.

## New Features

| Feature | Description |
| --- | --- |
| File tree & sidebar | Persistent, focusable workspace tree with lazy loading, fuzzy search, jump labels, multi-select, file operations (create/rename/copy/cut/paste/trash delete with undo), split opens, ignored-file toggle, and Git-aware status/coloring. It auto-reveals the current buffer, supports horizontal scroll for long names (`<`/`>`), and can filter to dirty files. |
| Global search & replace | Floating search-and-replace workspace with ripgrep-backed results, source preview, persistent query/options/selection, and focused Find/Replace editing with command-line cursor behavior. Supports case/word/regex/preserve-case options, open-at-match, and per-result or replace-all actions with previews. |
| Workspace sessions & restore | Per-workspace session persistence for buffers, split layout, per-view selections and scroll, sidebar state, and search inputs, stored in a compact binary format. Sessions save on exit or on demand and restore safely, with crash-recovery support and a recent-workspaces picker (`<space>W`). |
| Automatic reload & conflict handling | Debounced file watching reloads clean buffers automatically when files change on disk and refreshes visible tree directories. Dirty buffers get a safe Keep/Reload/Overwrite prompt so external changes and local edits can be resolved without silent data loss. |
| Folder picker workspace switcher | `:open-folder`/`:folder` opens a lazy two-pane directory picker for switching to another folder as a workspace with session restore behavior. It supports fuzzy filtering, absolute-path navigation, HOME-bounded browsing, and previews of direct subdirectories without walking the full tree. |
| Peek navigation & code lenses | `gP`/`gR` open definition/reference peeks in a searchable floating preview picker (references exclude the declaration by default). Code lenses can be listed per-line or per-file, resolved automatically, and executed from the same peek-style picker, with the current line's lens surfaced in the statusline. |
| Inline completions & snippets | Renders LSP 3.18 inline completions and configurable user snippets as faint ghost text at the cursor. Completions are requested asynchronously, dismissed on edits/movement, and accepted explicitly (Tab), with snippet placeholder and common filename variable expansion. |
| F1 actions & file tools | F1/`<space>t` provides a compact action picker for text transforms, base64/hex encode-decode, case conversions, line sorting, minify/beautify, document formatting, and creating files/folders from relative paths. It streamlines common editing and file-creation workflows without leaving the editor. |
| Diagnostics & diff utilities | Adds `<space>i` (diagnostics report) for an in-editor environment, config/session, and per-buffer syntax/diagnostic/LSP overview. Also provides a read-only `git diff HEAD` view for the current file via the command palette/static command (*Show git diff for the current file*) to review local changes in-place. |
| VSCode-like layout, tabs & statusline | Uses a pane-aware layout: `[panes][tabs]`, `[panes][breadcrumbs]`, `[panes][code]`, a full-width command/error line, and a full-width powerline at the bottom. Buffer tabs are always visible and numbered with file-type icons, muted separators, an accent active tab (no bold/underline), and modified markers; `<space>T` toggles the tab bar. |
| Breadcrumbs, powerline & window chrome | Shows breadcrumbs with the current file path relative to the project, and sets the terminal/window title to `Hx | <buffer>`. The full-width powerline spans tree and buffers with left `[mode][spinner][branch][file][read-only][modified]` and right `[diagnostics][selections][selection length] │ [position][total lines] │ [encoding][line ending][indent][file type]`, reflecting focused-pane/mode state. |
| Startup panes & global UI state | Folder launches restore the last global pane layout, while file/stdin/commit launches start with tree and tabs hidden; file launches do not persist temporary hidden panes. Pane visibility is saved globally (not per workspace) for predictable behavior across sessions. |
| Mouse modes & wheel scrolling | Configurable `mouse` modes (`navigation` default, `inherit`, `disable`) balance terminal text selection with in-app interaction. Navigation mode enables focus clicks and accelerated wheel scrolling across editor, tree, search, pickers, menus, and popups while keeping the selection clamped to the viewport. |
| Search, editor & command-line UX | Tree and search inputs use a real I-beam cursor with rich editing keys (arrows, word movement/deletion, clear shortcuts) and mode-aware hints. The fork also adds jump labels, categorized hint menus, global shortcut pass-through from focused panes, and picker-opened files revealed in the tree. |
| Rainbow brackets | Rainbow brackets are enabled by default and color nested bracket pairs by depth using a VSCode Dark+-style palette (`#FFD700`, `#DA70D6`, `#179FFF`). Themes can override the palette with a `rainbow` scope for consistent, readable nesting. |
| Defaults, theme & visual guides | Defaults to Monokai Community (Filter Spectrum), diff gutter, `both` line numbers (shows the absolute line number and the relative distance from the cursor side by side, `absolute │ relative`), soft wrap with gutter markers, indent/whitespace guides, cursorline, bordered popups, autosave, and refined cursor shapes. These defaults aim for a readable, IDE-like out-of-box experience while remaining configurable. |
| Large/binary files & startup behavior | Detects binary files before decoding and guards large files (default 5 MB) by skipping language detection, syntax highlighting, and LSP with a warning. Directory startup avoids the slow file picker and background work (like git status collection) keeps the UI responsive. |
