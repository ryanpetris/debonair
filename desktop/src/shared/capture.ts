// Turns this computer's key, button, pointer and wheel events into the input the adapter sends to
// the Target. It knows nothing about the DOM, so the window feeds it events and it calls a sink.
import { isModifier, keyForCharacter, keyUsage, modifierBit, modifierKind, type Modifier } from "./keys.ts";
import type { Macro, MacroStep, Settings, Shortcut } from "./settings.ts";

/** Where captured input goes. Keyboard and consumer calls carry complete state. */
export interface InputSink {
  keyboard(modifiers: number, keys: number[]): void;
  consumer(usages: number[]): void;
  mouse(buttons: number, x: number, y: number, wheel: number, pan: number): void;
}

export interface Clock {
  setTimeout(callback: () => void, ms: number): unknown;
  clearTimeout(handle: unknown): void;
}

export type Platform = "mac" | "windows" | "other";

/** Which modifiers an event reports as held. */
export interface ModifierState {
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  meta: boolean;
  /** AltGr, which Linux reports here rather than as Alt. */
  altGraph?: boolean;
}

export interface CaptureOptions {
  /**
   * macOS reports Caps Lock as down while it is on and up when it turns off, and sends no release
   * for a key pressed while Command is held. Windows reports AltGr as Left Ctrl and Right Alt.
   */
  platform: Platform;
  /** Wheel pixels that make one detent on this computer. */
  pixelsPerDetent: number;
  /** Called when the release shortcut fires. */
  onRelease(): void;
  /** Called when a macro's shortcut fires. */
  onMacro(macro: Macro): void;
}

/** Pointer motion is collected for this long before it is sent, since a Target receives at most
 * one report per Bluetooth connection interval of 7.5 ms or more. */
export const MOTION_MS = 4;
/** How long a macro holds its keys, and the pause after releasing them. */
export const HOLD_MS = 40;
export const GAP_MS = 20;
/** The pause between typed key changes, slow enough for any connection interval. */
export const TYPE_MS = 16;
/** How long a pinch holds Ctrl after its last scroll. */
export const PINCH_MS = 250;
/** How long a Left Ctrl press on Windows waits to see whether it is part of AltGr. */
export const ALTGR_MS = 10;

const HIGH_RESOLUTION = 120;
type Axis = "x" | "y" | "wheel" | "pan";
/** DOM mouse button numbers to HID button bits: primary, secondary, middle, back, forward. */
const BUTTON_BITS = [1, 4, 2, 8, 16];

export class Capture {
  readonly #sink: InputSink;
  readonly #clock: Clock;
  readonly #options: CaptureOptions;
  #settings: Settings;
  /** The Target's Caps Lock, which typed text corrects for. */
  capsLock = false;
  /**
   * Holds Esc back until it is released or another key is pressed, as while the keyboard is
   * locked in full screen, where holding Esc leaves full screen.
   */
  holdEscape = false;
  /** Keys held on this computer, in press order. */
  #held: string[] = [];
  /** Held keys that belong to a shortcut and are not sent. */
  #consumed = new Set<string>();
  /** The modifier-only shortcut that fires when its modifiers are released, if still armed. */
  #armed: Shortcut | null = null;
  #buttons = 0;
  #macro: { keyboard: string[]; cancelled: boolean } | null = null;
  #pinch = false;
  #pinchTimer: unknown = null;
  #motion = { x: 0, y: 0, wheel: 0, pan: 0 };
  #remainder = { x: 0, y: 0, wheel: 0, pan: 0 };
  #motionTimer: unknown = null;
  /** The Target starts with nothing held. */
  #sentKeyboard = "0:";
  #sentConsumer = "";
  /** A Windows Left Ctrl press waiting to see whether Right Alt follows: its event time. */
  #pendingCtrl: number | null = null;
  #pendingCtrlTimer: unknown = null;
  /** The release of the Left Ctrl that Windows adds to AltGr is ignored. */
  #ignoreCtrlUp = false;
  #pendingEscape = false;
  /** Keys tapped on macOS while Command was held, whose release may still arrive. */
  #tapped = new Set<string>();

  constructor(sink: InputSink, clock: Clock, settings: Settings, options: CaptureOptions) {
    this.#sink = sink;
    this.#clock = clock;
    this.#settings = settings;
    this.#options = options;
  }

  set settings(settings: Settings) {
    this.#settings = settings;
  }

  get macroRunning() {
    return this.#macro !== null;
  }

  // ---- Keyboard ----------------------------------------------------------------------------

  /** A key press. `time` is the event's time stamp in milliseconds. */
  keyDown(code: string, repeat = false, time = 0, state?: ModifierState) {
    if (!keyUsage(code)) return;
    const mac = this.#options.platform === "mac";
    if (this.#pendingCtrl !== null) {
      const pending = this.#pendingCtrl;
      this.#clearPendingCtrl();
      if (code === "AltRight" && Math.abs(time - pending) <= 1) this.#ignoreCtrlUp = true;
      else this.#press("ControlLeft");
    }
    if (state) this.sync(state);
    if (repeat) {
      // macOS repeats a key held with Command without ever releasing it.
      // A shortcut's key was never sent, so it is not repeated either.
      if (this.#tapped.has(code) && this.#heldModifiers().has("meta") && !this.#keyedShortcut(code)) {
        this.#tap(code);
      }
      return;
    }
    // A key pressed again was released, even if its release never arrived.
    this.#tapped.delete(code);
    if (this.#pinch) this.#endPinch();
    if (this.#pendingEscape) {
      // Esc held together with another key is a real press.
      this.#pendingEscape = false;
      this.#press("Escape");
    }
    if (this.#options.platform === "windows" && code === "ControlLeft" && !this.#held.includes(code)) {
      this.#pendingCtrl = time;
      this.#pendingCtrlTimer = this.#clock.setTimeout(() => this.#flushPendingCtrl(), ALTGR_MS);
      return;
    }
    if (code === "CapsLock" && mac) return this.#tap(code);
    const command = mac && !isModifier(code) && this.#heldModifiers().has("meta");
    if (code === "Escape" && this.holdEscape && !command && !this.#held.includes(code)) {
      this.#pendingEscape = true;
      return;
    }
    this.#press(code);
    if (command) {
      // macOS sends no release for this key, so release it now.
      this.#tapped.add(code);
      this.#release(code);
    }
  }

  /** A key release. `state` is the modifiers the event reports as still held. */
  keyUp(code: string, state?: ModifierState) {
    if (!keyUsage(code)) return;
    if (code === "ControlLeft") {
      if (this.#ignoreCtrlUp) {
        this.#ignoreCtrlUp = false;
        return;
      }
      this.#flushPendingCtrl();
    }
    if (code === "Escape" && this.#pendingEscape) {
      this.#pendingEscape = false;
      this.#press(code);
      this.#release(code);
    } else if (code === "CapsLock" && this.#options.platform === "mac") {
      this.#tap(code);
    } else if (this.#held.includes(code)) {
      this.#release(code);
    } else if (this.#tapped.delete(code)) {
      // Already released.
    } else if (!isModifier(code)) {
      // Some keys, such as Print Screen on Windows, report only their release.
      this.#tap(code);
    }
    // Elsewhere, a key's own release clears its modifier flag even while the other side is held.
    // Windows never delivers the first release when both sides are held, so there the event's
    // state is trusted for both sides.
    if (state) this.sync(state, this.#options.platform === "windows" ? null : modifierKind(code));
  }

  /**
   * Releases held modifiers that an event reports as up, for releases the system never
   * delivered, such as the first Shift when both are held on Windows.
   */
  sync(state: ModifierState, except: Modifier | null = null) {
    for (const code of [...this.#held]) {
      const kind = modifierKind(code);
      if (!kind || kind === except || state[kind]) continue;
      if (code === "AltRight" && state.altGraph) continue;
      this.#release(code);
    }
  }

  #press(code: string) {
    if (this.#held.includes(code)) return;
    this.#held.push(code);
    const shortcut = this.#keyedShortcut(code);
    if (shortcut) {
      this.#consumed.add(code);
      this.#armed = null;
      this.#fire(shortcut);
      return;
    }
    // A modifier-only shortcut arms when exactly its modifiers are held. Any other key disarms it
    // until its modifiers are released, so they still work in combinations.
    this.#armed = isModifier(code) ? this.#modifierShortcut() : null;
    this.#sendKeys();
  }

  #release(code: string) {
    const index = this.#held.indexOf(code);
    if (index < 0) return;
    const armed = this.#armed;
    this.#held.splice(index, 1);
    this.#consumed.delete(code);
    this.#sendKeys();
    this.#armed = null;
    const kind = modifierKind(code);
    if (armed && kind && armed.modifiers.includes(kind)) this.#fire(armed);
  }

  #tap(code: string) {
    this.#held.push(code);
    this.#sendKeys();
    this.#held.splice(this.#held.indexOf(code), 1);
    this.#sendKeys();
  }

  #clearPendingCtrl() {
    if (this.#pendingCtrlTimer !== null) this.#clock.clearTimeout(this.#pendingCtrlTimer);
    this.#pendingCtrlTimer = null;
    this.#pendingCtrl = null;
  }

  /** Sends a waiting Left Ctrl press: no Right Alt followed, so it is a real Ctrl. */
  #flushPendingCtrl() {
    if (this.#pendingCtrl === null) return;
    this.#clearPendingCtrl();
    this.#press("ControlLeft");
  }

  /** The held modifier kinds, other than `except`. */
  #heldModifiers(except?: string): Set<Modifier> {
    const kinds = new Set<Modifier>();
    for (const code of this.#held) {
      const kind = code === except ? null : modifierKind(code);
      if (kind) kinds.add(kind);
    }
    return kinds;
  }

  #shortcuts(): { shortcut: Shortcut; macro: Macro | null }[] {
    return [
      { shortcut: this.#settings.releaseShortcut, macro: null },
      ...this.#settings.macros.flatMap((macro) => (macro.shortcut ? [{ shortcut: macro.shortcut, macro }] : [])),
    ];
  }

  #keyedShortcut(code: string): Shortcut | null {
    if (isModifier(code)) return null;
    const held = this.#heldModifiers(code);
    for (const { shortcut } of this.#shortcuts()) {
      if (shortcut.code === code && sameModifiers(held, shortcut.modifiers)) return shortcut;
    }
    return null;
  }

  #modifierShortcut(): Shortcut | null {
    if (this.#held.some((code) => !isModifier(code))) return null;
    const held = this.#heldModifiers();
    return this.#shortcuts().find(({ shortcut }) => shortcut.code === null && sameModifiers(held, shortcut.modifiers))?.shortcut ?? null;
  }

  #fire(shortcut: Shortcut) {
    const entry = this.#shortcuts().find((s) => s.shortcut === shortcut);
    if (!entry) return;
    if (entry.macro) this.#options.onMacro(entry.macro);
    else this.#options.onRelease();
  }

  /** Sends held keys, or the running macro's keys, if they changed. */
  #sendKeys() {
    const codes = this.#macro ? this.#macro.keyboard : this.#held.filter((code) => !this.#consumed.has(code));
    let modifiers = 0;
    const keys: number[] = [];
    const consumer: number[] = [];
    for (const code of codes) {
      const usage = keyUsage(code);
      if (!usage) continue;
      const bit = modifierBit(code);
      if (bit) modifiers |= bit;
      else if (usage.page === "keyboard") keys.push(usage.usage);
      else consumer.push(usage.usage);
    }
    if (this.#pinch) modifiers |= 1;
    const keyboard = `${modifiers}:${keys.join(",")}`;
    const media = consumer.join(",");
    if (keyboard !== this.#sentKeyboard) {
      this.#flushMotion();
      this.#sentKeyboard = keyboard;
      this.#sink.keyboard(modifiers, keys);
    }
    if (media !== this.#sentConsumer) {
      this.#flushMotion();
      this.#sentConsumer = media;
      this.#sink.consumer(consumer);
    }
  }

  // ---- Mouse --------------------------------------------------------------------------------

  buttonDown(button: number) {
    this.#setButton(button, true);
  }

  buttonUp(button: number) {
    this.#setButton(button, false);
  }

  #setButton(button: number, down: boolean) {
    const bit = BUTTON_BITS[button];
    if (bit === undefined) return;
    if (down) {
      this.#flushPendingCtrl();
      // A click with the release shortcut's modifiers held is a combination, not the shortcut.
      this.#armed = null;
      if (this.#pinch) this.#endPinch();
    }
    const buttons = down ? this.#buttons | bit : this.#buttons & ~bit;
    if (buttons === this.#buttons) return;
    this.#flushMotion();
    this.#buttons = buttons;
    this.#sink.mouse(buttons, 0, 0, 0, 0);
  }

  /** Pointer motion in this computer's pixels. */
  move(dx: number, dy: number) {
    const speed = this.#settings.pointerSpeed;
    this.#accumulate("x", dx * speed);
    this.#accumulate("y", dy * speed);
    this.#scheduleMotion();
  }

  /** A DOM wheel event. Mode 0 is pixels, 1 is lines and 2 is pages. */
  wheel(dx: number, dy: number, mode: number, ctrlKey: boolean) {
    this.#flushPendingCtrl();
    const unit = mode === 1 ? HIGH_RESOLUTION / 3 : mode === 2 ? HIGH_RESOLUTION * 3 : HIGH_RESOLUTION / this.#options.pixelsPerDetent;
    const direction = this.#settings.reverseScroll ? -1 : 1;
    const scale = unit * this.#settings.scrollSpeed * direction;
    // A trackpad pinch arrives as a wheel event with Ctrl set while Ctrl is not held.
    const pinch = ctrlKey && this.#settings.pinchZoom && !this.#heldModifiers().has("ctrl");
    if (pinch) this.#startPinch();
    this.#accumulate("wheel", -dy * scale);
    this.#accumulate("pan", dx * scale);
    if (pinch) this.#flushMotion();
    else this.#scheduleMotion();
  }

  #startPinch() {
    if (this.#pinchTimer !== null) this.#clock.clearTimeout(this.#pinchTimer);
    this.#pinchTimer = this.#clock.setTimeout(() => this.#endPinch(), PINCH_MS);
    if (!this.#pinch) {
      this.#pinch = true;
      this.#sendKeys();
    }
  }

  /** Releases a pinch's Ctrl, before the next key or click so neither gets Ctrl added. */
  #endPinch() {
    if (this.#pinchTimer !== null) this.#clock.clearTimeout(this.#pinchTimer);
    this.#pinchTimer = null;
    if (!this.#pinch) return;
    this.#pinch = false;
    this.#sendKeys();
  }

  #accumulate(axis: Axis, value: number) {
    const total = this.#remainder[axis] + value;
    const whole = Math.trunc(total);
    this.#remainder[axis] = total - whole;
    this.#motion[axis] += whole;
  }

  #scheduleMotion() {
    if (this.#motionTimer !== null) return;
    this.#motionTimer = this.#clock.setTimeout(() => {
      this.#motionTimer = null;
      this.#flushMotion();
    }, MOTION_MS);
  }

  #flushMotion() {
    if (this.#motionTimer !== null) {
      this.#clock.clearTimeout(this.#motionTimer);
      this.#motionTimer = null;
    }
    const { x, y, wheel, pan } = this.#motion;
    if (x === 0 && y === 0 && wheel === 0 && pan === 0) return;
    this.#motion = { x: 0, y: 0, wheel: 0, pan: 0 };
    this.#sink.mouse(this.#buttons, x, y, wheel, pan);
  }

  // ---- Release and macros ------------------------------------------------------------------

  /** Releases everything held, as when capture stops or the window loses focus. */
  releaseAll() {
    if (this.#macro) this.#macro.cancelled = true;
    this.#macro = null;
    this.#held = [];
    this.#consumed.clear();
    this.#armed = null;
    this.#clearPendingCtrl();
    this.#ignoreCtrlUp = false;
    this.#pendingEscape = false;
    this.#tapped.clear();
    if (this.#pinchTimer !== null) this.#clock.clearTimeout(this.#pinchTimer);
    this.#pinchTimer = null;
    this.#pinch = false;
    this.#flushMotion();
    this.#remainder = { x: 0, y: 0, wheel: 0, pan: 0 };
    this.#sendKeys();
    if (this.#buttons) {
      this.#buttons = 0;
      this.#sink.mouse(0, 0, 0, 0, 0);
    }
  }

  /** Stops a running macro and sends the keys held on this computer again. */
  cancelMacro() {
    if (!this.#macro) return;
    this.#macro.cancelled = true;
    this.#macro = null;
    this.#sendKeys();
  }

  /**
   * Sends a macro's steps. Keys held on this computer are not sent while it runs, and are sent
   * again afterwards. A macro started while another runs replaces it.
   */
  async runMacro(macro: Macro) {
    if (this.#macro) this.#macro.cancelled = true;
    const run = { keyboard: [] as string[], cancelled: false };
    this.#macro = run;
    this.#sendKeys();
    for (const step of macro.steps) {
      if (run.cancelled) return;
      await this.#step(run, step);
    }
    if (!run.cancelled && this.#macro === run) {
      this.#macro = null;
      this.#sendKeys();
    }
  }

  async #step(run: { keyboard: string[]; cancelled: boolean }, step: MacroStep) {
    const set = (codes: string[]) => {
      if (run.cancelled) return;
      run.keyboard = codes;
      this.#sendKeys();
    };
    if (step.kind === "delay") return this.#sleep(step.ms);
    if (step.kind === "keys") {
      for (let i = 1; i <= step.codes.length; i++) set(step.codes.slice(0, i));
      await this.#sleep(HOLD_MS);
      for (let i = step.codes.length - 1; i >= 0; i--) set(step.codes.slice(0, i));
      return this.#sleep(GAP_MS);
    }
    for (const character of step.text) {
      const found = keyForCharacter(character);
      if (!found) continue;
      // Caps Lock on the Target inverts Shift for letters.
      const letter = /^[a-z]$/i.test(character);
      const key = { code: found.code, shift: letter && this.capsLock ? !found.shift : found.shift };
      const down = key.shift ? ["ShiftLeft", key.code] : [key.code];
      if (key.shift) {
        set(["ShiftLeft"]);
        await this.#sleep(TYPE_MS);
      }
      set(down);
      await this.#sleep(TYPE_MS);
      set(key.shift ? ["ShiftLeft"] : []);
      await this.#sleep(TYPE_MS);
      if (key.shift) {
        set([]);
        await this.#sleep(TYPE_MS);
      }
      if (run.cancelled) return;
    }
  }

  #sleep(ms: number) {
    return new Promise<void>((resolve) => this.#clock.setTimeout(resolve, ms));
  }
}

function sameModifiers(held: Set<Modifier>, wanted: Modifier[]) {
  return held.size === wanted.length && wanted.every((m) => held.has(m));
}
