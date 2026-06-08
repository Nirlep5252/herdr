import { contextBridge, ipcRenderer } from "electron";
import type { DesktopAction, HerdrDesktopApi } from "../shared/herdr.js";

const api: HerdrDesktopApi = {
  status: () => ipcRenderer.invoke("herdr:status"),
  snapshot: () => ipcRenderer.invoke("herdr:snapshot"),
  paneFrame: (paneId: string) => ipcRenderer.invoke("herdr:pane-frame", paneId),
  action: (action: DesktopAction) => ipcRenderer.invoke("herdr:action", action),
};

contextBridge.exposeInMainWorld("herdr", api);
