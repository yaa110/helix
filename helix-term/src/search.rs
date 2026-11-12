use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use helix_core::Transaction;
use helix_view::{DocumentId, Editor};
use regex::{Captures, Regex, RegexBuilder};

#[derive(Clone, Copy, Debug, Default)]
pub struct SearchOptions {
    pub match_case: bool,
    pub whole_word: bool,
    pub regex: bool,
    pub preserve_case: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    pub path: PathBuf,
    pub line: usize,
    pub col: usize,
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub preview: String,
}

#[derive(Clone, Debug)]
pub enum SearchEvent {
    Match(Match),
    Done { capped: bool },
    Error(String),
}

pub fn build_regex(query: &str, options: SearchOptions) -> Result<Regex, regex::Error> {
    let pattern = if options.regex {
        query.to_string()
    } else {
        regex::escape(query)
    };
    let pattern = if options.whole_word {
        format!(r"\b(?:{pattern})\b")
    } else {
        pattern
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(!options.match_case)
        .multi_line(true)
        .build()
}

pub fn find_matches(text: &str, regex: &Regex) -> Vec<(usize, usize, String)> {
    regex
        .find_iter(text)
        .map(|match_| (match_.start(), match_.end(), match_.as_str().to_string()))
        .collect()
}

pub fn replacement_for(
    caps: Option<&Captures<'_>>,
    matched: &str,
    template: &str,
    options: SearchOptions,
) -> String {
    let mut out = if options.regex {
        match caps {
            Some(caps) => {
                let mut out = String::new();
                caps.expand(template, &mut out);
                out
            }
            None => template.to_string(),
        }
    } else {
        template.to_string()
    };

    if options.preserve_case {
        out = preserve_case(matched, &out);
    }
    out
}

fn preserve_case(matched: &str, replacement: &str) -> String {
    if matched.is_empty() || replacement.is_empty() {
        return replacement.to_string();
    }
    let has_upper = matched.chars().any(|c| c.is_uppercase());
    let has_lower = matched.chars().any(|c| c.is_lowercase());
    if has_upper && !has_lower {
        return replacement.to_uppercase();
    }
    let mut chars = matched.chars();
    let first_upper = chars.next().is_some_and(|c| c.is_uppercase());
    let rest_lower = chars.all(|c| !c.is_uppercase());
    if first_upper && rest_lower {
        let mut out = String::new();
        let mut replacement_chars = replacement.chars();
        if let Some(first) = replacement_chars.next() {
            out.extend(first.to_uppercase());
        }
        out.extend(replacement_chars);
        return out;
    }
    replacement.to_string()
}

pub fn apply_replacements_to_text(
    text: &str,
    regex: &Regex,
    template: &str,
    options: SearchOptions,
) -> (String, usize) {
    let crlf = text.contains("\r\n");
    let mut output = String::with_capacity(text.len());
    let mut last = 0;
    let mut count = 0;

    for caps in regex.captures_iter(text) {
        let Some(matched) = caps.get(0) else {
            continue;
        };
        output.push_str(&text[last..matched.start()]);
        let mut replacement = replacement_for(Some(&caps), matched.as_str(), template, options);
        if crlf {
            replacement = replacement.replace("\r\n", "\n").replace('\n', "\r\n");
        }
        output.push_str(&replacement);
        last = matched.end();
        count += 1;
    }
    output.push_str(&text[last..]);
    (output, count)
}

pub fn write_atomic(path: &Path, contents: &str) -> anyhow::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(contents.as_bytes())?;
    tmp.flush()?;
    tmp.as_file().sync_all()?;
    if let Ok(metadata) = std::fs::metadata(path) {
        let _ = tmp.as_file().set_permissions(metadata.permissions());
    }
    #[cfg(windows)]
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    tmp.persist(path).map_err(|err| err.error)?;
    Ok(())
}

pub fn spawn_search(
    root: PathBuf,
    query: String,
    options: SearchOptions,
    open_docs: Vec<(PathBuf, String)>,
    tx: Sender<SearchEvent>,
    max_results: usize,
) {
    let regex = match build_regex(&query, options) {
        Ok(regex) => regex,
        Err(err) => {
            let _ = tx.send(SearchEvent::Error(err.to_string()));
            let _ = tx.send(SearchEvent::Done { capped: false });
            return;
        }
    };

    let mut emitted = 0usize;
    let mut capped = false;

    for (path, text) in &open_docs {
        for matched in matches_in_text(path, text, &regex) {
            if emitted >= max_results {
                capped = true;
                break;
            }
            emitted += 1;
            if tx.send(SearchEvent::Match(matched)).is_err() {
                return;
            }
        }
        if capped {
            break;
        }
    }

    if !capped {
        let open_paths: Vec<PathBuf> = open_docs.into_iter().map(|(path, _)| path).collect();
        for entry in ignore::WalkBuilder::new(&root)
            .standard_filters(true)
            .build()
        {
            let Ok(entry) = entry else {
                continue;
            };
            if !entry.path().is_file() || open_paths.iter().any(|path| path == entry.path()) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(entry.path()) else {
                continue;
            };
            for matched in matches_in_text(entry.path(), &text, &regex) {
                if emitted >= max_results {
                    capped = true;
                    break;
                }
                emitted += 1;
                if tx.send(SearchEvent::Match(matched)).is_err() {
                    let _ = tx.send(SearchEvent::Done { capped });
                    return;
                }
            }
            helix_event::request_redraw();
            if capped {
                break;
            }
        }
    }

    let _ = tx.send(SearchEvent::Done { capped });
    helix_event::request_redraw();
}

fn matches_in_text(path: &Path, text: &str, regex: &Regex) -> Vec<Match> {
    let mut results = Vec::new();
    let mut line = 0usize;
    let mut line_start = 0usize;
    for (start, end, matched_text) in find_matches(text, regex) {
        while let Some(newline) = text[line_start..start].find('\n') {
            line_start += newline + 1;
            line += 1;
        }
        let line_end = text[line_start..]
            .find('\n')
            .map(|offset| line_start + offset)
            .unwrap_or(text.len());
        let preview = text[line_start..line_end].trim_end().to_string();
        results.push(Match {
            path: path.to_path_buf(),
            line,
            col: start - line_start,
            start,
            end,
            text: matched_text,
            preview,
        });
    }
    results
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReplaceOutcome {
    pub replaced: usize,
    pub files: usize,
    pub skipped: Vec<PathBuf>,
}

/// Apply a replacement to one open document as a single undoable transaction.
pub fn replace_in_open_document(
    editor: &mut Editor,
    doc_id: DocumentId,
    regex: &Regex,
    template: &str,
    options: SearchOptions,
) -> anyhow::Result<usize> {
    let view_id = helix_view::view!(editor).id;
    let changes: Vec<_> = {
        let doc = editor.documents.get_mut(&doc_id).unwrap();
        doc.ensure_view_init(view_id);
        let text = doc.text().to_string();
        regex
            .captures_iter(&text)
            .filter_map(|caps| {
                let matched = caps.get(0)?;
                let replacement =
                    replacement_for(Some(&caps), matched.as_str(), template, options);
                Some((
                    doc.text().byte_to_char(matched.start()),
                    doc.text().byte_to_char(matched.end()),
                    Some(replacement.into()),
                ))
            })
            .collect()
    };
    let count = changes.len();
    if count == 0 {
        return Ok(0);
    }
    let doc = helix_view::doc_mut!(editor, &doc_id);
    let view = helix_view::view_mut!(editor, view_id);
    let transaction = Transaction::change(doc.text(), changes.into_iter());
    doc.apply(&transaction, view_id);
    doc.append_changes_to_history(view);
    Ok(count)
}

/// Replace in a file that is not necessarily open. The file is re-read and
/// re-searched immediately before writing so stale offsets cannot clobber it.
pub fn replace_in_file(
    path: &Path,
    regex: &Regex,
    template: &str,
    options: SearchOptions,
) -> anyhow::Result<usize> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(0);
    };
    let (output, count) = apply_replacements_to_text(&text, regex, template, options);
    if count == 0 {
        return Ok(0);
    }
    write_atomic(path, &output)?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(match_case: bool, whole_word: bool, regex: bool) -> SearchOptions {
        SearchOptions {
            match_case,
            whole_word,
            regex,
            preserve_case: false,
        }
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
        assert_eq!(
            replacement_for(Some(&caps), "hello", "[$1]", opts(false, false, true)),
            "[hello]"
        );
        assert_eq!(
            replacement_for(
                Some(&caps),
                "HELLO",
                "world",
                SearchOptions {
                    preserve_case: true,
                    ..opts(false, false, false)
                }
            ),
            "WORLD"
        );
    }

    #[test]
    fn replacement_keeps_crlf_line_endings() {
        let re = build_regex("a", opts(false, false, false)).unwrap();
        let (out, count) =
            apply_replacements_to_text("a\r\nb\r\n", &re, "x\ny", opts(false, false, false));
        assert_eq!(count, 1);
        assert_eq!(out, "x\r\ny\r\nb\r\n");
    }

    #[test]
    fn matches_carry_line_and_byte_offsets() {
        let re = build_regex("β", opts(true, false, false)).unwrap();
        let matches = matches_in_text(Path::new("u.txt"), "αβ\nβ\n", &re);
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].line, 0);
        assert_eq!(matches[1].line, 1);
        assert_eq!(&"αβ\nβ\n"[matches[1].start..matches[1].end], "β");
    }

    #[test]
    fn replace_in_file_rereads_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "foo\n").unwrap();
        let re = build_regex(
            "foo",
            SearchOptions {
                match_case: true,
                ..SearchOptions::default()
            },
        )
        .unwrap();

        std::fs::write(&path, "xfoo\n").unwrap();
        assert_eq!(
            replace_in_file(&path, &re, "bar", SearchOptions::default()).unwrap(),
            1
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "xbar\n");
    }

    #[test]
    fn replace_in_file_skips_binary_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bin");
        let original = [0xff_u8, 0xfe, b'f', b'o', b'o'];
        std::fs::write(&path, original).unwrap();
        let re = build_regex("foo", SearchOptions::default()).unwrap();
        assert_eq!(
            replace_in_file(&path, &re, "bar", SearchOptions::default()).unwrap(),
            0
        );
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
}
