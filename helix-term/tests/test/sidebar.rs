use helix_term::application::Application;
use helix_term::file_watcher;
use helix_view::doc;
use helix_view::sidebar::{SidebarFocus, SidebarMode};

use super::*;

#[tokio::test(flavor = "multi_thread")]
async fn space_b_opens_files_pane_and_escape_returns_focus() -> anyhow::Result<()> {
    let mut app = helpers::AppBuilder::new().build()?;

    let opened = |app: &Application| {
        assert_eq!(app.editor.sidebar.mode, SidebarMode::Files);
        assert_eq!(app.editor.sidebar.focus, SidebarFocus::Sidebar);
    };
    let escaped = |app: &Application| {
        assert_eq!(app.editor.sidebar.focus, SidebarFocus::Editor);
        assert_eq!(app.editor.sidebar.mode, SidebarMode::Files);
    };

    test_key_sequences(
        &mut app,
        vec![
            (Some("<space>b"), Some(&opened)),
            (Some("<esc>"), Some(&escaped)),
            (Some(":q!<ret>"), None),
        ],
        true,
    )
    .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn sidebar_keys_do_not_move_editor_cursor() -> anyhow::Result<()> {
    let mut app = helpers::AppBuilder::new()
        .with_input_text("#[o|]#ne\ntwo\n")
        .build()?;
    let before = {
        let (v, d) = helix_view::current_ref!(app.editor);
        d.selection(v.id).clone()
    };

    let check = |app: &Application| {
        let (v, d) = helix_view::current_ref!(app.editor);
        assert_eq!(before, d.selection(v.id).clone());
    };
    test_key_sequences(
        &mut app,
        vec![
            (Some("<space>bjj"), Some(&check)),
            (Some("<esc>:q!<ret>"), None),
        ],
        true,
    )
    .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn tree_navigation_opens_files_and_moves_focus() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("a.txt"), "alpha\n")?;
    std::fs::write(dir.path().join("b.txt"), "beta\n")?;
    let mut app = helpers::AppBuilder::new().build()?;
    app.editor.workspace_root = dir.path().to_path_buf();

    let check = |app: &Application| {
        assert_eq!(app.editor.sidebar.focus, SidebarFocus::Editor);
        assert!(doc!(app.editor).path().unwrap().ends_with("a.txt"));
    };
    test_key_sequences(
        &mut app,
        vec![
            (Some("<space>b"), None),
            (Some("<ret>"), Some(&check)),
            (Some(":q!<ret>"), None),
        ],
        true,
    )
    .await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn tree_refreshes_after_external_create() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let mut app = helpers::AppBuilder::new().build()?;
    app.editor.workspace_root = dir.path().to_path_buf();
    app.editor.sidebar.mode = SidebarMode::Files;
    app.editor.sidebar.focus = SidebarFocus::Sidebar;

    let before = app.editor.sidebar.refresh_epoch;
    let file = dir.path().join("appeared.txt");
    std::fs::write(&file, "x")?;
    file_watcher::apply_events(&mut app.editor, &[file]);
    assert!(app.editor.sidebar.refresh_epoch > before);

    app.handle_idle_timeout().await;
    assert!(app
        .sidebar_row_paths()
        .iter()
        .any(|path| path.ends_with("appeared.txt")));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn global_ctrl_bindings_work_while_tree_is_focused() -> anyhow::Result<()> {
    use helix_core::hashmap;
    use helix_term::keymap;
    use helix_view::document::Mode;

    let mut config = helpers::test_config();
    config
        .keys
        .insert(Mode::Normal, keymap!({"Normal" "C-e" => toggle_file_tree,}));
    let mut app = helpers::AppBuilder::new().with_config(config).build()?;

    helpers::send_keys(&mut app, "<space>b").await?;
    assert_eq!(app.editor.sidebar.mode, SidebarMode::Files);
    helpers::send_keys(&mut app, "<C-e>").await?;
    assert_eq!(app.editor.sidebar.mode, SidebarMode::Hidden);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn tree_reveals_the_current_buffer() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    std::fs::write(&a, "a\n")?;
    std::fs::write(&b, "b\n")?;
    let mut app = helpers::AppBuilder::new().build()?;
    app.editor.workspace_root = dir.path().to_path_buf();
    app.editor.sidebar.mode = SidebarMode::Files;
    app.editor.sidebar.focus = SidebarFocus::Editor;

    app.editor.open(&b, helix_view::editor::Action::Replace)?;
    app.editor.needs_redraw = true;
    app.handle_idle_timeout().await;
    assert_eq!(app.editor.sidebar.selected.as_deref(), Some(b.as_path()));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn manual_tree_selection_survives_escaping_to_editor() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    std::fs::write(&a, "a\n")?;
    std::fs::write(&b, "b\n")?;
    let mut app = helpers::AppBuilder::new().build()?;
    app.editor.workspace_root = dir.path().to_path_buf();
    app.editor.open(&a, helix_view::editor::Action::Replace)?;
    app.editor.needs_redraw = true;
    app.handle_idle_timeout().await;

    helpers::send_keys(&mut app, "<space>b").await?;
    helpers::send_keys(&mut app, "j").await?;
    helpers::send_keys(&mut app, "<esc>").await?;
    app.editor.needs_redraw = true;
    app.handle_idle_timeout().await;

    assert_eq!(app.editor.sidebar.selected.as_deref(), Some(b.as_path()));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn gw_jump_labels_select_a_row() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("a.txt"), "a\n")?;
    let b = dir.path().join("b.txt");
    std::fs::write(&b, "b\n")?;
    let mut app = helpers::AppBuilder::new().build()?;
    app.editor.workspace_root = dir.path().to_path_buf();

    // Open the tree, start jump labels, then pick the second row ("ab").
    helpers::send_keys(&mut app, "<space>bgwab").await?;
    assert_eq!(app.editor.sidebar.selected.as_deref(), Some(b.as_path()));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn slash_starts_tree_search_and_escape_unwinds() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("a.txt"), "a\n")?;
    std::fs::write(dir.path().join("b.txt"), "b\n")?;
    let mut app = helpers::AppBuilder::new().build()?;
    app.editor.workspace_root = dir.path().to_path_buf();

    helpers::send_keys(&mut app, "<space>b").await?;

    // Plain typing no longer filters the tree.
    helpers::send_keys(&mut app, "a").await?;
    assert_eq!(app.sidebar_row_paths().len(), 2);

    // `/` starts search; only then does typing filter.
    helpers::send_keys(&mut app, "/a").await?;
    assert_eq!(app.sidebar_row_paths().len(), 1);
    assert!(app.editor.sidebar.tree_search_active);

    // First Esc cancels search, second leaves the tree.
    helpers::send_keys(&mut app, "<esc>").await?;
    assert_eq!(app.editor.sidebar.focus, SidebarFocus::Sidebar);
    assert!(app.editor.sidebar.tree_filter.is_empty());
    assert!(!app.editor.sidebar.tree_search_active);
    assert_eq!(app.sidebar_row_paths().len(), 2);

    helpers::send_keys(&mut app, "<esc>").await?;
    assert_eq!(app.editor.sidebar.focus, SidebarFocus::Editor);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn ctrl_w_moves_between_tree_and_editor_and_keeps_results() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("a.txt"), "a\n")?;
    std::fs::write(dir.path().join("b.txt"), "b\n")?;
    let mut app = helpers::AppBuilder::new().build()?;
    app.editor.workspace_root = dir.path().to_path_buf();

    helpers::send_keys(&mut app, "<space>b/a<C-w><right>").await?;
    assert_eq!(app.editor.sidebar.focus, SidebarFocus::Editor);
    assert_eq!(app.editor.sidebar.tree_filter, "a");
    assert!(!app.editor.sidebar.tree_search_active);
    assert_eq!(app.sidebar_row_paths().len(), 1);

    helpers::send_keys(&mut app, "<C-w><left>").await?;
    assert_eq!(app.editor.sidebar.focus, SidebarFocus::Sidebar);
    assert_eq!(app.editor.sidebar.tree_filter, "a");
    assert_eq!(app.sidebar_row_paths().len(), 1);

    helpers::send_keys(&mut app, "<esc>").await?;
    assert!(app.editor.sidebar.tree_filter.is_empty());
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn colon_from_tree_opens_the_command_line() -> anyhow::Result<()> {
    let mut app = helpers::AppBuilder::new().build()?;
    test_key_sequences(&mut app, vec![(Some("<space>b:q!<ret>"), None)], true).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn creating_an_existing_file_does_not_truncate_it() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let file = dir.path().join("keep.txt");
    std::fs::write(&file, "keep me\n")?;
    let mut app = helpers::AppBuilder::new().build()?;
    app.editor.workspace_root = dir.path().to_path_buf();

    helpers::send_keys(&mut app, "<space>b<space>a").await?;
    helpers::send_keys(&mut app, "keep.txt<ret>").await?;
    assert_eq!(std::fs::read_to_string(&file)?, "keep me\n");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn create_rename_and_delete_from_tree() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let mut app = helpers::AppBuilder::new().build()?;
    app.editor.workspace_root = dir.path().to_path_buf();

    let created = dir.path().join("new");
    let renamed = dir.path().join("renamed");
    let check_created = {
        let created = created.clone();
        move |_app: &Application| assert!(created.is_file())
    };
    let check_renamed = {
        let created = created.clone();
        let renamed = renamed.clone();
        move |_app: &Application| {
            assert!(renamed.is_file());
            assert!(!created.exists());
        }
    };
    let check_deleted = {
        let renamed = renamed.clone();
        move |_app: &Application| assert!(!renamed.exists())
    };

    test_key_sequences(
        &mut app,
        vec![
            (Some("<space>b<space>a"), None),
            (Some("new<ret>"), Some(&check_created)),
            (Some("<space>r<C-u>renamed<ret>"), Some(&check_renamed)),
            // The delete confirmation defaults to No; Tab selects Yes.
            (Some("<space>d<tab><ret>"), Some(&check_deleted)),
            (Some("<esc>:q!<ret>"), None),
        ],
        true,
    )
    .await?;
    Ok(())
}
