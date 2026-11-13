# Helix Improved

![Helix Improved screenshot](assets/helix.png)

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

Both installers build from the `main` branch. There are no release tags or
prebuilt artifacts.

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
it. The tree is search-first: typing filters it live with fuzzy matching.

| Direct key | Action |
| --- | --- |
| printable characters | Live fuzzy filter |
| `Backspace` | Delete the last query character |
| `Ctrl-u` / `Ctrl-w` | Clear the query / delete the last query word |
| `Down` / `Up` | Move the selection |
| `Ctrl-Down` / `Ctrl-Up` | Smooth-scroll by `scroll-lines` (default 3) |
| `Right` / `Left` | Expand/open, or collapse/parent |
| `Enter` | Open the selected file and focus the editor |
| `Tab` | Open the selected file, or return to the editor on a directory |
| `Esc` | Clear the filter; when empty, return focus to the editor |

Press `<space>` in the tree to open a which-key-style command hint box:

| Space command | Action |
| --- | --- |
| `j` / `k` | Move down / up |
| `h` / `l` | Collapse / expand |
| `Enter`, `o` | Open |
| `s` / `v` | Open in a horizontal / vertical split |
| `a` / `A` | Create a file / directory |
| `r` | Rename |
| `d` | Delete (confirmation defaults to No) |
| `c` / `m` / `p` | Copy / cut / paste |
| `y` / `Y` | Copy the absolute / relative path |
| `x` | Close the current buffer |
| `w` | Show two-letter jump labels |
| `g` / `G` | First / last row |
| `J` / `K` | Move ten rows |
| `H` / `R` | Collapse all / refresh |
| `D` | Toggle dirty-files-only view |
| `.` | Reveal the current buffer |
| `[` / `]` | Shrink / grow the sidebar |
| `<` / `>` | Scroll long rows left / right |
| `q` / `Esc` | Return to the editor / close the command box |

Additional tree behavior:

- File-type icons and indentation guides are enabled by default. Set
  `[editor.sidebar] icons = false` if your terminal font lacks Nerd Font glyphs.
- Git status colors: modified files use the `diff.delta` color, untracked files
  use `diff.plus`, conflicts use `diff.delta.conflict`, and directories inherit
  the color of changed children. `<space>D` shows dirty files only.
- Long names are clipped at the pane boundary; the statusline shows the full
  selected path, and `<space><` / `<space>>` scroll long rows.
- Focus is obvious: the selected row has a `▍` bar, the focused pane gets a
  high-contrast border/header, and the inactive selection stays dimmed.
- Opening a file from any picker expands and selects it in the tree.
- `<space>x` closes the current buffer (same as `:buffer-close`).
- `<space>e` / `<space>E` open the file explorer at the workspace / current
  working directory.

### Search pane

`<space>/` opens the sidebar search pane.

- Visible `Find:` and `Replace:` inputs; long queries scroll inside the pane
  instead of overflowing.
- Empty query shows `Type to search`; no matches show `No results found`.
- Options line: `[Aa] case`, `[ab] word`, `[.*] regex`, `[AB] keep`.
- Results are grouped by file, streamed in, debounced, cancellable, and capped
  at 10,000 matches.
- Open buffers are searched from memory; other files are read from disk.
- Results revalidate against current text before replacing.

| Key | Action |
| --- | --- |
| `Tab` | Query → Replace → Results |
| `Space` (Results) / `Alt-Space` | Open the search-options hint box |
| `c` / `w` / `r` / `p` (options box) | Toggle case / word / regex / preserve-case |
| `Alt-c` / `Alt-w` / `Alt-r` / `Alt-p` | Toggle the same options directly |
| `j` / `k` | Move through results |
| `Ctrl-Down` / `Ctrl-Up` | Smooth-scroll results |
| `h` / `l` | Collapse / expand a file group |
| `Enter` | Open and select the match |
| `s` / `v` | Open the match in a split |
| `n` / `N`, `F4` / `Shift-F4` | Next / previous match |
| `Alt-Enter` | Select all matches in the current file |
| `Ctrl-r` | Replace the selected match (or all matches in the selected file) |
| `Ctrl-Alt-Enter` | Replace all matches |
| `R` | Re-run the query |
| `Esc` | Results → editor |

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

- **Session restore** of buffers, selections, cursor/scroll, sidebar state, and
  search inputs per workspace.
- **Automatic file reload** with silent reload for clean buffers and a
  Keep/Reload/Overwrite prompt for dirty buffers.
- **Searchable text transforms** with `F1` or `<space>t`: base64/hex encode and
  decode, case conversions, and line sorting.
- **Always-visible VSCode-like buffer tabs** (`bufferline = "always"`) with
  file icons, inactive separators, bold active tabs, and a colored modified dot.
- **Full-width powerline**: spans the tree and buffers. Left:
  `[mode][branch][diagnostics]`; right: `[selections][total lines][indent mode]
  [Tab/Spaces: N][encoding][line ending][file type]`.
- **Binary-file guard**: images and other binary files are detected before
  decoding and reported instead of rendering terminal garbage.
- **Monokai Community (Filter Spectrum)** as the default theme.

### Defaults

- Mouse capture is disabled by default. Enable with `mouse = true`.
- Cursor shape: underline in Normal/Select, I-beam-style bar in Insert.
- `cursorline`, `color-modes`, `auto-save`, and `true-color` are enabled.
- Soft wrap is enabled; counts such as `5x` still select five real lines.
- Indent guides are enabled with the conventional `│` character.
- Spaces and tabs are visible by default using `·` and `→`.
- Smooth scrolling: `Ctrl-Up` / `Ctrl-Down` moves by `scroll-lines` (default 3)
  in the editor and in focused tree/search panes.
- Line numbers can show both columns:

```toml
[editor]
line-number = "both"   # absolute, relative, or both
```

The `both` mode renders `relative │ absolute` side by side.
