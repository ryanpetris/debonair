// Exposes the narrow window API; the window has no Node or Electron access.
import { contextBridge, ipcRenderer, type IpcRendererEvent } from "electron";
import type { Action, AppState, DesktopApi, InputMessage } from "../shared/state.ts";

const api: DesktopApi = {
  state: () => ipcRenderer.invoke("state") as Promise<AppState>,
  onState: (listener) => {
    const handler = (_: IpcRendererEvent, state: AppState) => listener(state);
    ipcRenderer.on("state", handler);
    return () => void ipcRenderer.removeListener("state", handler);
  },
  act: (action: Action) => ipcRenderer.invoke("act", action),
  // Input is fire-and-forget so a busy main process never delays the window's events.
  input: (message: InputMessage) => ipcRenderer.send("input", message),
};

contextBridge.exposeInMainWorld("debonair", api);
