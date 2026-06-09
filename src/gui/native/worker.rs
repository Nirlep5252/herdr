//! Background API worker: owns the JSON API client, fetches the UI model, and
//! executes control commands.
//!
//! The worker rebuilds the whole [`UiModel`] on a short poll interval and
//! immediately after each control command, then pushes it to the winit loop.
//! Full refetch keeps the model consistent with the server without fragile
//! incremental event handling; it is cheap over the local socket.

use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use winit::event_loop::EventLoopProxy;

use crate::api::client::ApiClient;
use crate::api::schema::{
    AgentTarget, EmptyParams, Method, PaneLayoutParams, PaneListParams, PaneSplitParams,
    PaneTarget, Request, ResponseResult, SplitDirection, TabCreateParams, TabListParams, TabTarget,
    WorkspaceCreateParams, WorkspaceTarget,
};

use super::model::UiModel;
use super::UserEvent;

const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// Control commands issued by the UI thread.
#[derive(Debug, Clone)]
pub enum Cmd {
    FocusWorkspace(String),
    FocusTab(String),
    /// Focus a pane/terminal by its `terminal_id` (or any agent target string).
    FocusTarget(String),
    NewTab,
    NewWorkspace,
    SplitPane {
        pane_id: String,
        dir: SplitDirection,
    },
    ClosePane(String),
    CloseTab(String),
    Shutdown,
}

/// Spawns the worker thread and returns a command sender.
pub fn spawn(proxy: EventLoopProxy<UserEvent>) -> Sender<Cmd> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("herdr-gui-worker".to_string())
        .spawn(move || run(rx, proxy))
        .expect("spawn gui worker thread");
    tx
}

fn run(rx: Receiver<Cmd>, proxy: EventLoopProxy<UserEvent>) {
    let api = ApiClient::local();
    push_model(&api, &proxy);

    loop {
        match rx.recv_timeout(POLL_INTERVAL) {
            Ok(Cmd::Shutdown) => break,
            Ok(cmd) => {
                let mut cmds = vec![cmd];
                while let Ok(next) = rx.try_recv() {
                    if matches!(next, Cmd::Shutdown) {
                        return;
                    }
                    cmds.push(next);
                }
                for cmd in cmds {
                    apply_command(&api, cmd);
                }
                push_model(&api, &proxy);
            }
            Err(RecvTimeoutError::Timeout) => push_model(&api, &proxy),
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn apply_command(api: &ApiClient, cmd: Cmd) {
    let request = match cmd {
        Cmd::FocusWorkspace(workspace_id) => Some(Request {
            id: "gui:workspace.focus".into(),
            method: Method::WorkspaceFocus(WorkspaceTarget { workspace_id }),
        }),
        Cmd::FocusTab(tab_id) => Some(Request {
            id: "gui:tab.focus".into(),
            method: Method::TabFocus(TabTarget { tab_id }),
        }),
        Cmd::FocusTarget(target) => Some(Request {
            id: "gui:agent.focus".into(),
            method: Method::AgentFocus(AgentTarget { target }),
        }),
        Cmd::NewTab => Some(Request {
            id: "gui:tab.create".into(),
            method: Method::TabCreate(TabCreateParams {
                workspace_id: None,
                cwd: None,
                focus: true,
                label: None,
            }),
        }),
        Cmd::NewWorkspace => Some(Request {
            id: "gui:workspace.create".into(),
            method: Method::WorkspaceCreate(WorkspaceCreateParams {
                cwd: None,
                focus: true,
                label: None,
            }),
        }),
        Cmd::SplitPane { pane_id, dir } => Some(Request {
            id: "gui:pane.split".into(),
            method: Method::PaneSplit(PaneSplitParams {
                workspace_id: None,
                target_pane_id: Some(pane_id),
                direction: dir,
                ratio: None,
                cwd: None,
                focus: true,
            }),
        }),
        Cmd::ClosePane(pane_id) => Some(Request {
            id: "gui:pane.close".into(),
            method: Method::PaneClose(PaneTarget { pane_id }),
        }),
        Cmd::CloseTab(tab_id) => Some(Request {
            id: "gui:tab.close".into(),
            method: Method::TabClose(TabTarget { tab_id }),
        }),
        Cmd::Shutdown => None,
    };

    if let Some(request) = request {
        if let Err(err) = api.request(request) {
            tracing::warn!(error = %err, "gui control command failed");
        }
    }
}

fn push_model(api: &ApiClient, proxy: &EventLoopProxy<UserEvent>) {
    match fetch_model(api) {
        Ok(model) => {
            proxy.send_event(UserEvent::Model(Box::new(model))).ok();
        }
        Err(err) => {
            proxy.send_event(UserEvent::ApiError(err.to_string())).ok();
        }
    }
}

fn fetch_model(api: &ApiClient) -> Result<UiModel, crate::api::client::ApiClientError> {
    let workspaces = match call(
        api,
        "gui:workspace.list",
        Method::WorkspaceList(EmptyParams {}),
    )? {
        ResponseResult::WorkspaceList { workspaces } => workspaces,
        _ => Vec::new(),
    };

    let active_ws = workspaces
        .iter()
        .find(|ws| ws.focused)
        .or_else(|| workspaces.first());
    let active_workspace_id = active_ws.map(|ws| ws.workspace_id.clone());

    let tabs = match &active_workspace_id {
        Some(id) => match call(
            api,
            "gui:tab.list",
            Method::TabList(TabListParams {
                workspace_id: Some(id.clone()),
            }),
        )? {
            ResponseResult::TabList { tabs } => tabs,
            _ => Vec::new(),
        },
        None => Vec::new(),
    };

    let active_tab_id = tabs
        .iter()
        .find(|tab| tab.focused)
        .map(|tab| tab.tab_id.clone())
        .or_else(|| active_ws.map(|ws| ws.active_tab_id.clone()));

    let panes = match &active_workspace_id {
        Some(id) => match call(
            api,
            "gui:pane.list",
            Method::PaneList(PaneListParams {
                workspace_id: Some(id.clone()),
            }),
        )? {
            ResponseResult::PaneList { panes } => panes,
            _ => Vec::new(),
        },
        None => Vec::new(),
    };

    let all_panes = match call(
        api,
        "gui:pane.list.all",
        Method::PaneList(PaneListParams { workspace_id: None }),
    ) {
        Ok(ResponseResult::PaneList { panes }) => panes,
        _ => panes.clone(),
    };

    let focused_pane_id = panes
        .iter()
        .find(|pane| pane.focused && Some(&pane.tab_id) == active_tab_id.as_ref())
        .or_else(|| panes.iter().find(|pane| pane.focused))
        .map(|pane| pane.pane_id.clone());

    let layout = match call(
        api,
        "gui:pane.layout",
        Method::PaneLayout(PaneLayoutParams {
            pane_id: focused_pane_id.clone(),
        }),
    ) {
        Ok(ResponseResult::PaneLayout { layout }) => Some(layout),
        _ => None,
    };

    let agents = match call(api, "gui:agent.list", Method::AgentList(EmptyParams {})) {
        Ok(ResponseResult::AgentList { agents }) => agents,
        _ => Vec::new(),
    };

    Ok(UiModel {
        workspaces,
        tabs,
        panes,
        all_panes,
        agents,
        layout,
        active_workspace_id,
        active_tab_id,
        focused_pane_id,
    })
}

fn call(
    api: &ApiClient,
    id: &str,
    method: Method,
) -> Result<ResponseResult, crate::api::client::ApiClientError> {
    api.request(Request {
        id: id.to_string(),
        method,
    })
    .map(|response| response.result)
}
