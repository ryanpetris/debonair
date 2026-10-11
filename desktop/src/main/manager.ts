// Finds an adapter, keeps a session with it, and turns window actions into requests.
import { ErrorCode, Security, type Event } from "@debonair/protocol";
import { adapterStatus } from "./convert.ts";
import { AdapterError, Session, SessionError, type ByteStream, type Command } from "./session.ts";
import type { PortInfo } from "./serial.ts";
import type { Settings } from "../shared/settings.ts";
import { parseAddress, type Action, type ActionResult, type AppState, type InputMessage } from "../shared/state.ts";

export interface Ports {
  list(): Promise<PortInfo[]>;
  open(path: string): Promise<ByteStream>;
}

/** How often to look for adapters while none is connected. */
export const SCAN_MS = 1000;
const STATUS_ATTEMPTS = 3;

/** Sentences for the window naming the action that failed and why. `what` completes "The adapter
 * couldn't ...", such as "save the change". */
export function describe(error: unknown, what: string): string {
  const failed = `The adapter couldn't ${what}.`;
  if (error instanceof AdapterError) {
    switch (error.code) {
      case ErrorCode.NOT_READY:
        return `${failed} Bluetooth is still starting. Try again in a moment.`;
      case ErrorCode.BUSY:
        return `${failed} It is busy. Try again when the current operation finishes.`;
      case ErrorCode.NOT_FOUND:
        return `${failed} This device is no longer available.`;
      case ErrorCode.STORAGE_FAILED:
        return `${failed} Its storage failed.`;
      case ErrorCode.INVALID_VALUE:
        return `${failed} It didn't accept the value.`;
      case ErrorCode.UNSUPPORTED:
      case ErrorCode.UNKNOWN_COMMAND:
        return `${failed} Its firmware doesn't support this.`;
      default:
        return failed;
    }
  }
  if (error instanceof SessionError && error.timeout) {
    return `The adapter didn't respond, so it isn't known whether it could ${what}.`;
  }
  return `${failed} The adapter was disconnected.`;
}

export class Manager {
  readonly #ports: Ports;
  readonly #emit: (state: AppState) => void;
  #state: AppState;
  #session: Session | null = null;
  #timer: ReturnType<typeof setTimeout> | null = null;
  #scanning = false;
  #battery: number | null = null;
  #stopped = false;

  constructor(ports: Ports, state: AppState, emit: (state: AppState) => void) {
    this.#ports = ports;
    this.#state = state;
    this.#emit = emit;
  }

  get state() {
    return this.#state;
  }

  start() {
    void this.#scan();
  }

  async stop() {
    this.#stopped = true;
    if (this.#timer) clearTimeout(this.#timer);
    await this.#session?.close();
  }

  #update(change: Partial<AppState>) {
    this.#state = { ...this.#state, ...change };
    this.#emit(this.#state);
  }

  #schedule() {
    if (this.#stopped || this.#timer) return;
    this.#timer = setTimeout(() => {
      this.#timer = null;
      void this.#scan();
    }, SCAN_MS);
  }

  async #scan() {
    if (this.#scanning) return;
    this.#scanning = true;
    try {
      await this.#scanOnce();
    } finally {
      this.#scanning = false;
    }
    if (!this.#session) this.#schedule();
  }

  async #scanOnce() {
    let ports: PortInfo[] = [];
    try {
      ports = await this.#ports.list();
    } catch {
      // Listing can fail while a device is being plugged in; the next scan retries.
    }
    const ids = ports.map((p) => p.id);
    if (ids.join() !== this.#state.adapters.join()) this.#update({ adapters: ids });
    if (!this.#session) {
      const preferred = this.#state.settings.adapter;
      const port = ports.find((p) => p.id === preferred) ?? ports[0];
      if (port) await this.#connect(port);
    }
  }

  async #connect(port: PortInfo) {
    this.#update({ connection: "connecting" });
    let stream: ByteStream;
    try {
      stream = await this.#ports.open(port.path);
    } catch {
      this.#update({ connection: "searching" });
      return;
    }
    const session = new Session(
      stream,
      (event) => this.#event(session, event),
      () => this.#closed(session),
    );
    this.#session = session;
    for (let attempt = 1; attempt <= STATUS_ATTEMPTS; attempt++) {
      try {
        const status = await session.status();
        if (this.#session !== session) {
          // Replaced while connecting, such as by choosing another adapter.
          await session.close();
          return;
        }
        this.#update({ connection: "connected", adapter: adapterStatus(status) });
        await this.#sendBattery();
        return;
      } catch (error) {
        if (!(error instanceof SessionError && error.timeout)) break;
      }
    }
    // Not an adapter that answers: try again on the next scan.
    if (this.#session === session) {
      this.#session = null;
      await session.close();
      this.#update({ connection: "searching", adapter: null });
    }
  }

  #closed(session: Session) {
    if (this.#session !== session) return;
    this.#session = null;
    this.#update({ connection: "searching", adapter: null, passkeyRequested: false });
    this.#schedule();
  }

  #event(session: Session, event: Event) {
    if (this.#session !== session) return;
    switch (event.kind.case) {
      case "status":
        this.#update({ adapter: adapterStatus(event.kind.value) });
        break;
      case "passkeyRequest":
        this.#update({ passkeyRequested: true });
        break;
      case "pairingResult":
        this.#update({
          passkeyRequested: false,
          pairingResult: event.kind.value.success ? "succeeded" : "failed",
        });
        break;
    }
  }

  /** Sends `command`. A failure is described with `what`, or with `notReady` when given and the
   * adapter answers ERROR_CODE_NOT_READY. */
  async #request(command: Command, what: string, notReady?: string): Promise<ActionResult> {
    const session = this.#session;
    if (!session || this.#state.connection !== "connected") {
      return { ok: false, message: "No adapter is connected. Plug in an adapter and try again." };
    }
    try {
      const response = await session.request(command);
      if (response.result.case === "status" && this.#session === session) {
        this.#update({ adapter: adapterStatus(response.result.value) });
      }
      return { ok: true };
    } catch (error) {
      if (notReady && error instanceof AdapterError && error.code === ErrorCode.NOT_READY) {
        return { ok: false, message: notReady };
      }
      return { ok: false, message: describe(error, what) };
    }
  }

  /** Releases every key and button on the Target, for when the window can no longer do it. */
  releaseInput() {
    const session = this.#session;
    // A port that has not answered as an adapter yet gets nothing.
    if (!session || this.#state.connection !== "connected") return;
    session.input({ case: "keyboard", value: { modifiers: 0, keys: new Uint8Array() } });
    session.input({ case: "consumer", value: { usages: [] } });
    session.input({ case: "mouse", value: { buttons: 0 } });
  }

  async #sendBattery() {
    const level = this.#state.settings.shareBattery && this.#battery !== null ? this.#battery : 100;
    await this.#request({ case: "setBattery", value: { level } }, "update the battery level");
  }

  /** Applies new settings; a different preferred adapter reconnects. */
  async setSettings(settings: Settings) {
    const previous = this.#state.settings;
    this.#update({ settings });
    if (previous.shareBattery !== settings.shareBattery) await this.#sendBattery();
    if (settings.adapter !== previous.adapter && settings.adapter && this.#state.adapter?.id !== settings.adapter) {
      const session = this.#session;
      this.#session = null;
      await session?.close();
      this.#update({ connection: "searching", adapter: null, passkeyRequested: false });
      void this.#scan();
    }
  }

  async act(action: Exclude<Action, { type: "settings" | "clipboard" | "adapter.select" | "capturing" }>): Promise<ActionResult> {
    switch (action.type) {
      case "config": {
        const { name, keyboard, mouse, security } = action.change;
        return this.#request({
          case: "setConfig",
          value: {
            name,
            keyboard,
            mouse,
            security: security === undefined ? undefined : security === "passkey" ? Security.PASSKEY : Security.JUST_WORKS,
          },
        }, "save the change", "The adapter isn't ready.");
      }
      case "pairing":
        if (action.enabled) this.#update({ pairingResult: null });
        return action.enabled
          ? this.#request({ case: "startPairing", value: {} }, "start pairing")
          : this.#request({ case: "stopPairing", value: {} }, "stop pairing");
      case "passkey": {
        this.#update({ passkeyRequested: false });
        return this.#request(
          { case: "enterPasskey", value: { passkey: action.passkey ?? undefined } },
          action.passkey === null ? "cancel pairing" : "send the passkey",
        );
      }
      case "disconnect":
        return this.#request({ case: "disconnect", value: {} }, "disconnect this device");
      case "forget": {
        const address = parseAddress(action.target.address);
        if (!address) return { ok: false, message: "The adapter couldn't forget this device. This device is no longer available." };
        return this.#request(
          { case: "forgetTarget", value: { address, addressType: action.target.random ? 1 : 0 } },
          "forget this device",
        );
      }
      case "battery":
        if (!Number.isFinite(action.level)) return { ok: false, message: "The battery level is not a number." };
        this.#battery = Math.max(0, Math.min(100, Math.round(action.level)));
        await this.#sendBattery();
        return { ok: true };
      case "restart":
        return this.#request({ case: "restart", value: {} }, "restart");
      case "bootloader":
        return this.#request({ case: "enterBootloader", value: {} }, "restart into its bootloader");
    }
  }

  input(message: InputMessage) {
    const session = this.#session;
    if (!session) return;
    switch (message.k) {
      case "kb":
        session.input({ case: "keyboard", value: { modifiers: message.m & 0xff, keys: Uint8Array.from(message.keys.slice(0, 16)) } });
        break;
      case "cc":
        session.input({ case: "consumer", value: { usages: message.u.slice(0, 8) } });
        break;
      case "ms":
        session.input({
          case: "mouse",
          value: { buttons: message.b & 0x1f, x: message.x | 0, y: message.y | 0, wheel: message.w | 0, pan: message.p | 0 },
        });
        break;
    }
  }
}
