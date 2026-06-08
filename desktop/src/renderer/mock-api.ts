import type {
  DesktopAction,
  DesktopSnapshot,
  HerdrDesktopApi,
  HerdrStatus,
  PaneFrame,
} from "../shared/herdr";

export function createMockApi(): HerdrDesktopApi {
  return {
    async status(): Promise<HerdrStatus> {
      return { connected: true, mode: "demo", message: "Browser demo mode" };
    },
    async snapshot(): Promise<DesktopSnapshot> {
      return snapshot;
    },
    async paneFrame(paneId: string): Promise<PaneFrame> {
      return frameFor(paneId);
    },
    async action(_action: DesktopAction): Promise<void> {
      return;
    },
  };
}

const snapshot: DesktopSnapshot = {
  active_workspace_id: "demo-api",
  active_tab_id: "demo-api:1",
  focused_pane_id: "demo-api-1",
  workspaces: [
    {
      workspace_id: "demo-api",
      number: 1,
      label: "api",
      focused: true,
      pane_count: 3,
      tab_count: 2,
      active_tab_id: "demo-api:1",
      agent_status: "blocked",
    },
    {
      workspace_id: "demo-web",
      number: 2,
      label: "web",
      focused: false,
      pane_count: 2,
      tab_count: 1,
      active_tab_id: "demo-web:1",
      agent_status: "working",
    },
  ],
  tabs: [
    {
      tab_id: "demo-api:1",
      workspace_id: "demo-api",
      number: 1,
      label: "agents",
      focused: true,
      pane_count: 3,
      agent_status: "blocked",
    },
    {
      tab_id: "demo-api:2",
      workspace_id: "demo-api",
      number: 2,
      label: "tests",
      focused: false,
      pane_count: 1,
      agent_status: "idle",
    },
  ],
  panes: [
    pane("demo-api-1", "demo-api", "demo-api:1", "claude", "blocked", true),
    pane("demo-api-2", "demo-api", "demo-api:1", "codex", "working", false),
    pane("demo-api-3", "demo-api", "demo-api:1", "npm test", "idle", false),
  ],
  agents: [
    agent("demo-api-1", "demo-api", "demo-api:1", "claude", "blocked", true),
    agent("demo-api-2", "demo-api", "demo-api:1", "codex", "working", false),
    agent("demo-web-1", "demo-web", "demo-web:1", "pi", "done", false),
  ],
  active_layout: {
    workspace_id: "demo-api",
    tab_id: "demo-api:1",
    zoomed: false,
    area: { x: 0, y: 0, width: 120, height: 40 },
    focused_pane_id: "demo-api-1",
    panes: [
      { pane_id: "demo-api-1", focused: true, rect: { x: 0, y: 0, width: 60, height: 20 } },
      { pane_id: "demo-api-2", focused: false, rect: { x: 60, y: 0, width: 60, height: 20 } },
      { pane_id: "demo-api-3", focused: false, rect: { x: 0, y: 20, width: 120, height: 20 } },
    ],
    splits: [],
  },
};

type Status = "idle" | "working" | "blocked" | "done" | "unknown";

function pane(
  pane_id: string,
  workspace_id: string,
  tab_id: string,
  label: string,
  agent_status: Status,
  focused: boolean,
) {
  return {
    pane_id,
    terminal_id: `${pane_id}-term`,
    workspace_id,
    tab_id,
    focused,
    cwd: `/workspace/${workspace_id.replace("demo-", "")}`,
    label,
    agent: label,
    display_agent: label,
    agent_status,
    revision: 1,
  };
}

function agent(
  pane_id: string,
  workspace_id: string,
  tab_id: string,
  name: string,
  agent_status: Status,
  focused: boolean,
) {
  return {
    terminal_id: `${pane_id}-term`,
    name,
    agent: name,
    display_agent: name,
    agent_status,
    custom_status: agent_status === "working" ? "editing files" : undefined,
    workspace_id,
    tab_id,
    pane_id,
    focused,
    cwd: `/workspace/${workspace_id.replace("demo-", "")}`,
    revision: 1,
  };
}

function frameFor(paneId: string): PaneFrame {
  const ansi =
    paneId === "demo-api-1"
      ? "\x1b[2J\x1b[H\x1b[1mClaude Code\x1b[0m\n\n\x1b[31mPermission required\x1b[0m\nApprove editing src/api/schema.rs?\n\n[Allow once] [Deny]\n"
      : paneId === "demo-api-2"
        ? "\x1b[2J\x1b[H\x1b[1mCodex\x1b[0m\n\n\x1b[33mWorking...\x1b[0m updating terminal streaming contract\n\n✓ read schema\n• writing tests\n"
        : "\x1b[2J\x1b[H$ npm test\n\n1963 tests passed\n";
  return { pane_id: paneId, ansi, revision: Date.now() };
}
