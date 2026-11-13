use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::sidebar::SidebarMode;

/// Global (not per-workspace) UI preferences that should survive restarts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiState {
    pub sidebar_mode: SidebarMode,
    pub bufferline_visible: bool,
    #[serde(default)]
    pub recent_workspaces: Vec<PathBuf>,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            sidebar_mode: SidebarMode::Files,
            bufferline_visible: true,
            recent_workspaces: Vec::new(),
        }
    }
}

fn path() -> PathBuf {
    helix_loader::data_dir().join("ui-state.json")
}

impl UiState {
    pub fn load() -> Self {
        std::fs::read_to_string(path())
            .ok()
            .and_then(|contents| serde_json::from_str(&contents).ok())
            .unwrap_or_default()
    }

    pub fn record_workspace(&mut self, workspace: impl Into<PathBuf>) {
        let workspace = workspace.into();
        if workspace.as_os_str().is_empty() {
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
        let contents = serde_json::to_vec_pretty(self)?;
        std::fs::write(path, contents)
    }
}
