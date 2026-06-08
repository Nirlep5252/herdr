use crate::api::schema::{DesktopSnapshot, ResponseResult};
use crate::app::App;

use super::responses::encode_success;

impl App {
    pub(super) fn handle_desktop_snapshot(&mut self, id: String) -> String {
        encode_success(
            id,
            ResponseResult::DesktopSnapshot {
                snapshot: self.desktop_snapshot(),
            },
        )
    }

    fn desktop_snapshot(&self) -> DesktopSnapshot {
        let active_workspace_id = self
            .state
            .active
            .map(|ws_idx| self.public_workspace_id(ws_idx));
        let active_tab_id = self.state.active.and_then(|ws_idx| {
            let tab_idx = self.state.workspaces.get(ws_idx)?.active_tab;
            self.public_tab_id(ws_idx, tab_idx)
        });
        let focused_pane_id = self.state.active.and_then(|ws_idx| {
            let pane_id = self.state.workspaces.get(ws_idx)?.focused_pane_id()?;
            self.public_pane_id(ws_idx, pane_id)
        });
        let active_layout = self.state.active.and_then(|ws_idx| {
            let tab_idx = self.state.workspaces.get(ws_idx)?.active_tab;
            self.pane_layout_snapshot(ws_idx, tab_idx)
        });

        DesktopSnapshot {
            active_workspace_id,
            active_tab_id,
            focused_pane_id,
            workspaces: self
                .state
                .workspaces
                .iter()
                .enumerate()
                .map(|(ws_idx, _)| self.workspace_info(ws_idx))
                .collect(),
            tabs: self.collect_all_tabs(),
            panes: self.collect_all_panes(),
            agents: self.collect_agent_infos(),
            active_layout,
        }
    }

    fn collect_all_tabs(&self) -> Vec<crate::api::schema::TabInfo> {
        self.state
            .workspaces
            .iter()
            .enumerate()
            .flat_map(|(ws_idx, ws)| {
                (0..ws.tabs.len()).filter_map(move |tab_idx| self.tab_info(ws_idx, tab_idx))
            })
            .collect()
    }

    fn collect_all_panes(&self) -> Vec<crate::api::schema::PaneInfo> {
        self.state
            .workspaces
            .iter()
            .enumerate()
            .flat_map(|(ws_idx, ws)| {
                ws.tabs.iter().flat_map(move |tab| {
                    tab.layout
                        .pane_ids()
                        .into_iter()
                        .filter_map(move |pane_id| self.pane_info(ws_idx, pane_id))
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use crate::api::schema::{Method, Request, ResponseResult, SuccessResponse};
    use crate::app::App;
    use crate::config::Config;
    use crate::detect::{Agent, AgentState};
    use crate::terminal::TerminalState;
    use crate::workspace::Workspace;

    fn app_with_workspace() -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            true,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = vec![Workspace::test_new("api")];
        app.state.active = Some(0);
        app.state.view.terminal_area = ratatui::layout::Rect::new(0, 0, 120, 40);

        let root_pane = app.state.workspaces[0].tabs[0].root_pane;
        let terminal_id = app.state.workspaces[0].tabs[0]
            .terminal_id(root_pane)
            .expect("test workspace should have a root terminal")
            .clone();
        let mut terminal = TerminalState::new(terminal_id.clone(), "/workspace/api".into());
        terminal.set_detected_state(Some(Agent::Codex), AgentState::Blocked);
        app.state.terminals.insert(terminal_id, terminal);
        app
    }

    #[test]
    fn desktop_snapshot_returns_single_window_navigation_model() {
        let mut app = app_with_workspace();

        let response = app.handle_api_request_after_internal_events_drained(Request {
            id: "desktop".into(),
            method: Method::DesktopSnapshot(Default::default()),
        });

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        let ResponseResult::DesktopSnapshot { snapshot } = success.result else {
            panic!("expected desktop snapshot response");
        };

        assert_eq!(snapshot.workspaces.len(), 1);
        assert_eq!(snapshot.tabs.len(), 1);
        assert_eq!(snapshot.panes.len(), 1);
        assert_eq!(snapshot.agents.len(), 1);
        let workspace_id = snapshot.workspaces[0].workspace_id.as_str();
        assert_eq!(snapshot.active_workspace_id.as_deref(), Some(workspace_id));
        assert_eq!(
            snapshot.active_tab_id.as_deref(),
            Some(snapshot.tabs[0].tab_id.as_str())
        );
        assert_eq!(
            snapshot.focused_pane_id.as_deref(),
            Some(snapshot.panes[0].pane_id.as_str())
        );
        assert_eq!(snapshot.agents[0].agent.as_deref(), Some("codex"));
        assert_eq!(
            snapshot
                .active_layout
                .as_ref()
                .map(|layout| layout.panes.len()),
            Some(1)
        );
    }
}
