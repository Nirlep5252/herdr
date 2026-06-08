import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import "./styles.css";
import type {
  DesktopAction,
  DesktopSnapshot,
  HerdrDesktopApi,
  HerdrStatus,
  PaneInfo,
  PaneLayoutPane,
} from "../shared/herdr";
import { createMockApi } from "./mock-api";

const api: HerdrDesktopApi = window.herdr ?? createMockApi();
const app = document.querySelector<HTMLDivElement>("#app");

if (!app) {
  throw new Error("missing #app root");
}

interface PaneTerminal {
  terminal: Terminal;
  fit: FitAddon;
  element: HTMLElement;
  lastRevision?: number;
}

let snapshot: DesktopSnapshot | undefined;
let status: HerdrStatus = { connected: false, mode: "demo", message: "Starting" };
let activePaneId: string | undefined;
let commandOpen = false;
const terminals = new Map<string, PaneTerminal>();

app.innerHTML = `
  <div class="shell">
    <aside class="sidebar">
      <header class="brand">
        <div class="logo">H</div>
        <div>
          <h1>Herdr Desktop</h1>
          <p id="connection-status">Connecting...</p>
        </div>
      </header>
      <section>
        <div class="section-title">Workspaces</div>
        <div id="workspace-list" class="list"></div>
      </section>
      <section class="agents-section">
        <div class="section-title">Agents</div>
        <div id="agent-list" class="list"></div>
      </section>
    </aside>
    <main class="main">
      <div class="topbar">
        <div id="tab-list" class="tabs"></div>
        <div class="shortcut-hint">Ctrl+K palette · Ctrl+T tab · Ctrl+Shift+R split right · Ctrl+Shift+D split down</div>
      </div>
      <div id="pane-grid" class="pane-grid"></div>
    </main>
  </div>
  <div id="palette" class="palette hidden">
    <div class="palette-card">
      <input id="palette-input" placeholder="Jump to workspace or agent..." />
      <div id="palette-items"></div>
    </div>
  </div>
`;

const workspaceList = requireElement("workspace-list");
const agentList = requireElement("agent-list");
const tabList = requireElement("tab-list");
const paneGrid = requireElement("pane-grid");
const connectionStatus = requireElement("connection-status");
const palette = requireElement("palette");
const paletteInput = requireElement<HTMLInputElement>("palette-input");
const paletteItems = requireElement("palette-items");

window.addEventListener("resize", () => {
  for (const pane of terminals.values()) {
    pane.fit.fit();
  }
});

window.addEventListener("keydown", (event) => {
  const key = event.key.toLowerCase();
  if ((event.ctrlKey || event.metaKey) && key === "k") {
    event.preventDefault();
    togglePalette();
    return;
  }
  if (event.key === "Escape" && commandOpen) {
    event.preventDefault();
    closePalette();
    return;
  }
  if ((event.ctrlKey || event.metaKey) && key === "t") {
    event.preventDefault();
    // The backend tab-create API is available via CLI/socket, but the v1 shell
    // currently exposes creation from command palette in a later slice.
    flashStatus("New tab shortcut received");
    return;
  }
  if ((event.ctrlKey || event.metaKey) && event.shiftKey && key === "r") {
    event.preventDefault();
    void splitActivePane("right");
    return;
  }
  if ((event.ctrlKey || event.metaKey) && event.shiftKey && key === "d") {
    event.preventDefault();
    void splitActivePane("down");
    return;
  }
  if ((event.ctrlKey || event.metaKey) && key === "w" && activePaneId) {
    event.preventDefault();
    void runAction({ type: "pane.close", paneId: activePaneId });
  }
});

paletteInput.addEventListener("input", renderPalette);

void start();

async function start() {
  await refreshStatus();
  await refreshSnapshot();
  setInterval(() => {
    void refreshStatus();
    void refreshSnapshot();
  }, 1500);
  setInterval(() => {
    void refreshPaneFrames();
  }, 700);
}

async function refreshStatus() {
  try {
    status = await api.status();
  } catch (error) {
    status = { connected: false, mode: "socket", message: errorMessage(error) };
  }
  renderStatus();
}

async function refreshSnapshot() {
  try {
    snapshot = await api.snapshot();
    activePaneId = snapshot.focused_pane_id ?? activePaneId;
    render();
    await refreshPaneFrames();
  } catch (error) {
    connectionStatus.textContent = errorMessage(error);
  }
}

async function refreshPaneFrames() {
  const activePanes = snapshot?.active_layout?.panes ?? [];
  await Promise.all(
    activePanes.map(async (pane) => {
      const target = terminals.get(pane.pane_id);
      if (!target) return;
      const frame = await api.paneFrame(pane.pane_id);
      if (target.lastRevision === frame.revision) return;
      target.lastRevision = frame.revision;
      target.terminal.reset();
      target.terminal.write(frame.ansi);
    }),
  );
}

function render() {
  if (!snapshot) return;
  renderStatus();
  renderWorkspaces();
  renderAgents();
  renderTabs();
  renderPanes();
  if (commandOpen) {
    renderPalette();
  }
}

function renderStatus() {
  connectionStatus.textContent = status.connected
    ? status.mode === "demo"
      ? "Demo mode"
      : "Connected"
    : status.message ?? "Disconnected";
  connectionStatus.className = status.connected ? "ok" : "error";
}

function renderWorkspaces() {
  workspaceList.replaceChildren();
  for (const workspace of snapshot?.workspaces ?? []) {
    const button = document.createElement("button");
    button.className = `row ${workspace.focused ? "active" : ""}`;
    button.innerHTML = `
      <span class="status-dot ${workspace.agent_status}"></span>
      <span class="row-main">${escapeHtml(workspace.label)}</span>
      <span class="row-meta">${workspace.pane_count} panes</span>
    `;
    button.addEventListener("click", () =>
      void runAction({ type: "workspace.focus", workspaceId: workspace.workspace_id }),
    );
    workspaceList.append(button);
  }
}

function renderAgents() {
  agentList.replaceChildren();
  for (const agent of snapshot?.agents ?? []) {
    const button = document.createElement("button");
    button.className = `row ${agent.focused ? "active" : ""}`;
    const label = agent.display_agent ?? agent.name ?? agent.agent ?? "agent";
    button.innerHTML = `
      <span class="status-dot ${agent.agent_status}"></span>
      <span class="row-main">${escapeHtml(label)}</span>
      <span class="row-meta">${escapeHtml(agent.custom_status ?? agent.agent_status)}</span>
    `;
    button.addEventListener("click", () =>
      void runAction({ type: "agent.focus", target: agent.name ?? agent.pane_id }),
    );
    agentList.append(button);
  }
}

function renderTabs() {
  tabList.replaceChildren();
  const activeWorkspaceId = snapshot?.active_workspace_id;
  for (const tab of snapshot?.tabs.filter((candidate) => candidate.workspace_id === activeWorkspaceId) ?? []) {
    const button = document.createElement("button");
    button.className = `tab ${tab.focused ? "active" : ""}`;
    button.innerHTML = `<span class="status-dot ${tab.agent_status}"></span>${escapeHtml(tab.label)}`;
    button.addEventListener("click", () => void runAction({ type: "tab.focus", tabId: tab.tab_id }));
    tabList.append(button);
  }
}

function renderPanes() {
  const layout = snapshot?.active_layout;
  if (!layout) {
    paneGrid.textContent = "No active tab";
    return;
  }

  const seen = new Set<string>();
  for (const pane of layout.panes) {
    seen.add(pane.pane_id);
    renderPane(pane);
  }

  for (const [paneId, terminal] of terminals) {
    if (!seen.has(paneId)) {
      terminal.element.remove();
      terminals.delete(paneId);
    }
  }
}

function renderPane(layoutPane: PaneLayoutPane) {
  const pane = snapshot?.panes.find((candidate) => candidate.pane_id === layoutPane.pane_id);
  const layout = snapshot?.active_layout;
  if (!pane || !layout) return;

  let target = terminals.get(layoutPane.pane_id);
  if (!target) {
    target = createPaneTerminal(pane);
    terminals.set(layoutPane.pane_id, target);
    paneGrid.append(target.element);
  }

  const rect = layoutPane.rect;
  const area = layout.area;
  const left = (rect.x / area.width) * 100;
  const top = (rect.y / area.height) * 100;
  const width = (rect.width / area.width) * 100;
  const height = (rect.height / area.height) * 100;
  target.element.style.left = `${left}%`;
  target.element.style.top = `${top}%`;
  target.element.style.width = `${width}%`;
  target.element.style.height = `${height}%`;
  target.element.classList.toggle("focused", layoutPane.focused || pane.focused);
  target.element.querySelector(".pane-title")!.textContent = paneTitle(pane);
  target.element.querySelector(".pane-status")!.textContent = pane.agent_status;
  target.fit.fit();
}

function createPaneTerminal(pane: PaneInfo): PaneTerminal {
  const element = document.createElement("section");
  element.className = "pane";
  element.innerHTML = `
    <header class="pane-header">
      <span class="pane-title">${escapeHtml(paneTitle(pane))}</span>
      <span class="pane-status ${pane.agent_status}">${pane.agent_status}</span>
    </header>
    <div class="terminal"></div>
  `;
  element.addEventListener("click", () => {
    activePaneId = pane.pane_id;
  });

  const terminalElement = element.querySelector<HTMLDivElement>(".terminal")!;
  const terminal = new Terminal({
    convertEol: true,
    cursorBlink: true,
    fontFamily: "JetBrains Mono, Menlo, Consolas, monospace",
    fontSize: 13,
    theme: {
      background: "#11111b",
      foreground: "#cdd6f4",
      cursor: "#89b4fa",
      selectionBackground: "#45475a",
    },
  });
  const fit = new FitAddon();
  terminal.loadAddon(fit);
  terminal.open(terminalElement);
  terminal.onData((text) => {
    void runAction({ type: "pane.send", paneId: pane.pane_id, text });
  });
  queueMicrotask(() => fit.fit());
  return { terminal, fit, element };
}

function paneTitle(pane: PaneInfo) {
  return pane.display_agent ?? pane.label ?? pane.title ?? pane.agent ?? pane.pane_id;
}

async function splitActivePane(direction: "right" | "down") {
  if (!activePaneId) {
    flashStatus("No active pane");
    return;
  }
  await runAction({ type: "pane.split", paneId: activePaneId, direction });
}

async function runAction(action: DesktopAction) {
  try {
    await api.action(action);
    await refreshSnapshot();
  } catch (error) {
    flashStatus(errorMessage(error));
  }
}

function togglePalette() {
  commandOpen ? closePalette() : openPalette();
}

function openPalette() {
  commandOpen = true;
  palette.classList.remove("hidden");
  paletteInput.value = "";
  renderPalette();
  paletteInput.focus();
}

function closePalette() {
  commandOpen = false;
  palette.classList.add("hidden");
}

function renderPalette() {
  const query = paletteInput.value.toLowerCase();
  paletteItems.replaceChildren();
  const items = [
    ...(snapshot?.workspaces ?? []).map((workspace) => ({
      label: `Workspace: ${workspace.label}`,
      meta: workspace.agent_status,
      run: () => runAction({ type: "workspace.focus", workspaceId: workspace.workspace_id }),
    })),
    ...(snapshot?.agents ?? []).map((agent) => ({
      label: `Agent: ${agent.display_agent ?? agent.name ?? agent.agent ?? agent.pane_id}`,
      meta: agent.agent_status,
      run: () => runAction({ type: "agent.focus", target: agent.name ?? agent.pane_id }),
    })),
  ].filter((item) => item.label.toLowerCase().includes(query));

  for (const item of items.slice(0, 12)) {
    const button = document.createElement("button");
    button.className = "palette-item";
    button.innerHTML = `<span>${escapeHtml(item.label)}</span><span>${escapeHtml(item.meta)}</span>`;
    button.addEventListener("click", () => {
      closePalette();
      void item.run();
    });
    paletteItems.append(button);
  }
}

function flashStatus(message: string) {
  connectionStatus.textContent = message;
  connectionStatus.className = "warn";
  window.setTimeout(renderStatus, 1600);
}

function requireElement<T extends HTMLElement = HTMLElement>(id: string): T {
  const element = document.getElementById(id);
  if (!element) {
    throw new Error(`missing #${id}`);
  }
  return element as T;
}

function escapeHtml(value: string): string {
  return value.replace(/[&<>"']/g, (char) => {
    switch (char) {
      case "&":
        return "&amp;";
      case "<":
        return "&lt;";
      case ">":
        return "&gt;";
      case '"':
        return "&quot;";
      default:
        return "&#039;";
    }
  });
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
