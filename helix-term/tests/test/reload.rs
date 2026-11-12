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
