use helix_term::file_watcher;
use helix_view::doc;

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn reload_document_refreshes_text_and_clears_modified() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("a.txt");
    std::fs::write(&file, "old\n")?;
    let mut app = helpers::AppBuilder::new().with_file(&file, None).build()?;
    let doc_id = doc!(app.editor).id();
    {
        let (view, doc) = helix_view::current!(app.editor);
        let transaction = helix_core::Transaction::insert(
            doc.text(),
            doc.selection(view.id),
            "new".into(),
        );
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
        let transaction = helix_core::Transaction::insert(
            doc.text(),
            doc.selection(view.id),
            "local".into(),
        );
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
