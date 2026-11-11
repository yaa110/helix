use helix_event::register_hook;
use helix_view::events::{DocumentDidClose, DocumentDidOpen, DocumentFocusLost};
use helix_view::handlers::Handlers;

pub(super) fn register_hooks(_handlers: &Handlers) {
    register_hook!(move |event: &mut DocumentDidOpen<'_>| {
        event.editor.session_dirty = true;
        Ok(())
    });
    register_hook!(move |event: &mut DocumentDidClose<'_>| {
        event.editor.session_dirty = true;
        Ok(())
    });
    register_hook!(move |event: &mut DocumentFocusLost<'_>| {
        event.editor.session_dirty = true;
        Ok(())
    });
}
