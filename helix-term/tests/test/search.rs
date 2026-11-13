use helix_term::application::Application;
use helix_term::search;
use helix_view::doc;

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

    helpers::send_keys(&mut app, "<space>//needle<ret>").await?;
    wait_for_search_results(&mut app, 1).await;
    helpers::send_keys(&mut app, "<ret>").await?; // open first match

    let (view, doc) = helix_view::current_ref!(app.editor);
    assert!(doc.path().unwrap().ends_with("a.txt"));
    assert_eq!(doc.selection(view.id).primary().from(), 5); // start of "needle"
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn select_all_matches_in_file_creates_multiple_selections() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("a.txt"), "x\nx\nx\n")?;
    let mut app = helpers::AppBuilder::new().build()?;
    app.editor.workspace_root = dir.path().to_path_buf();

    helpers::send_keys(&mut app, "<space>//x<ret>").await?;
    wait_for_search_results(&mut app, 3).await;
    helpers::send_keys(&mut app, "<A-ret>").await?;

    let (view, doc) = helix_view::current_ref!(app.editor);
    assert_eq!(doc.selection(view.id).ranges().len(), 3);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn unicode_match_opens_at_char_offsets() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("u.txt"), "αβγ\n")?;
    let mut app = helpers::AppBuilder::new().build()?;
    app.editor.workspace_root = dir.path().to_path_buf();

    helpers::send_keys(&mut app, "<space>//β<ret>").await?;
    wait_for_search_results(&mut app, 1).await;
    helpers::send_keys(&mut app, "<ret>").await?;

    let (view, doc) = helix_view::current_ref!(app.editor);
    assert_eq!(doc.selection(view.id).primary().from(), 1);
    assert_eq!(doc.selection(view.id).primary().to(), 2);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn replace_all_updates_open_buffer_in_memory_and_disk_files() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let open = dir.path().join("open.txt");
    let closed = dir.path().join("closed.txt");
    std::fs::write(&open, "foo\n")?;
    std::fs::write(&closed, "foo\n")?;
    let mut app = helpers::AppBuilder::new().with_file(&open, None).build()?;
    app.editor.workspace_root = dir.path().to_path_buf();

    let re = search::build_regex("foo", search::SearchOptions::default())?;
    let doc_id = doc!(app.editor).id();
    let n = search::replace_in_open_document(
        &mut app.editor,
        doc_id,
        &re,
        "bar",
        search::SearchOptions::default(),
    )?;
    assert_eq!(n, 1);
    assert!(doc!(app.editor).is_modified());
    search::replace_in_file(&closed, &re, "bar", search::SearchOptions::default())?;
    assert_eq!(std::fs::read_to_string(&closed)?, "bar\n");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn global_space_shortcuts_work_from_search_results() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("a.txt"), "foo\n")?;
    let mut app = helpers::AppBuilder::new().build()?;
    app.editor.workspace_root = dir.path().to_path_buf();

    helpers::send_keys(&mut app, "<space>//foo<ret>").await?;
    wait_for_search_results(&mut app, 1).await;
    helpers::send_keys(&mut app, "<space>b").await?;
    assert_eq!(
        app.editor.sidebar.mode,
        helix_view::sidebar::SidebarMode::Files
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn slash_edits_then_enter_runs_and_tab_cycles_fields() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("a.txt"), "foo\n")?;
    let mut app = helpers::AppBuilder::new().build()?;
    app.editor.workspace_root = dir.path().to_path_buf();

    helpers::send_keys(&mut app, "<space>/").await?;
    // Plain typing is ignored until `/` is pressed.
    helpers::send_keys(&mut app, "foo").await?;
    assert_eq!(app.editor.sidebar.search.query, "");
    assert_eq!(app.search_result_count(), 0);

    helpers::send_keys(&mut app, "/foo").await?;
    assert_eq!(app.editor.sidebar.search.query, "foo");
    assert_eq!(app.search_result_count(), 0);

    // Tab cycles Find -> Replace.
    helpers::send_keys(&mut app, "<tab>bar").await?;
    assert_eq!(app.editor.sidebar.search.replacement, "bar");

    // Tab cycles Replace -> Find; Enter runs the search.
    helpers::send_keys(&mut app, "<tab><ret>").await?;
    wait_for_search_results(&mut app, 1).await;
    assert_eq!(app.editor.sidebar.search.query, "foo");

    // Pressing `/` again edits the existing value without clearing it.
    helpers::send_keys(&mut app, "/").await?;
    assert_eq!(app.editor.sidebar.search.query, "foo");

    // Reopening the window returns to results mode; typing stays ignored.
    helpers::send_keys(&mut app, "<esc><esc><space>/x").await?;
    assert_eq!(app.editor.sidebar.search.query, "foo");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn alt_or_cmd_delete_clears_search_inputs() -> anyhow::Result<()> {
    let mut app = helpers::AppBuilder::new().build()?;

    helpers::send_keys(&mut app, "<space>//foo").await?;
    assert_eq!(app.editor.sidebar.search.query, "foo");
    helpers::send_keys(&mut app, "<A-backspace>").await?;
    assert_eq!(app.editor.sidebar.search.query, "");

    helpers::send_keys(&mut app, "<tab>bar<A-del>").await?;
    assert_eq!(app.editor.sidebar.search.replacement, "");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn paste_goes_into_the_global_search_inputs() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("a.txt"), "foo\n")?;
    let mut app = helpers::AppBuilder::new().build()?;
    app.editor.workspace_root = dir.path().to_path_buf();

    helpers::send_keys(&mut app, "<space>/").await?;
    helpers::send_paste(&mut app, "foo").await?;
    assert_eq!(app.editor.sidebar.search.query, "foo");
    helpers::send_keys(&mut app, "<ret>").await?;
    assert!(!helix_view::doc!(app.editor)
        .text()
        .to_string()
        .contains("foo"));
    wait_for_search_results(&mut app, 1).await;

    helpers::send_keys(&mut app, "/<tab>").await?;
    helpers::send_paste(&mut app, "bar").await?;
    assert_eq!(app.editor.sidebar.search.replacement, "bar");
    assert!(!helix_view::doc!(app.editor)
        .text()
        .to_string()
        .contains("bar"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn esc_dismisses_search_options_before_the_window() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("a.txt"), "foo\n")?;
    let mut app = helpers::AppBuilder::new().build()?;
    app.editor.workspace_root = dir.path().to_path_buf();

    helpers::send_keys(&mut app, "<space>//foo<ret>").await?;
    wait_for_search_results(&mut app, 1).await;
    helpers::send_keys(&mut app, "<space>").await?;
    assert!(app.editor.autoinfo.is_some());
    assert!(app.editor.sidebar.search_modal);

    helpers::send_keys(&mut app, "<esc>").await?;
    assert!(app.editor.autoinfo.is_none());
    assert!(app.editor.sidebar.search_modal);

    helpers::send_keys(&mut app, "<esc>").await?;
    assert!(!app.editor.sidebar.search_modal);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn replace_all_from_search_pane_updates_disk_and_reports() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    std::fs::write(&a, "foo\n")?;
    std::fs::write(&b, "foo\n")?;
    let mut app = helpers::AppBuilder::new().build()?;
    app.editor.workspace_root = dir.path().to_path_buf();

    helpers::send_keys(&mut app, "<space>//foo<ret>").await?;
    wait_for_search_results(&mut app, 2).await;
    // Put the replace text into the pane, then replace across the result set.
    app.editor.sidebar.search.replacement = "bar".into();
    helpers::send_keys(&mut app, "<C-A-ret>").await?;

    assert_eq!(std::fs::read_to_string(&a)?, "bar\n");
    assert_eq!(std::fs::read_to_string(&b)?, "bar\n");
    assert!(app.editor.get_status().is_some());
    Ok(())
}
