use helix_term::application::Application;

use super::*;

async fn wait_for_search_results(app: &mut Application, expected: usize) {
    for _ in 0..60 {
        run_event_loop_until_idle(app).await;
        if app.search_result_count() >= expected {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!(
        "search results did not arrive (have {})",
        app.search_result_count()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn search_pane_finds_and_opens_a_match() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("a.txt"), "zero\nneedle\n")?;
    let mut app = helpers::AppBuilder::new().build()?;
    app.editor.workspace_root = dir.path().to_path_buf();

    helpers::send_keys(&mut app, "<space>/needle").await?;
    wait_for_search_results(&mut app, 1).await;
    helpers::send_keys(&mut app, "<ret>").await?; // query -> results
    helpers::send_keys(&mut app, "<ret>").await?; // open first match

    let (view, doc) = helix_view::current_ref!(app.editor);
    assert!(doc.path().unwrap().ends_with("a.txt"));
    assert_eq!(doc.selection(view.id).primary().from(), 5); // start of "needle"
    Ok(())
}
