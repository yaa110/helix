use helix_term::application::Application;
use helix_view::editor::Action;
use helix_view::session::{self, SessionStore};
use helix_view::sidebar::SidebarMode;
use helix_view::view::ViewPosition;
use std::path::PathBuf;

use super::*;

fn session_config() -> helix_term::config::Config {
    let mut config = helpers::test_config();
    config.editor.restore_session = true;
    config
}

#[tokio::test(flavor = "multi_thread")]
async fn restore_round_trip_preserves_buffers_and_positions() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("main.rs");
    std::fs::write(&file, "alpha\nbeta\ngamma\n")?;

    let mut app = helpers::AppBuilder::new().with_file(&file, None).build()?;
    app.editor.workspace_root = dir.path().to_path_buf();
    {
        let (view, doc) = helix_view::current!(app.editor);
        doc.set_selection(view.id, helix_core::Selection::single(2, 8));
        doc.set_view_offset(
            view.id,
            ViewPosition {
                anchor: 6,
                horizontal_offset: 0,
                vertical_offset: 0,
            },
        );
    }

    let store = SessionStore::new(dir.path().join("state"));
    let saved = session::collect(&app.editor);
    store.save(&saved)?;

    let restored = helpers::AppBuilder::new()
        .with_session_store(store)
        .with_workspace_dir(dir.path())
        .with_config(session_config())
        .build()?;
    assert_eq!(restored.editor.documents().count(), 1);
    let (view, doc) = helix_view::current_ref!(restored.editor);
    assert_eq!(doc.text().to_string(), "alpha\nbeta\ngamma\n");
    assert_eq!(doc.selection(view.id), &helix_core::Selection::single(2, 8));
    assert_eq!(
        doc.view_offset(view.id),
        ViewPosition {
            anchor: 6,
            horizontal_offset: 0,
            vertical_offset: 0,
        }
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn restoring_views_before_job_queue_init_does_not_deadlock() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let workspace = std::fs::canonicalize(dir.path())?;
    let file = workspace.join("one.txt");
    std::fs::write(&file, "one\n")?;

    let path = PathBuf::from("one.txt");
    let selection = session::SelectionState {
        anchor: session::Point { line: 0, col: 0 },
        head: session::Point { line: 0, col: 1 },
    };
    let saved = session::Session {
        version: session::SESSION_VERSION,
        workspace: workspace.clone(),
        focused: Some(path.clone()),
        buffers: vec![session::BufferState {
            path: path.clone(),
            selections: vec![selection.clone()],
            primary: 0,
            scroll: session::ScrollState::default(),
        }],
        views: vec![session::ViewSession {
            path,
            selections: vec![selection],
            primary: 0,
            scroll: session::ScrollState::default(),
            split: session::SplitKind::Replace,
        }],
        focused_view: 0,
        sidebar: session::SidebarSession::default(),
        search: Default::default(),
    };

    let store = SessionStore::new(workspace.join("state"));
    store.save(&saved)?;

    // Build only one Application in this runtime. Adding a second build would
    // initialize the job queue first and mask the startup deadlock.
    let app = helpers::AppBuilder::new()
        .with_session_store(store)
        .with_workspace_dir(&workspace)
        .with_config(session_config())
        .build()?;

    assert_eq!(app.editor.documents().count(), 1);
    assert!(app.editor.documents().all(|doc| !doc.is_pristine_scratch()));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn close_writes_session_and_explicit_files_do_not() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("one.txt");
    std::fs::write(&file, "one\n")?;
    let store = SessionStore::new(dir.path().join("state"));

    let mut app = helpers::AppBuilder::new()
        .with_session_store(store.clone())
        .with_workspace_dir(dir.path())
        .with_config(session_config())
        .build()?;
    app.editor.open(&file, Action::Load)?;
    app.close().await;
    assert!(store.load(dir.path()).is_some());

    let other = tempfile::tempdir()?;
    let file = other.path().join("two.txt");
    std::fs::write(&file, "two\n")?;
    let store2 = SessionStore::new(other.path().join("state"));
    let mut app = helpers::AppBuilder::new()
        .with_session_store(store2.clone())
        .with_file(&file, None)
        .with_config(session_config())
        .build()?;
    app.close().await;
    assert!(store2.load(other.path()).is_none());
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn session_commands_save_and_clear() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let store = SessionStore::new(dir.path().join("state"));
    let mut app = helpers::AppBuilder::new()
        .with_session_store(store.clone())
        .with_config(session_config())
        .build()?;
    app.editor.workspace_root = dir.path().to_path_buf();
    app.editor.session_active = true;

    let check_saved = |_app: &Application| assert!(store.load(dir.path()).is_some());
    let check_cleared = |_app: &Application| assert!(store.load(dir.path()).is_none());

    test_key_sequences(
        &mut app,
        vec![
            (Some(":session-save<ret>"), Some(&check_saved)),
            (Some(":session-clear<ret>"), Some(&check_cleared)),
            (Some(":q!<ret>"), None),
        ],
        true,
    )
    .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn sidebar_state_round_trips() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("a.txt"), "a\n")?;
    let mut app = helpers::AppBuilder::new()
        .with_workspace_dir(dir.path())
        .with_config(session_config())
        .build()?;
    app.editor.sidebar.mode = SidebarMode::Files;
    app.editor.sidebar.width = 34;
    app.editor.sidebar.expanded.insert(dir.path().to_path_buf());
    app.editor.sidebar.selected = Some(dir.path().join("a.txt"));

    let saved = helix_view::session::collect(&app.editor);
    assert_eq!(saved.sidebar.width, 34);
    assert_eq!(saved.sidebar.mode, SidebarMode::Files);
    assert!(saved.sidebar.selected.is_some());

    app.editor.sidebar.search.query = "foo".into();
    app.editor.sidebar.search.replacement = "bar".into();
    app.editor.sidebar.search.regex = true;
    let saved = helix_view::session::collect(&app.editor);
    assert_eq!(saved.search.query, "foo");
    assert_eq!(saved.search.replacement, "bar");
    assert!(saved.search.regex);
    Ok(())
}
