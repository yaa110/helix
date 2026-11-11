use helix_view::editor::Action;
use helix_view::session::{self, SessionStore};
use helix_view::view::ViewPosition;

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

    let mut restored = helpers::AppBuilder::new()
        .with_session_store(store)
        .with_workspace_dir(dir.path())
        .with_config(session_config())
        .build()?;
    assert_eq!(restored.editor.documents().count(), 1);
    let (view, doc) = helix_view::current_ref!(restored.editor);
    assert_eq!(doc.text().to_string(), "alpha\nbeta\ngamma\n");
    assert_eq!(
        doc.selection(view.id),
        &helix_core::Selection::single(2, 8)
    );
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
