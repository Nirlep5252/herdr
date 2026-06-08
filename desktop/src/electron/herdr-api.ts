import { spawn } from "node:child_process";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import type {
  DesktopAction,
  DesktopSnapshot,
  HerdrStatus,
  PaneFrame,
} from "../shared/herdr.js";

interface ApiSuccess<T = unknown> {
  id: string;
  result: T;
}

interface ApiError {
  id: string;
  error: {
    code: string;
    message: string;
  };
}

type ApiResponse<T = unknown> = ApiSuccess<T> | ApiError;

let requestSeq = 0;

export class HerdrApi {
  private lastError: string | undefined;
  private readonly demoMode = process.env.HERDR_DESKTOP_DEMO === "1";

  async status(): Promise<HerdrStatus> {
    if (this.demoMode) {
      return { connected: true, mode: "demo", message: "Demo data" };
    }

    const socketPath = resolveApiSocketPath();
    try {
      await this.request("ping", {});
      this.lastError = undefined;
      return { connected: true, mode: "socket", socketPath };
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      this.lastError = message;
      return { connected: false, mode: "socket", socketPath, message };
    }
  }

  async snapshot(): Promise<DesktopSnapshot> {
    if (this.demoMode) {
      return demoSnapshot();
    }
    const response = await this.request<{ type: "desktop_snapshot"; snapshot: DesktopSnapshot }>(
      "desktop.snapshot",
      {},
    );
    return response.snapshot;
  }

  async paneFrame(paneId: string): Promise<PaneFrame> {
    if (this.demoMode) {
      return demoPaneFrame(paneId);
    }

    const response = await this.request<{
      type: "pane_read";
      read: { pane_id: string; text: string; revision?: number };
    }>("pane.read", {
      pane_id: paneId,
      source: "visible",
      format: "ansi",
      strip_ansi: false,
    });

    return {
      pane_id: response.read.pane_id,
      ansi: response.read.text,
      revision: response.read.revision,
    };
  }

  async action(action: DesktopAction): Promise<void> {
    if (this.demoMode) {
      return;
    }

    switch (action.type) {
      case "workspace.focus":
        await this.request("workspace.focus", { workspace_id: action.workspaceId });
        return;
      case "tab.focus":
        await this.request("tab.focus", { tab_id: action.tabId });
        return;
      case "agent.focus":
        await this.request("agent.focus", { target: action.target });
        return;
      case "pane.split":
        await this.request("pane.split", {
          target_pane_id: action.paneId,
          direction: action.direction,
          focus: true,
        });
        return;
      case "pane.zoom":
        await this.request("pane.zoom", { pane_id: action.paneId });
        return;
      case "pane.close":
        await this.request("pane.close", { pane_id: action.paneId });
        return;
      case "pane.send":
        await this.request("pane.send_text", { pane_id: action.paneId, text: action.text });
        return;
    }
  }

  async request<T>(method: string, params: Record<string, unknown>): Promise<T> {
    if (this.demoMode) {
      throw new Error("demo mode does not support raw requests");
    }

    try {
      return await requestOverSocket<T>({ id: nextRequestId(method), method, params });
    } catch (socketError) {
      const cliResponse = await requestOverCli<T>(method, params).catch(() => {
        throw socketError;
      });
      return cliResponse;
    }
  }

  error(): string | undefined {
    return this.lastError;
  }
}

function nextRequestId(method: string): string {
  requestSeq += 1;
  return `desktop:${method}:${requestSeq}`;
}

function resolveApiSocketPath(): string {
  if (process.env.HERDR_SOCKET_PATH) {
    return process.env.HERDR_SOCKET_PATH;
  }

  const session = process.env.HERDR_SESSION;
  const configDir = process.env.XDG_CONFIG_HOME
    ? path.join(process.env.XDG_CONFIG_HOME, "herdr")
    : platformConfigDir();

  if (session && session !== "default") {
    return path.join(configDir, "sessions", session, "herdr.sock");
  }
  return path.join(configDir, "herdr.sock");
}

function platformConfigDir(): string {
  if (process.platform === "win32") {
    if (process.env.APPDATA) {
      return path.join(process.env.APPDATA, "herdr");
    }
    if (process.env.USERPROFILE) {
      return path.join(process.env.USERPROFILE, "AppData", "Roaming", "herdr");
    }
  }
  return path.join(os.homedir(), ".config", "herdr");
}

function requestOverSocket<T>(request: {
  id: string;
  method: string;
  params: Record<string, unknown>;
}): Promise<T> {
  return new Promise((resolve, reject) => {
    const socketPath = resolveNodeSocketPath(resolveApiSocketPath());
    const socket = net.createConnection(socketPath);
    let buffer = "";
    let settled = false;

    const fail = (error: Error) => {
      if (settled) return;
      settled = true;
      socket.destroy();
      reject(error);
    };

    socket.setTimeout(2000, () => fail(new Error(`timed out connecting to ${socketPath}`)));
    socket.on("error", fail);
    socket.on("connect", () => {
      socket.write(`${JSON.stringify(request)}\n`);
    });
    socket.on("data", (chunk: Buffer) => {
      buffer += chunk.toString("utf8");
      const newline = buffer.indexOf("\n");
      if (newline === -1) {
        return;
      }
      const line = buffer.slice(0, newline);
      settled = true;
      socket.end();
      try {
        const response = JSON.parse(line) as ApiResponse<T>;
        if ("error" in response) {
          reject(new Error(response.error.message));
        } else {
          resolve(response.result);
        }
      } catch (error) {
        reject(error);
      }
    });
  });
}

function resolveNodeSocketPath(socketPath: string): string {
  if (process.platform !== "win32") {
    return socketPath;
  }
  if (socketPath.startsWith("\\\\.\\")) {
    return socketPath;
  }
  return `\\\\.\\pipe\\${socketPath}`;
}

async function requestOverCli<T>(
  method: string,
  params: Record<string, unknown>,
): Promise<T> {
  if (method === "desktop.snapshot") {
    const stdout = await runHerdr(["desktop", "snapshot"]);
    return parseCliJsonResponse<T>(stdout);
  }

  if (method === "pane.read") {
    const paneId = stringParam(params, "pane_id");
    const text = await runHerdr(["pane", "read", paneId, "--source", "visible", "--raw"]);
    return {
      type: "pane_read",
      read: { pane_id: paneId, text, revision: Date.now() },
    } as T;
  }

  if (method === "pane.send_text") {
    await runHerdr(["pane", "send-text", stringParam(params, "pane_id"), stringParam(params, "text")]);
    return { type: "ok" } as T;
  }

  if (method === "workspace.focus") {
    return runJsonCli<T>(["workspace", "focus", stringParam(params, "workspace_id")]);
  }

  if (method === "tab.focus") {
    return runJsonCli<T>(["tab", "focus", stringParam(params, "tab_id")]);
  }

  if (method === "agent.focus") {
    return runJsonCli<T>(["agent", "focus", stringParam(params, "target")]);
  }

  if (method === "pane.split") {
    return runJsonCli<T>([
      "pane",
      "split",
      stringParam(params, "target_pane_id"),
      "--direction",
      stringParam(params, "direction"),
      "--focus",
    ]);
  }

  if (method === "pane.zoom") {
    return runJsonCli<T>(["pane", "zoom", stringParam(params, "pane_id")]);
  }

  if (method === "pane.close") {
    return runJsonCli<T>(["pane", "close", stringParam(params, "pane_id")]);
  }

  throw new Error(`CLI fallback is not available for ${method}`);
}

async function runJsonCli<T>(args: string[]): Promise<T> {
  const stdout = await runHerdr(args);
  return parseCliJsonResponse<T>(stdout);
}

function parseCliJsonResponse<T>(stdout: string): T {
  const response = JSON.parse(stdout) as ApiResponse<T>;
  if ("error" in response) {
    throw new Error(response.error.message);
  }
  return response.result;
}

function stringParam(params: Record<string, unknown>, key: string): string {
  const value = params[key];
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`missing ${key}`);
  }
  return value;
}

function runHerdr(args: string[]): Promise<string> {
  const candidates = [
    process.env.HERDR_BINARY,
    path.resolve(process.cwd(), "..", "target", "debug", process.platform === "win32" ? "herdr.exe" : "herdr"),
    "herdr",
  ].filter(Boolean) as string[];

  return new Promise((resolve, reject) => {
    const tryCandidate = (index: number) => {
      const command = candidates[index];
      if (!command) {
        reject(new Error("could not find herdr binary"));
        return;
      }

      const child = spawn(command, args, {
        stdio: ["ignore", "pipe", "pipe"],
        windowsHide: true,
      });
      let stdout = "";
      let stderr = "";
      child.stdout.on("data", (chunk: Buffer) => {
        stdout += chunk.toString("utf8");
      });
      child.stderr.on("data", (chunk: Buffer) => {
        stderr += chunk.toString("utf8");
      });
      child.on("error", () => tryCandidate(index + 1));
      child.on("close", (code) => {
        if (code === 0) {
          resolve(stdout);
        } else if (index + 1 < candidates.length) {
          tryCandidate(index + 1);
        } else {
          reject(new Error(stderr.trim() || `herdr exited with ${code}`));
        }
      });
    };

    tryCandidate(0);
  });
}

function demoSnapshot(): DesktopSnapshot {
  return {
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
      demoPane("demo-api-1", "demo-api", "demo-api:1", "claude", "blocked", true),
      demoPane("demo-api-2", "demo-api", "demo-api:1", "codex", "working", false),
      demoPane("demo-api-3", "demo-api", "demo-api:1", "npm test", "idle", false),
    ],
    agents: [
      demoAgent("demo-api-1", "demo-api", "demo-api:1", "claude", "blocked", true),
      demoAgent("demo-api-2", "demo-api", "demo-api:1", "codex", "working", false),
      demoAgent("demo-web-1", "demo-web", "demo-web:1", "pi", "done", false),
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
}

function demoPane(
  pane_id: string,
  workspace_id: string,
  tab_id: string,
  label: string,
  agent_status: PaneFrameStatus,
  focused: boolean,
) {
  return {
    pane_id,
    terminal_id: pane_id.replace("pane", "terminal"),
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

type PaneFrameStatus = "idle" | "working" | "blocked" | "done" | "unknown";

function demoAgent(
  pane_id: string,
  workspace_id: string,
  tab_id: string,
  agent: string,
  agent_status: PaneFrameStatus,
  focused: boolean,
) {
  return {
    terminal_id: pane_id.replace("pane", "terminal"),
    name: agent,
    agent,
    display_agent: agent,
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

function demoPaneFrame(paneId: string): PaneFrame {
  const title = paneId === "demo-api-1" ? "Claude Code" : paneId === "demo-api-2" ? "Codex" : "Shell";
  const body =
    paneId === "demo-api-1"
      ? "\x1b[31mPermission required\x1b[0m\nApprove editing src/api.rs?\n\n[Allow once] [Deny]\n"
      : paneId === "demo-api-2"
        ? "\x1b[33mWorking...\x1b[0m updating terminal streaming contract\n\n✓ read schema\n• writing tests\n"
        : "$ npm test\n\n1963 tests passed\n";
  return {
    pane_id: paneId,
    ansi: `\x1b[2J\x1b[H\x1b[1m${title}\x1b[0m\n${body}`,
    revision: Date.now(),
  };
}
