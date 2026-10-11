import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Capture, type Platform } from "../shared/capture.ts";
import type { AppState } from "../shared/state.ts";
import { api, useAppState } from "./api.ts";
import { CaptureView } from "./components/CaptureView.tsx";
import { DevicesView, PasskeyDialog, targetName } from "./components/DevicesView.tsx";
import { MacrosView } from "./components/MacrosView.tsx";
import { SettingsView } from "./components/SettingsView.tsx";

type Tab = "control" | "devices" | "macros" | "settings";
const TABS: [Tab, string][] = [
  ["control", "Control"],
  ["devices", "Devices"],
  ["macros", "Key Combinations"],
  ["settings", "Settings"],
];

const PLATFORM: Platform = navigator.userAgent.includes("Mac OS") ? "mac" : navigator.userAgent.includes("Windows") ? "windows" : "other";
/** Wheel pixels per detent in Chromium on each platform. */
const PIXELS_PER_DETENT: Record<Platform, number> = { windows: 100, other: 53, mac: 40 };

interface BatteryManager extends EventTarget {
  level: number;
}

/** Reports this computer's battery level to the adapter while it changes. */
function useBattery() {
  useEffect(() => {
    const getBattery = (navigator as Navigator & { getBattery?: () => Promise<BatteryManager> }).getBattery;
    if (!getBattery) return;
    let battery: BatteryManager | null = null;
    const report = () => battery && void api().act({ type: "battery", level: Math.round(battery.level * 100) });
    void getBattery.call(navigator).then((b) => {
      battery = b;
      report();
      b.addEventListener("levelchange", report);
    });
    return () => battery?.removeEventListener("levelchange", report);
  }, []);
}

function statusText(state: AppState) {
  const adapter = state.adapter;
  if (!adapter) return state.connection === "connecting" ? "Connecting" : "No Adapter";
  switch (adapter.link) {
    case "off":
      return "Starting";
    case "pairing":
      return "Pairing";
    case "waiting":
      return "Not Connected";
    case "connected": {
      const index = adapter.targets.findIndex((t) => t.address === adapter.connected?.address);
      const target = adapter.connected;
      return target ? `Connected to ${targetName(state, target, index < 0 ? adapter.targets.length : index)}` : "Connected";
    }
  }
}

export function App() {
  const state = useAppState();
  const [tab, setTab] = useState<Tab>("control");
  const [capturing, setCapturing] = useState(false);
  const release = useRef<() => void>(() => {});
  useBattery();

  const capture = useMemo(
    () =>
      new Capture(
        {
          keyboard: (m, keys) => api().input({ k: "kb", m, keys }),
          consumer: (u) => api().input({ k: "cc", u }),
          mouse: (b, x, y, w, p) => api().input({ k: "ms", b, x, y, w, p }),
        },
        { setTimeout: (callback, ms) => window.setTimeout(callback, ms), clearTimeout: (handle) => window.clearTimeout(handle as number) },
        // Replaced as soon as the state arrives.
        { pointerSpeed: 1, rawPointer: false, scrollSpeed: 1, reverseScroll: false, pinchZoom: true, fullScreen: false, releaseShortcut: { modifiers: ["ctrl", "alt"], code: null }, shareBattery: true, macros: [], adapter: null, targetNames: {} },
        {
          platform: PLATFORM,
          pixelsPerDetent: PIXELS_PER_DETENT[PLATFORM],
          onRelease: () => release.current(),
          onMacro: (macro) => void capture.runMacro(macro),
        },
      ),
    [],
  );
  useEffect(() => {
    if (state) capture.settings = state.settings;
  }, [capture, state?.settings]);
  useEffect(() => {
    capture.capsLock = ((state?.adapter?.leds ?? 0) & 2) !== 0;
  }, [capture, state?.adapter?.leds]);
  const onCapturing = useCallback((value: boolean) => {
    setCapturing(value);
    void api().act({ type: "capturing", capturing: value });
  }, []);

  if (!state) return null;
  const link = state.adapter?.link;
  return (
    <div className="app">
      <header className="header">
        <span className="title">Debonair</span>
        <nav className="tabs">
          {TABS.map(([id, label]) => (
            <button key={id} type="button" disabled={capturing} className={tab === id ? "selected" : ""} onClick={() => setTab(id)}>
              {label}
            </button>
          ))}
        </nav>
        <span className="status">
          <span className={`dot ${link === "connected" ? "connected" : link === "pairing" ? "pairing" : ""}`} />
          <span className="status-text">{statusText(state)}</span>
        </span>
      </header>
      <main className="content">
        {tab === "control" ? (
          <CaptureView state={state} capture={capture} release={release} onCapturing={onCapturing} />
        ) : tab === "devices" ? (
          <DevicesView state={state} />
        ) : tab === "macros" ? (
          <MacrosView state={state} capture={capture} />
        ) : (
          <SettingsView state={state} />
        )}
      </main>
      {state.passkeyRequested ? <PasskeyDialog /> : null}
    </div>
  );
}
