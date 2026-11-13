use std::path::Path;

/// Nerd Font file-type icons. Terminals without a Nerd Font can disable these
/// with `[editor.sidebar] icons = false`.
pub fn folder_icon(expanded: bool) -> &'static str {
    if expanded {
        "\u{f07c}" // nf-fa-folder_open
    } else {
        "\u{f07b}" // nf-fa-folder
    }
}

pub fn file_icon(path: &Path) -> &'static str {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match name.as_str() {
        "cargo.toml" | "cargo.lock" => return "\u{e7a8}",
        "dockerfile" | "docker-compose.yml" | "docker-compose.yaml" => return "\u{f308}",
        "makefile" | "gnumakefile" => return "\u{e673}",
        ".gitignore" | ".gitattributes" | ".gitmodules" => return "\u{e702}",
        "license" | "license.md" | "license.txt" => return "\u{f02d}",
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
        "scss" | "sass" | "less" => "\u{e74b}",
        "sh" | "bash" | "zsh" | "fish" => "\u{f489}",
        "lua" => "\u{e620}",
        "c" | "h" => "\u{e61e}",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => "\u{e61d}",
        "java" => "\u{e738}",
        "kt" | "kts" => "\u{e634}",
        "rb" => "\u{e739}",
        "php" => "\u{e73d}",
        "swift" => "\u{e755}",
        "sql" => "\u{e706}",
        "xml" => "\u{e619}",
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg" => "\u{f03e}",
        "pdf" => "\u{f1c1}",
        "zip" | "tar" | "gz" | "bz2" | "xz" | "zst" => "\u{f1c6}",
        "lock" => "\u{f023}",
        _ => "\u{f15b}", // nf-fa-file
    }
}
