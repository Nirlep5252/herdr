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
    /// Panes in the focused workspace (all tabs).
    pub panes: Vec<PaneInfo>,
    /// Panes across every workspace, used to keep per-pane attach streams alive
    /// while another workspace is focused.
    pub all_panes: Vec<PaneInfo>,
    pub agents: Vec<AgentInfo>,
    pub layout: Option<PaneLayoutSnapshot>,
    pub active_workspace_id: Option<String>,
    pub active_tab_id: Option<String>,
    pub focused_pane_id: Option<String>,
}

impl UiModel {
    /// Terminal ids whose attach streams should stay open across workspace focus changes.
    pub fn retained_terminal_ids(&self) -> std::collections::HashSet<String> {
        self.all_panes
            .iter()
            .map(|pane| pane.terminal_id.clone())
            .filter(|terminal_id| !terminal_id.is_empty())
            .collect()
    }

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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::api::schema::AgentStatus;

    fn pane(id: &str, terminal_id: &str, tab_id: &str) -> PaneInfo {
        PaneInfo {
            pane_id: id.into(),
            terminal_id: terminal_id.into(),
            workspace_id: "ws0".into(),
            tab_id: tab_id.into(),
            focused: false,
            cwd: None,
            foreground_cwd: None,
            label: None,
            agent: None,
            title: None,
            display_agent: None,
            agent_status: AgentStatus::Unknown,
            custom_status: None,
            state_labels: HashMap::new(),
            agent_session: None,
            revision: 0,
        }
    }

    #[test]
    fn retained_terminal_ids_skip_empty_terminal_ids() {
        let model = UiModel {
            all_panes: vec![
                pane("ws0:tab0:pane0", "term-a", "ws0:tab0"),
                PaneInfo {
                    terminal_id: String::new(),
                    ..pane("ws0:tab0:pane1", "", "ws0:tab0")
                },
            ],
            ..UiModel::default()
        };

        let retained = model.retained_terminal_ids();
        assert_eq!(retained.len(), 1);
        assert!(retained.contains("term-a"));
    }

    #[test]
    fn retained_terminal_ids_include_every_workspace() {
        let model = UiModel {
            panes: vec![pane("ws0:tab0:pane0", "term-a", "ws0:tab0")],
            all_panes: vec![
                pane("ws0:tab0:pane0", "term-a", "ws0:tab0"),
                pane("ws1:tab0:pane0", "term-b", "ws1:tab0"),
            ],
            ..UiModel::default()
        };

        let retained = model.retained_terminal_ids();
        assert_eq!(retained.len(), 2);
        assert!(retained.contains("term-a"));
        assert!(retained.contains("term-b"));
    }
}
