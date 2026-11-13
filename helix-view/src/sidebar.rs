use std::collections::HashSet;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Which pane, if any, the left sidebar is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SidebarMode {
    #[default]
    Hidden,
    Files,
    Search,
}

/// Who currently owns keyboard input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SidebarFocus {
    #[default]
    Editor,
    Sidebar,
}

/// Layout configuration for the sidebar pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", default, deny_unknown_fields)]
pub struct SidebarConfig {
    /// Preferred sidebar width in columns. Defaults to 30.
    pub width: u16,
    /// Minimum width when resolving/clamping. Defaults to 20.
    pub min_width: u16,
    /// Maximum width when resolving/clamping. Defaults to 50.
    pub max_width: u16,
    /// Whether to render Nerd Font file icons. Defaults to true.
    pub icons: bool,
    /// Whether to color tree entries by git status. Defaults to true.
    pub git_status: bool,
}

impl Default for SidebarConfig {
    fn default() -> Self {
        Self {
            width: 30,
            min_width: 20,
            max_width: 50,
            icons: true,
            git_status: true,
        }
    }
}

/// Persistent state of the search pane inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case", default, deny_unknown_fields)]
pub struct SearchInputs {
    pub query: String,
    pub replacement: String,
    pub match_case: bool,
    pub whole_word: bool,
    pub regex: bool,
    pub preserve_case: bool,
}

/// Shared sidebar state. Commands and session persistence reach this through
/// `Editor`; UI-only data (rows, scroll, text cursors) lives in `helix-term`.
#[derive(Debug, Clone)]
pub struct SidebarState {
    pub mode: SidebarMode,
    pub focus: SidebarFocus,
    pub width: u16,
    pub expanded: HashSet<PathBuf>,
    pub selected: Option<PathBuf>,
    pub search: SearchInputs,
    /// Directories that received filesystem events and need a refresh.
    pub refresh_dirs: HashSet<PathBuf>,
    /// Incremented on every filesystem event so the UI can coalesce refreshes.
    pub refresh_epoch: u64,
    /// Files in the current search result set (runtime only, not persisted).
    pub search_files: Vec<PathBuf>,
    /// Request the search pane to focus its query input on the next frame.
    pub search_focus_query: bool,
    /// Set when the focused buffer changes; the tree reveals it once.
    pub reveal_current: bool,
    /// Current file-tree filter text (runtime only).
    pub tree_filter: String,
}

impl Default for SidebarState {
    fn default() -> Self {
        Self {
            mode: SidebarMode::Hidden,
            focus: SidebarFocus::Editor,
            width: SidebarConfig::default().width,
            expanded: HashSet::new(),
            selected: None,
            search: SearchInputs::default(),
            refresh_dirs: HashSet::new(),
            refresh_epoch: 0,
            search_files: Vec::new(),
            search_focus_query: false,
            reveal_current: false,
            tree_filter: String::new(),
        }
    }
}

impl SidebarState {
    pub fn is_visible(&self) -> bool {
        self.mode != SidebarMode::Hidden
    }

    /// Resolve the sidebar width for the current terminal, leaving at least 40
    /// columns for the editor. Returns 0 when hidden or too narrow.
    pub fn visible_width(&self, area_width: u16, config: &SidebarConfig) -> u16 {
        if self.mode == SidebarMode::Hidden {
            return 0;
        }
        let min = config.min_width.min(config.max_width);
        let max = config.max_width.max(min);
        let width = self.width.clamp(min, max);
        if area_width < width.saturating_add(40) {
            0
        } else {
            width
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_and_narrow_sidebars_have_zero_width() {
        let cfg = SidebarConfig::default();
        let mut state = SidebarState::default();
        assert_eq!(state.visible_width(120, &cfg), 0);

        state.mode = SidebarMode::Files;
        assert_eq!(state.visible_width(69, &cfg), 0); // 30 + 40 editor minimum
        assert_eq!(state.visible_width(70, &cfg), 30);
    }

    #[test]
    fn width_is_clamped_to_config_bounds() {
        let cfg = SidebarConfig::default();
        let mut state = SidebarState::default();
        state.mode = SidebarMode::Files;
        state.width = 99;
        assert_eq!(state.visible_width(200, &cfg), 50);
        state.width = 1;
        assert_eq!(state.visible_width(200, &cfg), 20);
    }
}
