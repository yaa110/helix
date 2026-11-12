use helix_term::application::Application;
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
