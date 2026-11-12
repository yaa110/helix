use helix_term::application::Application;
use helix_term::file_watcher;
use helix_view::doc;

use super::*;

fn make_dirty(app: &mut Application, text: &str) {
    let (view, doc) = helix_view::current!(app.editor);
    let transaction =
        helix_core::Transaction::insert(doc.text(), doc.selection(view.id), text.into());
    doc.apply(&transaction, view.id);
    doc.append_changes_to_history(view);
}

async fn open_conflict(app: &mut Application, file: &std::path::Path) -> anyhow::Result<()> {
    make_dirty(app, "local");
    std::fs::write(file, "external\n")?;
    file_watcher::apply_events(&mut app.editor, &[file.to_path_buf()]);
    app.handle_idle_timeout().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn reload_document_refreshes_text_and_clears_modified() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "old\n")?;
    let mut app = helpers::AppBuilder::new().with_file(&file, None).build()?;
    let doc_id = doc!(app.editor).id();
    {
        let (view, doc) = helix_view::current!(app.editor);
        let transaction =
            helix_core::Transaction::insert(doc.text(), doc.selection(view.id), "new".into());
        doc.apply(&transaction, view.id);
        doc.append_changes_to_history(view);
    }
    assert!(doc!(app.editor).is_modified());

    std::fs::write(&file, "disk\n")?;
    app.editor.reload_document(doc_id)?;
    assert_eq!(doc!(app.editor).text().to_string(), "disk\n");
    assert!(!doc!(app.editor).is_modified());
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn clean_buffer_reloads_and_dirty_buffer_conflicts() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "old\n")?;
    let mut app = helpers::AppBuilder::new().with_file(&file, None).build()?;

    std::fs::write(&file, "new\n")?;
    file_watcher::apply_events(&mut app.editor, &[file.clone()]);
    assert_eq!(doc!(app.editor).text().to_string(), "new\n");

    let doc_id = doc!(app.editor).id();
    {
        let (view, doc) = helix_view::current!(app.editor);
        let transaction =
            helix_core::Transaction::insert(doc.text(), doc.selection(view.id), "local".into());
        doc.apply(&transaction, view.id);
        doc.append_changes_to_history(view);
    }
    let local = doc!(app.editor).text().to_string();
    std::fs::write(&file, "external\n")?;
    file_watcher::apply_events(&mut app.editor, &[file.clone()]);
    assert_eq!(doc!(app.editor).text().to_string(), local);
    assert!(app.editor.reload_conflict(doc_id).is_some());
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn matching_disk_content_does_not_reload_or_conflict() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "text\n")?;
    let mut app = helpers::AppBuilder::new().with_file(&file, None).build()?;
    let doc_id = doc!(app.editor).id();

    // Simulate the content after an own save: disk and buffer agree.
    file_watcher::apply_events(&mut app.editor, &[file]);
    assert_eq!(doc!(app.editor).text().to_string(), "text\n");
    assert!(!doc!(app.editor).is_modified());
    assert!(app.editor.reload_conflict(doc_id).is_none());
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn conflict_menu_reload() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "old\n")?;
    let mut app = helpers::AppBuilder::new().with_file(&file, None).build()?;
    open_conflict(&mut app, &file).await?;

    let check = |app: &Application| {
        assert_eq!(doc!(app.editor).text().to_string(), "external\n");
        assert!(!doc!(app.editor).is_modified());
    };
    test_key_sequences(
        &mut app,
        vec![
            (Some("j<ret>"), Some(&check)), // Keep -> Reload
            (Some(":q!<ret>"), None),
        ],
        true,
    )
    .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn conflict_menu_overwrite() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "old\n")?;
    let mut app = helpers::AppBuilder::new().with_file(&file, None).build()?;
    open_conflict(&mut app, &file).await?;
    let local = doc!(app.editor).text().to_string();

    let check = |_app: &Application| {
        assert_eq!(std::fs::read_to_string(&file).unwrap(), local);
    };
    test_key_sequences(
        &mut app,
        vec![
            (Some("jj<ret>"), Some(&check)), // Keep -> Reload -> Overwrite
            (Some(":q!<ret>"), None),
        ],
        true,
    )
    .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn conflict_menu_keep() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "old\n")?;
    let mut app = helpers::AppBuilder::new().with_file(&file, None).build()?;
    open_conflict(&mut app, &file).await?;
    let local = doc!(app.editor).text().to_string();

    let check = |app: &Application| {
        assert_eq!(doc!(app.editor).text().to_string(), local);
        assert!(app.editor.reload_conflicts.is_empty());
    };
    test_key_sequences(
        &mut app,
        vec![
            (Some("<ret>"), Some(&check)), // default Keep
            (Some(":q!<ret>"), None),
        ],
        true,
    )
    .await?;
    Ok(())
}
