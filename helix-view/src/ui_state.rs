use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::sidebar::{SidebarConfig, SidebarMode};

/// Global (not per-workspace) UI preferences that should survive restarts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiState {
    pub sidebar_mode: SidebarMode,
    pub bufferline_visible: bool,
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: u16,
    #[serde(default)]
    pub recent_workspaces: Vec<PathBuf>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            sidebar_mode: SidebarMode::Files,
            bufferline_visible: true,
            sidebar_width: SidebarConfig::default().width,
            recent_workspaces: Vec::new(),
        }
    }
}

fn path() -> PathBuf {
    helix_loader::data_dir().join("ui-state.json")
}

fn default_sidebar_width() -> u16 {
    SidebarConfig::default().width
}

/// Temporary/test workspaces should never appear in the recent-workspace
/// picker. Keep real project roots only.
fn is_ephemeral_workspace(workspace: &Path) -> bool {
    if workspace.as_os_str().is_empty() {
        return true;
    }

    let temp = std::env::temp_dir();
    let canonical_temp = std::fs::canonicalize(&temp).unwrap_or_else(|_| temp.clone());
    let canonical_workspace =
        std::fs::canonicalize(workspace).unwrap_or_else(|_| workspace.to_path_buf());

    [workspace, canonical_workspace.as_path()]
        .iter()
        .any(|path| {
            path.starts_with(&temp)
                || path.starts_with(&canonical_temp)
                || path.starts_with("/tmp")
                || path.starts_with("/private/tmp")
                || path.starts_with("/var/folders")
                || path.starts_with("/private/var/folders")
        })
}

impl UiState {
    pub fn load() -> Self {
        let mut state: UiState = std::fs::read_to_string(path())
            .ok()
            .and_then(|contents| serde_json::from_str(&contents).ok())
            .unwrap_or_default();
        state
            .recent_workspaces
            .retain(|workspace| !is_ephemeral_workspace(workspace));
        state
    }

    pub fn record_workspace(&mut self, workspace: impl Into<PathBuf>) {
        let workspace = workspace.into();
        if is_ephemeral_workspace(&workspace) {
            return;
        }
        self.recent_workspaces.retain(|path| path != &workspace);
        self.recent_workspaces.insert(0, workspace);
        self.recent_workspaces.truncate(15);
    }

    pub fn save(&self) -> std::io::Result<()> {
        let path = path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut state = self.clone();
        state
            .recent_workspaces
            .retain(|workspace| !is_ephemeral_workspace(workspace));
        let contents = serde_json::to_vec_pretty(&state)?;
        std::fs::write(path, contents)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporary_workspaces_are_not_recorded() {
        let mut state = UiState::default();
        state.record_workspace(std::env::temp_dir().join("helix-temp-workspace"));
        assert!(state.recent_workspaces.is_empty());

        let project = std::env::current_dir().unwrap().join("project");
        state.record_workspace(project.clone());
        assert_eq!(state.recent_workspaces, vec![project]);
    }
}
