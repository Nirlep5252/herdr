export type AgentStatus = "idle" | "working" | "blocked" | "done" | "unknown";

export interface WorkspaceInfo {
  workspace_id: string;
  number: number;
  label: string;
  focused: boolean;
  pane_count: number;
  tab_count: number;
  active_tab_id: string;
  agent_status: AgentStatus;
}

export interface TabInfo {
  tab_id: string;
  workspace_id: string;
  number: number;
  label: string;
  focused: boolean;
  pane_count: number;
  agent_status: AgentStatus;
}

export interface PaneInfo {
  pane_id: string;
  terminal_id: string;
  workspace_id: string;
  tab_id: string;
  focused: boolean;
  cwd?: string;
  foreground_cwd?: string;
  label?: string;
  agent?: string;
  title?: string;
  display_agent?: string;
  agent_status: AgentStatus;
  custom_status?: string;
  revision: number;
}

export interface AgentInfo {
  terminal_id: string;
  name?: string;
  agent?: string;
  title?: string;
  display_agent?: string;
  agent_status: AgentStatus;
  custom_status?: string;
  workspace_id: string;
  tab_id: string;
  pane_id: string;
  focused: boolean;
  cwd?: string;
  foreground_cwd?: string;
  revision: number;
}

export interface PaneLayoutRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface PaneLayoutPane {
  pane_id: string;
  focused: boolean;
  rect: PaneLayoutRect;
}

export interface PaneLayoutSplit {
  id: string;
  direction: "right" | "down";
  ratio: number;
  rect: PaneLayoutRect;
}

export interface PaneLayoutSnapshot {
  workspace_id: string;
  tab_id: string;
  zoomed: boolean;
  area: PaneLayoutRect;
  focused_pane_id: string;
  panes: PaneLayoutPane[];
  splits: PaneLayoutSplit[];
}

export interface DesktopSnapshot {
  active_workspace_id?: string;
  active_tab_id?: string;
  focused_pane_id?: string;
  workspaces: WorkspaceInfo[];
  tabs: TabInfo[];
  panes: PaneInfo[];
  agents: AgentInfo[];
  active_layout?: PaneLayoutSnapshot;
}

export interface PaneFrame {
  pane_id: string;
  ansi: string;
  revision?: number;
}

export type SplitDirection = "right" | "down";

export type DesktopAction =
  | { type: "workspace.focus"; workspaceId: string }
  | { type: "tab.focus"; tabId: string }
  | { type: "agent.focus"; target: string }
  | { type: "pane.split"; paneId: string; direction: SplitDirection }
  | { type: "pane.zoom"; paneId: string }
  | { type: "pane.close"; paneId: string }
  | { type: "pane.send"; paneId: string; text: string };

export interface HerdrStatus {
  connected: boolean;
  mode: "socket" | "demo";
  message?: string;
  socketPath?: string;
}

export interface HerdrDesktopApi {
  status(): Promise<HerdrStatus>;
  snapshot(): Promise<DesktopSnapshot>;
  paneFrame(paneId: string): Promise<PaneFrame>;
  action(action: DesktopAction): Promise<void>;
}
