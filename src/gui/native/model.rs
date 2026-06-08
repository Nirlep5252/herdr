//! UI-side state model for the native chrome.
//!
//! This mirrors the slice of server state the native widgets need: the
//! workspace list, the active workspace's tabs, the active tab's panes, the
//! agent list (for the sidebar), and the active tab's layout snapshot. It is
//! rebuilt wholesale by the worker thread whenever the server reports a change,
//! which keeps the model trivially consistent with the server.

use crate::api::schema::{AgentInfo, PaneInfo, PaneLayoutSnapshot, TabInfo, WorkspaceInfo};

#[derive(Debug, Clone, Default)]
pub struct UiModel {
    pub workspaces: Vec<WorkspaceInfo>,
    pub tabs: Vec<TabInfo>,
    pub panes: Vec<PaneInfo>,
    pub agents: Vec<AgentInfo>,
    pub layout: Option<PaneLayoutSnapshot>,
    pub active_workspace_id: Option<String>,
    pub active_tab_id: Option<String>,
    pub focused_pane_id: Option<String>,
}

impl UiModel {
    /// Panes belonging to the active tab, in layout order when available.
    pub fn active_panes(&self) -> Vec<&PaneInfo> {
        let Some(tab_id) = self.active_tab_id.as_deref() else {
            return self.panes.iter().collect();
        };
        self.panes
            .iter()
            .filter(|pane| pane.tab_id == tab_id)
            .collect()
    }

    /// The terminal id attached to the currently focused pane, if any.
    pub fn focused_terminal_id(&self) -> Option<&str> {
        let pane_id = self.focused_pane_id.as_deref()?;
        self.panes
            .iter()
            .find(|pane| pane.pane_id == pane_id)
            .map(|pane| pane.terminal_id.as_str())
    }
}
