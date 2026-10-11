declare const __DEBONAIR_VERSION__: string;
// Electron entry point: the window, saved settings and the adapter manager.
import { BrowserWindow, Menu, app, clipboard, ipcMain, nativeTheme } from "electron";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { Manager } from "./manager.ts";
import { listPorts, openSerial } from "./serial.ts";
import { simulatorPorts } from "./simulator.ts";
import { settingsFrom, type Settings } from "../shared/settings.ts";
import { isInputMessage, type Action, type ActionResult, type AppState } from "../shared/state.ts";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const log = (message: string) => console.log(`[debonair] ${new Date().toISOString()} ${message}`);

if (!app.requestSingleInstanceLock()) {
  app.exit(0);
}

let window: BrowserWindow | null = null;
let manager: Manager;

// ---- Settings ------------------------------------------------------------------------------

const settingsPath = () => join(app.getPath("userData"), "settings.json");

function loadSettings(): Settings {
  try {
    return settingsFrom(JSON.parse(readFileSync(settingsPath(), "utf8")));
  } catch {
    return settingsFrom(null);
  }
}

function saveSettings(settings: Settings) {
  try {
    mkdirSync(app.getPath("userData"), { recursive: true });
    writeFileSync(settingsPath(), JSON.stringify(settings, null, 2));
  } catch (error) {
    log(`saving settings failed: ${(error as Error).message}`);
  }
}

// ---- Window --------------------------------------------------------------------------------

/**
 * Window colors for the current theme, matching `--window`, `--header` and `--fg` in the
 * renderer's styles, so the window and its control buttons blend into the app's header.
 */
function colors() {
  const dark = nativeTheme.shouldUseDarkColors;
  return {
    window: dark ? "#222226" : "#fafafb",
    // The header is 52px with a 1px bottom border, which stays visible below the buttons.
    overlay: { color: dark ? "#2e2e32" : "#ebebed", symbolColor: dark ? "#ffffff" : "#2a2a30", height: 51 },
  };
}

function createWindow() {
  window = new BrowserWindow({
    width: 1040,
    height: 720,
    // Room for the header and the window controls drawn over it.
    minWidth: 860,
    minHeight: 520,
    title: "Debonair",
    backgroundColor: colors().window,
    // The app draws the title bar; the system draws only the window controls over its header.
    titleBarStyle: "hidden",
    titleBarOverlay: process.platform === "darwin" ? true : colors().overlay,
    // Centers the macOS buttons vertically in the header.
    trafficLightPosition: { x: 20, y: 18 },
    icon: join(root, "assets", "icons", "app-512.png"),
    show: false,
    webPreferences: {
      preload: join(root, "out", "preload", "index.cjs"),
      contextIsolation: true,
      sandbox: true,
      nodeIntegration: false,
      spellcheck: false,
    },
  });
  window.once("ready-to-show", () => window?.show());
  // When the window can no longer release what it holds, the main process does.
  window.on("blur", () => manager.releaseInput());
  window.webContents.on("render-process-gone", () => manager.releaseInput());
  window.webContents.on("did-start-loading", () => manager.releaseInput());
  window.on("closed", () => {
    window = null;
  });
  // Links never open inside the app.
  window.webContents.setWindowOpenHandler(() => ({ action: "deny" }));
  window.webContents.on("will-navigate", (event) => event.preventDefault());
  void window.loadFile(join(root, "out", "renderer", "index.html"));
}

/**
 * The macOS menu, whose Edit items make Cmd+C, Cmd+V and the like work in text fields. While
 * capturing there is no menu, so its shortcuts reach the Target.
 */
function setMenu(capturing: boolean) {
  if (process.platform !== "darwin") {
    // No menu bar: its accelerators and Alt activation would take keys meant for the Target.
    Menu.setApplicationMenu(null);
    return;
  }
  Menu.setApplicationMenu(
    Menu.buildFromTemplate(capturing ? [] : [{ role: "appMenu" }, { role: "editMenu" }]),
  );
}

function send(state: AppState) {
  window?.webContents.send("state", state);
}

// ---- Actions -------------------------------------------------------------------------------

async function act(action: Action): Promise<ActionResult> {
  switch (action.type) {
    case "settings": {
      const settings = settingsFrom(action.settings);
      saveSettings(settings);
      await manager.setSettings(settings);
      return { ok: true };
    }
    case "adapter.select": {
      const settings = { ...manager.state.settings, adapter: action.id };
      saveSettings(settings);
      await manager.setSettings(settings);
      return { ok: true };
    }
    case "clipboard":
      return { ok: true, text: await clipboard.readText() };
    case "capturing":
      setMenu(action.capturing === true);
      return { ok: true };
    case "config":
    case "pairing":
    case "passkey":
    case "disconnect":
    case "forget":
    case "battery":
    case "restart":
    case "bootloader":
      return manager.act(action);
    default:
      return { ok: false, message: "Debonair didn't recognize the action." };
  }
}

// ---- Startup -------------------------------------------------------------------------------

app.on("second-instance", () => {
  if (!window) return createWindow();
  if (window.isMinimized()) window.restore();
  window.focus();
});

app.whenReady().then(() => {
  setMenu(false);
  nativeTheme.on("updated", () => {
    window?.setBackgroundColor(colors().window);
    if (process.platform !== "darwin") window?.setTitleBarOverlay(colors().overlay);
  });
  manager = new Manager(
    process.env.DEBONAIR_SIMULATOR ? simulatorPorts(process.env.DEBONAIR_SIMULATOR) : { list: listPorts, open: openSerial },
    {
      version: __DEBONAIR_VERSION__,
      platform: process.platform,
      connection: "searching",
      adapters: [],
      adapter: null,
      settings: loadSettings(),
      passkeyRequested: false,
      pairingResult: null,
    },
    send,
  );
  ipcMain.handle("state", () => manager.state);
  ipcMain.handle("act", (_, action: Action) =>
    typeof action === "object" && action !== null
      ? act(action)
      : { ok: false, message: "Debonair didn't recognize the action." },
  );
  ipcMain.on("input", (_, message: unknown) => {
    if (isInputMessage(message)) manager.input(message);
  });
  manager.start();
  createWindow();
});

app.on("window-all-closed", () => {
  void manager?.stop().finally(() => app.quit());
});
