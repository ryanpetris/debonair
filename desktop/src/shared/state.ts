// State and actions shared by the main process and the window.
import type { Settings } from "./settings.ts";

export interface Target {
  /** Bluetooth address, most significant byte first, as "AA:BB:CC:DD:EE:FF". */
  address: string;
  random: boolean;
}

export type Link = "off" | "waiting" | "pairing" | "connected";
export type Security = "justWorks" | "passkey";

export interface AdapterStatus {
  id: string;
  name: string;
  firmwareVersion: string;
  board: string;
  development: boolean;
  ready: boolean;
  keyboard: boolean;
  mouse: boolean;
  security: Security;
  link: Link;
  connected: Target | null;
  targets: Target[];
  /** Keyboard LEDs set by the Target: bit 0 Num Lock, bit 1 Caps Lock, bit 2 Scroll Lock. */
  leds: number;
  smoothScrolling: boolean;
  battery: number;
  maxTargets: number;
}

export type Connection = "searching" | "connecting" | "connected";

export interface AppState {
  version: string;
  platform: string;
  connection: Connection;
  /** Adapter IDs that are plugged in. */
  adapters: string[];
  adapter: AdapterStatus | null;
  settings: Settings;
  /** A Target is waiting for the passkey it shows. */
  passkeyRequested: boolean;
  /** The result of the last pairing, until pairing starts again. */
  pairingResult: "succeeded" | "failed" | null;
}

export interface ConfigChange {
  name?: string;
  keyboard?: boolean;
  mouse?: boolean;
  security?: Security;
}

export type Action =
  | { type: "settings"; settings: Settings }
  | { type: "adapter.select"; id: string }
  | { type: "config"; change: ConfigChange }
  | { type: "pairing"; enabled: boolean }
  | { type: "passkey"; passkey: number | null }
  | { type: "disconnect" }
  | { type: "forget"; target: Target }
  | { type: "battery"; level: number }
  | { type: "restart" }
  | { type: "bootloader" }
  | { type: "clipboard" }
  | { type: "capturing"; capturing: boolean };

export type ActionResult = { ok: true; text?: string } | { ok: false; message: string };

/** Input for the Target, in the compact form sent from the window for every event. */
export type InputMessage =
  | { k: "kb"; m: number; keys: number[] }
  | { k: "cc"; u: number[] }
  | { k: "ms"; b: number; x: number; y: number; w: number; p: number };

/** The API the preload script exposes to the window as `window.debonair`. */
export interface DesktopApi {
  state(): Promise<AppState>;
  onState(listener: (state: AppState) => void): () => void;
  act(action: Action): Promise<ActionResult>;
  input(message: InputMessage): void;
}

export function formatAddress(bytes: Uint8Array) {
  return [...bytes].map((b) => b.toString(16).padStart(2, "0").toUpperCase()).join(":");
}

export function parseAddress(text: string): Uint8Array | null {
  const parts = text.split(":");
  if (parts.length !== 6 || !parts.every((p) => /^[0-9A-Fa-f]{2}$/.test(p))) return null;
  return Uint8Array.from(parts.map((p) => parseInt(p, 16)));
}

const isNumber = (value: unknown, min: number, max: number) =>
  typeof value === "number" && Number.isInteger(value) && value >= min && value <= max;

/** Whether a value from the window is a well-formed input message. */
export function isInputMessage(value: unknown): value is InputMessage {
  if (typeof value !== "object" || value === null) return false;
  const m = value as Record<string, unknown>;
  const usages = (list: unknown, max: number) =>
    Array.isArray(list) && list.length <= 16 && list.every((u) => isNumber(u, 0, max));
  switch (m.k) {
    case "kb":
      return isNumber(m.m, 0, 255) && usages(m.keys, 255);
    case "cc":
      return usages(m.u, 0xffff);
    case "ms":
      return (
        isNumber(m.b, 0, 31) &&
        [m.x, m.y, m.w, m.p].every((v) => isNumber(v, -0x7fffffff, 0x7fffffff))
      );
    default:
      return false;
  }
}
