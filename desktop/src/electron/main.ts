import { app, BrowserWindow, ipcMain, nativeTheme } from "electron";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { HerdrApi } from "./herdr-api.js";
import type { DesktopAction } from "../shared/herdr.js";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const herdr = new HerdrApi();

async function createWindow() {
  nativeTheme.themeSource = "dark";

  const window = new BrowserWindow({
    width: 1320,
    height: 860,
    minWidth: 960,
    minHeight: 620,
    title: "Herdr Desktop",
    backgroundColor: "#11111b",
    webPreferences: {
      preload: path.join(__dirname, "preload.js"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
    },
  });

  if (process.env.VITE_DEV_SERVER_URL) {
    await window.loadURL(process.env.VITE_DEV_SERVER_URL);
  } else {
    await window.loadFile(path.join(__dirname, "..", "..", "dist", "index.html"));
  }
}

app.whenReady().then(async () => {
  registerIpcHandlers();
  await createWindow();

  app.on("activate", async () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      await createWindow();
    }
  });
});

app.on("window-all-closed", () => {
  if (process.platform !== "darwin") {
    app.quit();
  }
});

function registerIpcHandlers() {
  ipcMain.handle("herdr:status", () => herdr.status());
  ipcMain.handle("herdr:snapshot", () => herdr.snapshot());
  ipcMain.handle("herdr:pane-frame", (_event, paneId: string) => herdr.paneFrame(paneId));
  ipcMain.handle("herdr:action", (_event, action: DesktopAction) => herdr.action(action));
}
