use std::path::Path;

use helix_view::graphics::{Color, Style};

/// File-type icon for the buffer tab bar.
pub fn file_icon_style(path: &Path) -> Style {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let color = match (name.as_str(), extension.as_str()) {
        ("cargo.toml" | "cargo.lock", _) | (_, "rs") => (222, 165, 132),
        (_, "go" | "mod" | "sum") => (0, 173, 216),
        (_, "py" | "pyi" | "pyw") => (53, 114, 165),
        (_, "js" | "mjs" | "cjs") => (241, 224, 90),
        (_, "ts" | "mts" | "cts" | "tsx" | "jsx") => (49, 120, 198),
        (_, "json" | "jsonc") => (203, 203, 65),
        (_, "md" | "markdown" | "mdx") => (81, 154, 186),
        (_, "toml" | "ini" | "conf" | "cfg") => (156, 66, 33),
        (_, "yaml" | "yml") => (203, 23, 30),
        (_, "html" | "htm") => (227, 76, 38),
        (_, "css" | "scss" | "sass" | "less") => (86, 61, 124),
        (_, "sh" | "bash" | "zsh" | "fish") => (137, 224, 81),
        (_, "lua") => (81, 160, 207),
        (_, "c" | "h") => (85, 85, 85),
        (_, "cc" | "cpp" | "cxx" | "hpp" | "hh") => (243, 75, 125),
        (_, "java") => (176, 114, 25),
        (_, "rb") => (112, 21, 22),
        (_, "php") => (79, 93, 149),
        (_, "swift") => (240, 81, 56),
        (_, "sql") => (227, 140, 0),
        (_, "xml") => (0, 96, 172),
        (_, "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg") => (160, 116, 196),
        (_, "pdf") => (179, 11, 0),
        (_, "zip" | "tar" | "gz" | "bz2" | "xz" | "zst") => (235, 164, 0),
        (_, "lock") => (212, 167, 44),
        _ => (154, 165, 177),
    };
    Style::default().fg(Color::Rgb(color.0, color.1, color.2))
}

pub fn file_icon(path: &Path) -> &'static str {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match name.as_str() {
        "cargo.toml" | "cargo.lock" => return "\u{e7a8}",
        "dockerfile" => return "\u{f308}",
        "makefile" => return "\u{e673}",
        _ => {}
    }

    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "rs" => "\u{e7a8}",
        "go" | "mod" | "sum" => "\u{e627}",
        "py" | "pyi" | "pyw" => "\u{e73c}",
        "js" | "mjs" | "cjs" => "\u{e74e}",
        "jsx" | "tsx" => "\u{e7ba}",
        "ts" | "mts" | "cts" => "\u{e628}",
        "json" | "jsonc" => "\u{e60b}",
        "md" | "markdown" | "mdx" => "\u{e73e}",
        "toml" | "ini" | "conf" | "cfg" => "\u{e6b2}",
        "yaml" | "yml" => "\u{e615}",
        "html" | "htm" => "\u{e736}",
        "css" => "\u{e749}",
        "sh" | "bash" | "zsh" | "fish" => "\u{f489}",
        "lua" => "\u{e620}",
        "c" | "h" => "\u{e61e}",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => "\u{e61d}",
        "java" => "\u{e738}",
        "rb" => "\u{e739}",
        "php" => "\u{e73d}",
        "swift" => "\u{e755}",
        "sql" => "\u{e706}",
        "xml" => "\u{e619}",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg" => "\u{f03e}",
        "pdf" => "\u{f1c1}",
        "zip" | "tar" | "gz" | "bz2" | "xz" | "zst" => "\u{f1c6}",
        "lock" => "\u{f023}",
        _ => "\u{f15b}",
    }
}
