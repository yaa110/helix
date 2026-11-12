use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Duration;

use helix_view::sidebar::SidebarMode;
use helix_view::Editor;
use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, Debouncer};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

/// A single debounced OS watcher shared by auto-reload and tree refresh.
pub struct FileWatcher {
    debouncer: Debouncer<RecommendedWatcher>,
    tx: UnboundedSender<Vec<PathBuf>>,
    doc_dirs: HashSet<PathBuf>,
    sidebar_dirs: HashSet<PathBuf>,
    refcounts: HashMap<PathBuf, usize>,
}

impl FileWatcher {
    pub fn new(debounce: Duration) -> anyhow::Result<(Self, UnboundedReceiver<Vec<PathBuf>>)> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let events_tx = tx.clone();
        let debouncer = new_debouncer(debounce, move |result: DebounceEventResult| match result {
            Ok(events) => {
                let paths: Vec<PathBuf> = events.into_iter().map(|event| event.path).collect();
                if !paths.is_empty() {
                    let _ = events_tx.send(paths);
                }
            }
            Err(err) => log::warn!("file watcher error: {err}"),
        })?;

        Ok((
            Self {
                debouncer,
                tx,
                doc_dirs: HashSet::new(),
                sidebar_dirs: HashSet::new(),
                refcounts: HashMap::new(),
            },
            rx,
        ))
    }

    /// Replace the complete watch set. Directories needed by both documents and
    /// the sidebar are watched once (tracked by the refcount map).
    pub fn set_watch_set(&mut self, doc_dirs: HashSet<PathBuf>, sidebar_dirs: HashSet<PathBuf>) {
        let mut desired: HashMap<PathBuf, usize> = HashMap::new();
        for dir in &doc_dirs {
            *desired.entry(dir.clone()).or_default() += 1;
        }
        for dir in &sidebar_dirs {
            *desired.entry(dir.clone()).or_default() += 1;
        }

        for dir in desired.keys() {
            if !self.refcounts.contains_key(dir) {
                if let Err(err) = self
                    .debouncer
                    .watcher()
                    .watch(dir, RecursiveMode::NonRecursive)
                {
                    log::warn!("failed to watch {dir:?}: {err}");
                }
            }
        }

        let stale: Vec<PathBuf> = self
            .refcounts
            .keys()
            .filter(|dir| !desired.contains_key(*dir))
            .cloned()
            .collect();
        for dir in stale {
            if let Err(err) = self.debouncer.watcher().unwatch(&dir) {
                log::warn!("failed to unwatch {dir:?}: {err}");
            }
        }

        self.refcounts = desired;
        self.doc_dirs = doc_dirs;
        self.sidebar_dirs = sidebar_dirs;
    }

    /// Watch the parent directories of all open documents.
    pub fn sync_documents(&mut self, editor: &Editor) {
        let dirs = editor
            .documents()
            .filter_map(|doc| doc.path())
            .filter_map(|path| path.parent().map(PathBuf::from))
            .collect();
        let sidebar = self.sidebar_dirs.clone();
        self.set_watch_set(dirs, sidebar);
    }

    /// Watch the directories needed to keep the visible tree fresh.
    pub fn sync_sidebar(&mut self, editor: &Editor) {
        let dirs = if editor.sidebar.mode == SidebarMode::Files {
            let mut dirs = HashSet::new();
            if !editor.workspace_root.as_os_str().is_empty() {
                dirs.insert(editor.workspace_root.clone());
            }
            for dir in &editor.sidebar.expanded {
                dirs.insert(dir.clone());
            }
            dirs
        } else {
            HashSet::new()
        };
        let docs = self.doc_dirs.clone();
        self.set_watch_set(docs, dirs);
    }

    /// The sending half of the debounced event channel. Kept so the channel
    /// stays open for the lifetime of the watcher.
    pub fn sender(&self) -> UnboundedSender<Vec<PathBuf>> {
        self.tx.clone()
    }

    pub fn doc_dirs(&self) -> &HashSet<PathBuf> {
        &self.doc_dirs
    }

    pub fn sidebar_dirs(&self) -> &HashSet<PathBuf> {
        &self.sidebar_dirs
    }
}

fn disk_content_matches(doc: &helix_view::Document) -> bool {
    let Some(path) = doc.path() else {
        return false;
    };
    match std::fs::read_to_string(path) {
        Ok(contents) => contents == doc.text().to_string(),
        Err(_) => false,
    }
}

/// Apply debounced filesystem paths to the editor: reload clean buffers,
/// queue conflicts for dirty ones, and mark tree directories for refresh.
pub fn apply_events(editor: &mut Editor, paths: &[PathBuf]) {
    for raw_path in paths {
        let path = helix_stdx::path::canonicalize(raw_path);
        if let Some(doc_id) = editor.document_id_by_path(&path) {
            if !path.exists() {
                editor.set_status(format!(
                    "'{}' deleted on disk",
                    path.file_name()
                        .map(|name| name.to_string_lossy())
                        .unwrap_or_default()
                ));
            } else {
                let disk_modified = std::fs::metadata(&path)
                    .and_then(|metadata| metadata.modified())
                    .unwrap_or_else(|_| std::time::SystemTime::now());
                let (modified, matches) = match editor.documents.get(&doc_id) {
                    Some(doc) => (doc.is_modified(), disk_content_matches(doc)),
                    None => continue,
                };
                if matches {
                    if let Some(doc) = editor.documents.get_mut(&doc_id) {
                        doc.pickup_last_saved_time();
                    }
                } else if modified {
                    editor.mark_reload_conflict(doc_id, disk_modified);
                    editor.set_status("file changed on disk; resolve the reload conflict");
                } else if let Err(err) = editor.reload_document(doc_id) {
                    editor.set_error(format!("failed to reload: {err}"));
                } else {
                    editor.set_status("reloaded from disk");
                }
            }
        }

        if let Some(parent) = path.parent().map(PathBuf::from) {
            editor.sidebar.refresh_dirs.insert(parent);
            editor.sidebar.refresh_epoch = editor.sidebar.refresh_epoch.wrapping_add(1);
            editor.needs_redraw = true;
        }
    }
}

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
