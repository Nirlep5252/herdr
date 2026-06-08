# Herdr Desktop plan

Herdr Desktop is a fast, single-window Windows/Linux GUI client for Herdr. It
keeps real terminal panes and uses native desktop shortcuts. The existing Herdr
server remains the source of truth for panes, sessions, agent detection,
persistence, integrations, and automation.

## Product requirements

- Name: Herdr Desktop.
- Platforms: Windows first, Linux second.
- Window model: single main window; no detached panes or multi-window layout in
  v1.
- Pane model: real terminal panes only; no interpreted agent chat UI.
- Navigation: workspace-first and agent-first surfaces are both first-class.
- Shortcuts: native desktop shortcuts, not prefix-driven terminal shortcuts.
- Remote: out of scope for v1.

## V1 surface

The main window contains:

- workspace sidebar with agent status rollups
- agent sidebar/section with all agents across workspaces
- tab bar for the active workspace
- terminal pane grid for the active tab
- command palette / quick switcher
- settings and integrations screens
- desktop notifications and in-app toasts

## Implementation sequence

### 1. Desktop server contract

Add a read-only API snapshot for the single-window shell:

- active workspace, tab, and focused pane
- all workspaces
- all tabs
- all panes
- all agents
- active tab layout

This lets the GUI initialize from one server-authored model and reuse Herdr's
existing workspace, pane, and agent rollup semantics.

### 2. Cross-platform terminal streaming

Add a GUI-suitable per-pane terminal stream that works on Windows and Linux:

- subscribe to a terminal/pane frame stream
- resize a pane's terminal surface
- send encoded keyboard, mouse, focus, paste, and clipboard events
- surface cursor state, hyperlinks, and graphics capability flags

This should reuse the existing `FrameData`, input encoding, and server-owned
PTY/ConPTY runtime where possible.

### 3. Desktop client shell

Build the single-window app around the server contract:

- connect to a local Herdr session
- load `desktop.snapshot`
- keep state fresh with event subscriptions
- render native workspace/agent/tab chrome
- render real terminal panes
- route native shortcuts to Herdr actions

The toolkit remains open. Responsiveness and terminal fidelity are the deciding
constraints.

### 4. Settings, integrations, and packaging

Add GUI flows for:

- theme/font/terminal settings
- notification and sound settings
- pane history and restore settings
- integration install/uninstall/status
- Windows/Linux packaging and update flows

## Non-goals for v1

- remote GUI attach
- multi-window or detachable panes
- custom agent chat views
- cloud sync or collaboration
- replacing the terminal client
