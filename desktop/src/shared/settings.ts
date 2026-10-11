// Preferences kept by the app. They affect only how this computer's input is captured and sent;
// how the adapter appears to Targets is saved on the adapter.
import { keyUsage, type Modifier } from "./keys.ts";

/**
 * A key combination pressed on this computer. With a key, it fires when that key goes down while
 * exactly these modifiers are held. Without one, it fires when the modifiers are released after
 * being held together with no other key, so the same modifiers still work in combinations.
 */
export interface Shortcut {
  modifiers: Modifier[];
  code: string | null;
}

export type MacroStep =
  /** Presses the keys in order, holds them, then releases them in reverse order. */
  | { kind: "keys"; codes: string[] }
  /** Types text with a US keyboard layout. */
  | { kind: "text"; text: string }
  | { kind: "delay"; ms: number };

/** A named key combination or sequence, sent from a button or a shortcut. */
export interface Macro {
  id: string;
  name: string;
  steps: MacroStep[];
  shortcut: Shortcut | null;
  /** Shown as a button while capturing. */
  button: boolean;
}

export interface Settings {
  /** Multiplier for pointer motion. */
  pointerSpeed: number;
  /** Uses motion before this computer's pointer acceleration, where supported. */
  rawPointer: boolean;
  /** Multiplier for scrolling. */
  scrollSpeed: number;
  reverseScroll: boolean;
  /** Sends a trackpad pinch as Ctrl and scroll, which zooms on most Targets. */
  pinchZoom: boolean;
  /** Captures in full screen, where system shortcuts such as Alt+Tab can be captured too. */
  fullScreen: boolean;
  releaseShortcut: Shortcut;
  /** Reports this computer's battery level to Targets. */
  shareBattery: boolean;
  macros: Macro[];
  /** Adapter ID to connect to when several are plugged in. */
  adapter: string | null;
  /** Names for paired Targets, by address. */
  targetNames: Record<string, string>;
}

const keys = (...codes: string[]): MacroStep[] => [{ kind: "keys", codes }];

export const DEFAULT_MACROS: Macro[] = [
  {
    id: "ctrl-alt-delete",
    name: "Ctrl+Alt+Delete",
    steps: keys("ControlLeft", "AltLeft", "Delete"),
    shortcut: { modifiers: ["ctrl", "alt"], code: "End" },
    button: true,
  },
  { id: "ctrl-shift-escape", name: "Ctrl+Shift+Esc", steps: keys("ControlLeft", "ShiftLeft", "Escape"), shortcut: null, button: true },
  { id: "alt-tab", name: "Alt+Tab", steps: keys("AltLeft", "Tab"), shortcut: null, button: true },
  { id: "alt-f4", name: "Alt+F4", steps: keys("AltLeft", "F4"), shortcut: null, button: true },
  { id: "super", name: "Super", steps: keys("MetaLeft"), shortcut: null, button: true },
  { id: "escape", name: "Esc", steps: keys("Escape"), shortcut: null, button: true },
  { id: "print-screen", name: "Print Screen", steps: keys("PrintScreen"), shortcut: null, button: true },
  { id: "volume-down", name: "Volume Down", steps: keys("AudioVolumeDown"), shortcut: null, button: false },
  { id: "volume-up", name: "Volume Up", steps: keys("AudioVolumeUp"), shortcut: null, button: false },
  { id: "mute", name: "Mute", steps: keys("AudioVolumeMute"), shortcut: null, button: false },
  { id: "play-pause", name: "Play/Pause", steps: keys("MediaPlayPause"), shortcut: null, button: false },
];

export const DEFAULT_SETTINGS: Settings = {
  pointerSpeed: 1,
  rawPointer: false,
  scrollSpeed: 1,
  reverseScroll: false,
  pinchZoom: true,
  fullScreen: false,
  releaseShortcut: { modifiers: ["ctrl", "alt"], code: null },
  shareBattery: true,
  macros: DEFAULT_MACROS,
  adapter: null,
  targetNames: {},
};

export const SPEED_RANGE = { min: 0.1, max: 5 };
const MODIFIERS: readonly Modifier[] = ["ctrl", "alt", "shift", "meta"];

const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

function speed(value: unknown, fallback: number) {
  return typeof value === "number" && Number.isFinite(value)
    ? Math.min(SPEED_RANGE.max, Math.max(SPEED_RANGE.min, value))
    : fallback;
}

export function shortcutFrom(value: unknown): Shortcut | null {
  if (!isRecord(value) || !Array.isArray(value.modifiers)) return null;
  const modifiers = MODIFIERS.filter((m) => (value.modifiers as unknown[]).includes(m));
  const code = typeof value.code === "string" && keyUsage(value.code)?.page === "keyboard" ? value.code : null;
  if (code === null && modifiers.length < 2) return null;
  if (code !== null && modifiers.length === 0) return null;
  return { modifiers, code };
}

function stepFrom(value: unknown): MacroStep | null {
  if (!isRecord(value)) return null;
  if (value.kind === "keys" && Array.isArray(value.codes)) {
    const codes = value.codes.filter((c): c is string => typeof c === "string" && keyUsage(c) !== undefined);
    return codes.length ? { kind: "keys", codes } : null;
  }
  if (value.kind === "text" && typeof value.text === "string" && value.text.length) {
    return { kind: "text", text: value.text.slice(0, 4096) };
  }
  if (value.kind === "delay" && typeof value.ms === "number" && Number.isFinite(value.ms)) {
    return { kind: "delay", ms: Math.min(10000, Math.max(0, Math.round(value.ms))) };
  }
  return null;
}

function macroFrom(value: unknown): Macro | null {
  if (!isRecord(value) || typeof value.id !== "string" || typeof value.name !== "string") return null;
  const steps = Array.isArray(value.steps) ? value.steps.map(stepFrom).filter((s) => s !== null) : [];
  if (!value.name.trim() || !steps.length) return null;
  return {
    id: value.id,
    name: value.name.trim(),
    steps,
    shortcut: shortcutFrom(value.shortcut),
    button: value.button !== false,
  };
}

/** Saved settings with every missing or invalid value replaced by its default. */
export function settingsFrom(value: unknown): Settings {
  if (!isRecord(value)) return structuredClone(DEFAULT_SETTINGS);
  const macros = Array.isArray(value.macros)
    ? value.macros.map(macroFrom).filter((m) => m !== null)
    : structuredClone(DEFAULT_MACROS);
  const names: Record<string, string> = {};
  if (isRecord(value.targetNames)) {
    for (const [address, name] of Object.entries(value.targetNames)) {
      if (typeof name === "string" && name.trim()) names[address] = name.trim();
    }
  }
  return {
    pointerSpeed: speed(value.pointerSpeed, DEFAULT_SETTINGS.pointerSpeed),
    rawPointer: typeof value.rawPointer === "boolean" ? value.rawPointer : DEFAULT_SETTINGS.rawPointer,
    scrollSpeed: speed(value.scrollSpeed, DEFAULT_SETTINGS.scrollSpeed),
    reverseScroll: typeof value.reverseScroll === "boolean" ? value.reverseScroll : DEFAULT_SETTINGS.reverseScroll,
    pinchZoom: typeof value.pinchZoom === "boolean" ? value.pinchZoom : DEFAULT_SETTINGS.pinchZoom,
    fullScreen: typeof value.fullScreen === "boolean" ? value.fullScreen : DEFAULT_SETTINGS.fullScreen,
    releaseShortcut: shortcutFrom(value.releaseShortcut) ?? structuredClone(DEFAULT_SETTINGS.releaseShortcut),
    shareBattery: typeof value.shareBattery === "boolean" ? value.shareBattery : DEFAULT_SETTINGS.shareBattery,
    macros,
    adapter: typeof value.adapter === "string" ? value.adapter : null,
    targetNames: names,
  };
}

const MODIFIER_LABELS: Record<Modifier, string> = { ctrl: "Ctrl", alt: "Alt", shift: "Shift", meta: "Super" };

export function shortcutLabel(shortcut: Shortcut, labelOf: (code: string) => string) {
  const parts = MODIFIERS.filter((m) => shortcut.modifiers.includes(m)).map((m) => MODIFIER_LABELS[m]);
  if (shortcut.code) parts.push(labelOf(shortcut.code));
  return parts.join("+");
}
