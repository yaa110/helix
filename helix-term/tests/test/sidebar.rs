use helix_term::application::Application;
use helix_term::file_watcher;
use helix_view::doc;
use helix_view::session::SessionStore;
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
async fn diagnostics_report_opens_in_a_buffer() -> anyhow::Result<()> {
    let mut app = helpers::AppBuilder::new().build()?;
    helpers::send_keys(&mut app, "<space>i").await?;
    let report = doc!(app.editor).text().to_string();
    assert!(report.contains("Helix diagnostics"));
    assert!(report.contains("Session"));
    assert!(report.contains("Buffers"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn open_folder_command_and_alias_open_the_picker() -> anyhow::Result<()> {
    let mut app = helpers::AppBuilder::new().build()?;
    assert_eq!(app.compositor_layer_count(), 1);

    helpers::send_keys(&mut app, ":open-folder<ret>").await?;
    assert_eq!(app.compositor_layer_count(), 2);
    helpers::send_keys(&mut app, "<esc>").await?;
    assert_eq!(app.compositor_layer_count(), 1);

    helpers::send_keys(&mut app, ":folder<ret>").await?;
    assert_eq!(app.compositor_layer_count(), 2);
    helpers::send_keys(&mut app, "<esc>").await?;
    assert_eq!(app.compositor_layer_count(), 1);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn workspace_switch_refuses_to_discard_modified_buffers() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let current = root.path().join("current");
    let target_parent = root.path().join("target");
    let target = target_parent.join("child");
    std::fs::create_dir_all(&target)?;
    std::fs::create_dir_all(&current)?;
    let store = SessionStore::new(root.path().join("state"));

    let mut app = helpers::AppBuilder::new()
        .with_session_store(store)
        .with_workspace_dir(&current)
        .build()?;
    helpers::send_keys(&mut app, "iX<esc>").await?;
    assert!(doc!(app.editor).is_modified());

    helpers::send_keys(&mut app, ":open-folder<ret>").await?;
    let path = target_parent.display().to_string();
    helpers::send_keys(&mut app, &format!("p<A-backspace>{path}<ret>")).await?;
    helpers::send_keys(&mut app, "<ret>").await?;

    assert_eq!(
        std::fs::canonicalize(&app.editor.workspace_root)?,
        std::fs::canonicalize(&current)?
    );
    assert!(doc!(app.editor).is_modified());
    assert!(app
        .editor
        .get_status()
        .is_some_and(|(status, _)| status.contains("modified buffers")));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn folder_picker_opens_the_selected_folder_as_a_workspace() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let home = root.path().join("home");
    let child = home.join("child");
    std::fs::create_dir_all(&child)?;
    let store = SessionStore::new(root.path().join("state"));

    let mut app = helpers::AppBuilder::new()
        .with_session_store(store)
        .with_workspace_dir(root.path())
        .build()?;
    helpers::send_keys(&mut app, ":open-folder<ret>").await?;
    let path = home.display().to_string();
    helpers::send_keys(&mut app, &format!("p<A-backspace>{path}<ret>")).await?;
    helpers::send_keys(&mut app, "<ret>").await?;

    assert_eq!(
        std::fs::canonicalize(&app.editor.workspace_root)?,
        std::fs::canonicalize(&child)?
    );
    assert_eq!(app.compositor_layer_count(), 1);
    assert!(app
        .editor
        .get_status()
        .is_some_and(|(status, _)| status.starts_with("Opened ")));
    Ok(())
}

#[cfg(not(windows))]
#[tokio::test(flavor = "multi_thread")]
async fn navigation_wheel_scrolls_code() -> anyhow::Result<()> {
    use termina::event::{Modifiers, MouseEvent, MouseEventKind};

    let text = format!(
        "#[|]#{}",
        (0..200)
            .map(|line| format!("line {line}\n"))
            .collect::<String>()
    );
    let mut app = helpers::AppBuilder::new().with_input_text(text).build()?;
    let before_offset = {
        let (view, doc) = helix_view::current_ref!(app.editor);
        doc.view_offset(view.id)
    };

    helpers::send_mouse(
        &mut app,
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 60,
            row: 10,
            modifiers: Modifiers::NONE,
        },
    )
    .await?;

    let after_offset = {
        let (view, doc) = helix_view::current_ref!(app.editor);
        doc.view_offset(view.id)
    };
    assert_ne!(before_offset, after_offset);
    Ok(())
}

#[cfg(not(windows))]
#[tokio::test(flavor = "multi_thread")]
async fn native_scroll_wheel_scrolls_the_tree_viewport() -> anyhow::Result<()> {
    use termina::event::{Modifiers, MouseEvent, MouseEventKind};

    let root = tempfile::tempdir()?;
    for index in 0..30 {
        std::fs::create_dir(root.path().join(format!("dir{index:02}")))?;
    }
    let mut app = helpers::AppBuilder::new()
        .with_workspace_dir(root.path())
        .build()?;
    helpers::run_event_loop_until_idle(&mut app).await;

    helpers::send_mouse(
        &mut app,
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 5,
            row: 5,
            modifiers: Modifiers::NONE,
        },
    )
    .await?;

    assert!(app.editor.sidebar.selected.is_some());
    Ok(())
}

#[cfg(not(windows))]
#[tokio::test(flavor = "multi_thread")]
async fn mouse_disable_ignores_wheel_and_clicks() -> anyhow::Result<()> {
    use termina::event::{Modifiers, MouseButton, MouseEvent, MouseEventKind};

    let text = format!(
        "#[|]#{}",
        (0..50)
            .map(|line| format!("line {line}\n"))
            .collect::<String>()
    );
    let mut config = helpers::test_config();
    config.editor.mouse = helix_view::editor::MouseConfig::Disable;
    let mut app = helpers::AppBuilder::new()
        .with_config(config)
        .with_input_text(text)
        .build()?;
    let before_offset = {
        let (view, doc) = helix_view::current_ref!(app.editor);
        doc.view_offset(view.id)
    };
    let before_selection = {
        let (view, doc) = helix_view::current_ref!(app.editor);
        doc.selection(view.id).clone()
    };

    for kind in [
        MouseEventKind::ScrollDown,
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        helpers::send_mouse(
            &mut app,
            MouseEvent {
                kind,
                column: 60,
                row: 10,
                modifiers: Modifiers::NONE,
            },
        )
        .await?;
    }

    let after_offset = {
        let (view, doc) = helix_view::current_ref!(app.editor);
        doc.view_offset(view.id)
    };
    let after_selection = {
        let (view, doc) = helix_view::current_ref!(app.editor);
        doc.selection(view.id).clone()
    };
    assert_eq!(before_offset, after_offset);
    assert_eq!(before_selection, after_selection);
    Ok(())
}

#[cfg(not(windows))]
#[tokio::test(flavor = "multi_thread")]
async fn navigation_click_places_the_cursor_without_a_selection_range() -> anyhow::Result<()> {
    use termina::event::{Modifiers, MouseButton, MouseEvent, MouseEventKind};

    let text = format!(
        "#[|]#{}",
        (0..50)
            .map(|line| format!("line {line}\n"))
            .collect::<String>()
    );
    let mut app = helpers::AppBuilder::new().with_input_text(text).build()?;
    let before = {
        let (view, doc) = helix_view::current_ref!(app.editor);
        doc.selection(view.id)
            .primary()
            .cursor_line(doc.text().slice(..))
    };

    helpers::send_mouse(
        &mut app,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 60,
            row: 10,
            modifiers: Modifiers::NONE,
        },
    )
    .await?;

    let (view, doc) = helix_view::current_ref!(app.editor);
    let primary = doc.selection(view.id).primary();
    let after = primary.cursor_line(doc.text().slice(..));
    assert!(after > before);
    assert_eq!(primary.to() - primary.from(), 1);
    Ok(())
}

#[cfg(not(windows))]
#[tokio::test(flavor = "multi_thread")]
async fn navigation_click_outside_tree_returns_focus_to_editor() -> anyhow::Result<()> {
    use termina::event::{Modifiers, MouseButton, MouseEvent, MouseEventKind};

    let root = tempfile::tempdir()?;
    let text = format!(
        "#[|]#{}",
        (0..20)
            .map(|line| format!("line {line}\n"))
            .collect::<String>()
    );
    let mut app = helpers::AppBuilder::new()
        .with_workspace_dir(root.path())
        .with_input_text(text)
        .build()?;
    app.editor.sidebar.mode = SidebarMode::Files;
    app.editor.sidebar.focus = SidebarFocus::Sidebar;
    app.editor.needs_redraw = true;
    app.handle_idle_timeout().await;

    helpers::send_mouse(
        &mut app,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 60,
            row: 5,
            modifiers: Modifiers::NONE,
        },
    )
    .await?;

    assert_eq!(app.editor.sidebar.focus, SidebarFocus::Editor);
    Ok(())
}

#[cfg(not(windows))]
#[tokio::test(flavor = "multi_thread")]
async fn navigation_click_empty_editor_space_clears_tree_focus() -> anyhow::Result<()> {
    use termina::event::{Modifiers, MouseButton, MouseEvent, MouseEventKind};

    let root = tempfile::tempdir()?;
    let text = format!(
        "#[|]#{}",
        (0..5)
            .map(|line| format!("line {line}\n"))
            .collect::<String>()
    );
    let mut app = helpers::AppBuilder::new()
        .with_workspace_dir(root.path())
        .with_input_text(text)
        .build()?;
    app.editor.sidebar.mode = SidebarMode::Files;
    app.editor.sidebar.focus = SidebarFocus::Sidebar;
    app.editor.needs_redraw = true;
    app.handle_idle_timeout().await;

    helpers::send_mouse(
        &mut app,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 60,
            row: 120,
            modifiers: Modifiers::NONE,
        },
    )
    .await?;

    assert_eq!(app.editor.sidebar.focus, SidebarFocus::Editor);
    Ok(())
}

#[cfg(not(windows))]
#[tokio::test(flavor = "multi_thread")]
async fn navigation_click_focuses_another_split() -> anyhow::Result<()> {
    use termina::event::{Modifiers, MouseButton, MouseEvent, MouseEventKind};

    let dir = tempfile::tempdir()?;
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    std::fs::write(&a, "a\n".repeat(20))?;
    std::fs::write(&b, "b\n".repeat(20))?;

    let mut app = helpers::AppBuilder::new().build()?;
    app.editor.workspace_root = dir.path().to_path_buf();
    app.editor.open(&a, helix_view::editor::Action::Replace)?;
    app.editor
        .open(&b, helix_view::editor::Action::VerticalSplit)?;
    app.editor.needs_redraw = true;
    app.handle_idle_timeout().await;

    let focused = app.editor.tree.focus;
    let target = app
        .editor
        .tree
        .views()
        .find(|(view, _)| view.id != focused)
        .map(|(view, _)| (view.id, view.area))
        .expect("expected a second split");

    helpers::send_mouse(
        &mut app,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: target.1.x + 30,
            row: target.1.y + 5,
            modifiers: Modifiers::NONE,
        },
    )
    .await?;

    assert_eq!(app.editor.tree.focus, target.0);
    Ok(())
}

#[cfg(not(windows))]
#[tokio::test(flavor = "multi_thread")]
async fn navigation_click_focuses_and_selects_a_tree_row() -> anyhow::Result<()> {
    use termina::event::{Modifiers, MouseButton, MouseEvent, MouseEventKind};

    let root = tempfile::tempdir()?;
    for index in 0..8 {
        std::fs::write(root.path().join(format!("file{index}.txt")), "x\n")?;
    }
    let mut app = helpers::AppBuilder::new()
        .with_workspace_dir(root.path())
        .build()?;
    app.editor.sidebar.mode = SidebarMode::Files;
    app.editor.sidebar.focus = SidebarFocus::Editor;
    app.editor.sidebar.selected = None;
    app.editor.needs_redraw = true;
    app.handle_idle_timeout().await;

    helpers::send_mouse(
        &mut app,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 5,
            modifiers: Modifiers::NONE,
        },
    )
    .await?;

    assert_eq!(app.editor.sidebar.focus, SidebarFocus::Sidebar);
    assert!(app.editor.sidebar.selected.is_some());
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_workspace_has_a_branch_fallback() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("a.txt"), "a\n")?;
    let status = std::process::Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(dir.path())
        .status()?;
    assert!(status.success());
    let status = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-q",
            "-m",
            "init",
            "--allow-empty",
        ])
        .current_dir(dir.path())
        .status()?;
    assert!(status.success());

    let app = helpers::AppBuilder::new()
        .with_workspace_dir(dir.path())
        .build()?;
    assert_eq!(app.editor.workspace_branch.as_deref(), Some("main"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn tree_search_cursor_supports_arrows_and_modifiers() -> anyhow::Result<()> {
    let mut app = helpers::AppBuilder::new().build()?;
    helpers::send_keys(&mut app, "<space>b/foo bar").await?;

    helpers::send_keys(&mut app, "<left><left><left>X").await?;
    assert_eq!(app.editor.sidebar.tree_filter, "foo Xbar");

    helpers::send_keys(&mut app, "<C-left>Y").await?;
    assert_eq!(app.editor.sidebar.tree_filter, "foo YXbar");

    helpers::send_keys(&mut app, "<C-backspace>").await?;
    assert_eq!(app.editor.sidebar.tree_filter, "foo Xbar");
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn tree_batch_delete_and_undo() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    let c = dir.path().join("c.txt");
    for file in [&a, &b, &c] {
        std::fs::write(file, "data\n")?;
    }
    let mut app = helpers::AppBuilder::new().build()?;
    app.editor.workspace_root = dir.path().to_path_buf();

    helpers::send_keys(&mut app, "<space>b").await?;
    helpers::send_keys(&mut app, "<space>t<space>t").await?;
    helpers::send_keys(&mut app, "<space>d<tab><ret>").await?;

    assert!(!a.exists());
    assert!(!b.exists());
    assert!(c.exists());

    helpers::send_keys(&mut app, "<space>u").await?;
    assert!(a.exists());
    assert!(b.exists());
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn tree_can_toggle_ignored_files() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join(".gitignore"), "ignored.txt\n")?;
    std::fs::write(dir.path().join("ignored.txt"), "hidden\n")?;
    std::fs::write(dir.path().join("visible.txt"), "shown\n")?;
    let mut app = helpers::AppBuilder::new().build()?;
    app.editor.workspace_root = dir.path().to_path_buf();

    helpers::send_keys(&mut app, "<space>b").await?;
    assert!(app
        .sidebar_row_paths()
        .iter()
        .any(|path| path.ends_with("ignored.txt")));

    helpers::send_keys(&mut app, "<space>I").await?;
    assert!(!app
        .sidebar_row_paths()
        .iter()
        .any(|path| path.ends_with("ignored.txt")));

    helpers::send_keys(&mut app, "<space>I").await?;
    assert!(app
        .sidebar_row_paths()
        .iter()
        .any(|path| path.ends_with("ignored.txt")));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn ctrl_up_down_scrolls_the_tree_while_focused() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    for name in ["a", "b", "c", "d", "e", "f", "g", "h"] {
        std::fs::write(dir.path().join(format!("{name}.txt")), format!("{name}\n"))?;
    }
    let mut app = helpers::AppBuilder::new().build()?;
    app.editor.workspace_root = dir.path().to_path_buf();

    helpers::send_keys(&mut app, "<space>b").await?;
    let rows = app.sidebar_row_paths();
    let lines = app.editor.config().scroll_lines.unsigned_abs().max(1) as usize;
    assert!(rows.len() > lines);

    helpers::send_keys(&mut app, "<C-down>").await?;
    assert_eq!(
        app.editor.sidebar.selected.as_deref(),
        Some(rows[lines].as_path())
    );

    helpers::send_keys(&mut app, "<C-up>").await?;
    assert_eq!(
        app.editor.sidebar.selected.as_deref(),
        Some(rows[0].as_path())
    );
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
async fn navigating_buffers_reveals_each_file_in_the_tree() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    std::fs::write(&a, "a\n")?;
    std::fs::write(&b, "b\n")?;

    let mut app = helpers::AppBuilder::new().build()?;
    app.editor.workspace_root = dir.path().to_path_buf();
    app.editor.sidebar.mode = SidebarMode::Files;
    app.editor.sidebar.focus = SidebarFocus::Editor;
    app.editor.open(&a, helix_view::editor::Action::Replace)?;
    app.editor.open(&b, helix_view::editor::Action::Load)?;
    app.editor.needs_redraw = true;
    app.handle_idle_timeout().await;
    assert_eq!(app.editor.sidebar.selected.as_deref(), Some(a.as_path()));

    app.editor.sidebar.tree_filter = "zzz".to_string();
    app.editor.sidebar.tree_search_active = true;
    helpers::send_keys(&mut app, "gn").await?;
    assert_eq!(app.editor.sidebar.selected.as_deref(), Some(b.as_path()));
    assert!(app.editor.sidebar.tree_filter.is_empty());

    helpers::send_keys(&mut app, "gp").await?;
    assert_eq!(app.editor.sidebar.selected.as_deref(), Some(a.as_path()));

    app.editor.new_file(helix_view::editor::Action::Replace);
    app.editor.needs_redraw = true;
    app.handle_idle_timeout().await;
    assert_eq!(app.editor.sidebar.selected.as_deref(), Some(a.as_path()));
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
async fn tree_enter_finishes_search_and_slash_resumes_it() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("a.txt"), "a\n")?;
    std::fs::write(dir.path().join("ab.txt"), "ab\n")?;
    let mut app = helpers::AppBuilder::new().build()?;
    app.editor.workspace_root = dir.path().to_path_buf();

    helpers::send_keys(&mut app, "<space>b/a").await?;
    assert!(app.editor.sidebar.tree_search_active);
    assert_eq!(app.sidebar_row_paths().len(), 2);

    // Enter leaves the input but keeps the filtered rows.
    helpers::send_keys(&mut app, "<ret>").await?;
    assert!(!app.editor.sidebar.tree_search_active);
    assert_eq!(app.editor.sidebar.tree_filter, "a");
    assert_eq!(app.sidebar_row_paths().len(), 2);

    // Plain typing is ignored, while Space actions still work.
    helpers::send_keys(&mut app, "z").await?;
    assert_eq!(app.editor.sidebar.tree_filter, "a");
    helpers::send_keys(&mut app, "<space>").await?;
    assert!(app.editor.autoinfo.is_some());
    helpers::send_keys(&mut app, "<esc>").await?;

    // `/` resumes editing the existing query.
    helpers::send_keys(&mut app, "/b").await?;
    assert_eq!(app.editor.sidebar.tree_filter, "ab");
    assert!(app.editor.sidebar.tree_search_active);

    // Cmd/Alt+Delete clears while staying in edit mode.
    helpers::send_keys(&mut app, "<A-backspace>").await?;
    assert_eq!(app.editor.sidebar.tree_filter, "");
    assert!(app.editor.sidebar.tree_search_active);
    helpers::send_keys(&mut app, "a").await?;
    assert_eq!(app.editor.sidebar.tree_filter, "a");
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
