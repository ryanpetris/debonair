import { describe, expect, it } from "vitest";
import { ALTGR_MS, Capture, GAP_MS, HOLD_MS, MOTION_MS, PINCH_MS, TYPE_MS, type InputSink, type Platform } from "../src/shared/capture.ts";
import { DEFAULT_SETTINGS, settingsFrom, type Macro, type Settings } from "../src/shared/settings.ts";

type Sent = ["keyboard", number, number[]] | ["consumer", number[]] | ["mouse", number, number, number, number, number];

/** A clock whose timers run only when the test advances it. */
class FakeClock {
  now = 0;
  #timers: { at: number; callback: () => void; id: number }[] = [];
  #next = 1;
  setTimeout(callback: () => void, ms: number) {
    const id = this.#next++;
    this.#timers.push({ at: this.now + ms, callback, id });
    return id;
  }
  clearTimeout(id: unknown) {
    this.#timers = this.#timers.filter((t) => t.id !== id);
  }
  async advance(ms: number) {
    const end = this.now + ms;
    for (;;) {
      this.#timers.sort((a, b) => a.at - b.at);
      const timer = this.#timers[0];
      if (!timer || timer.at > end) break;
      this.#timers.shift();
      this.now = timer.at;
      timer.callback();
      // Let awaiting macro steps continue before the next timer.
      for (let i = 0; i < 5; i++) await Promise.resolve();
    }
    this.now = end;
  }
}

function setup(settings: Partial<Settings> = {}, platform: Platform = "other") {
  const sent: Sent[] = [];
  const events: string[] = [];
  const clock = new FakeClock();
  const sink: InputSink = {
    keyboard: (modifiers, keys) => sent.push(["keyboard", modifiers, keys]),
    consumer: (usages) => sent.push(["consumer", usages]),
    mouse: (buttons, x, y, wheel, pan) => sent.push(["mouse", buttons, x, y, wheel, pan]),
  };
  const capture = new Capture(sink, clock, { ...structuredClone(DEFAULT_SETTINGS), ...settings }, {
    platform,
    pixelsPerDetent: 100,
    onRelease: () => events.push("release"),
    onMacro: (macro) => events.push(`macro:${macro.id}`),
  });
  return { capture, sent, events, clock };
}

describe("keyboard", () => {
  it("sends complete state with modifiers as bits and keys in press order", () => {
    const { capture, sent } = setup();
    capture.keyDown("ShiftLeft");
    capture.keyDown("KeyB");
    capture.keyDown("KeyA");
    capture.keyDown("KeyA", true);
    capture.keyUp("KeyB");
    capture.keyUp("ShiftLeft");
    capture.keyUp("KeyA");
    expect(sent).toEqual([
      ["keyboard", 2, []],
      ["keyboard", 2, [0x05]],
      ["keyboard", 2, [0x05, 0x04]],
      ["keyboard", 2, [0x04]],
      ["keyboard", 0, [0x04]],
      ["keyboard", 0, []],
    ]);
  });

  it("sends media keys as consumer controls", () => {
    const { capture, sent } = setup();
    capture.keyDown("AudioVolumeUp");
    capture.keyUp("AudioVolumeUp");
    expect(sent).toEqual([
      ["consumer", [0xe9]],
      ["consumer", []],
    ]);
  });

  it("taps keys that report only a release, and toggling Caps Lock", () => {
    const { capture, sent } = setup({}, "mac");
    capture.keyUp("PrintScreen");
    capture.keyDown("CapsLock");
    capture.keyUp("CapsLock");
    expect(sent).toEqual([
      ["keyboard", 0, [0x46]],
      ["keyboard", 0, []],
      ["keyboard", 0, [0x39]],
      ["keyboard", 0, []],
      ["keyboard", 0, [0x39]],
      ["keyboard", 0, []],
    ]);
  });

  it("ignores keys it cannot send", () => {
    const { capture, sent } = setup();
    capture.keyDown("Unidentified");
    capture.keyDown("Fn");
    expect(sent).toEqual([]);
  });
});

describe("shortcuts", () => {
  it("releases on Ctrl+Alt pressed and released alone", () => {
    const { capture, events } = setup();
    capture.keyDown("ControlLeft");
    capture.keyDown("AltLeft");
    capture.keyUp("AltLeft");
    expect(events).toEqual(["release"]);
  });

  it("does not release when Ctrl+Alt is used with another key", () => {
    const { capture, events, sent } = setup();
    capture.keyDown("ControlLeft");
    capture.keyDown("AltLeft");
    capture.keyDown("KeyT");
    capture.keyUp("KeyT");
    capture.keyUp("AltLeft");
    capture.keyUp("ControlLeft");
    expect(events).toEqual([]);
    expect(sent).toContainEqual(["keyboard", 5, [0x17]]);
  });

  it("does not release when another modifier joins", () => {
    const { capture, events } = setup();
    capture.keyDown("ControlLeft");
    capture.keyDown("AltLeft");
    capture.keyDown("ShiftLeft");
    capture.keyUp("ShiftLeft");
    capture.keyUp("AltLeft");
    expect(events).toEqual([]);
  });

  it("consumes a macro shortcut's key and does not release afterwards", () => {
    const { capture, events, sent } = setup();
    capture.keyDown("ControlRight");
    capture.keyDown("AltLeft");
    capture.keyDown("End");
    capture.keyUp("End");
    capture.keyUp("AltLeft");
    capture.keyUp("ControlRight");
    expect(events).toEqual(["macro:ctrl-alt-delete"]);
    expect(sent.some((s) => s[0] === "keyboard" && s[2].includes(0x4d))).toBe(false);
  });
});

describe("macros", () => {
  const ctrlAltDelete = DEFAULT_SETTINGS.macros[0]!;

  it("presses in order, holds, releases in reverse, then restores held keys", async () => {
    const { capture, sent, clock } = setup();
    capture.keyDown("ShiftLeft");
    sent.length = 0;
    const done = capture.runMacro(ctrlAltDelete);
    expect(capture.macroRunning).toBe(true);
    await clock.advance(HOLD_MS + GAP_MS);
    await done;
    expect(sent).toEqual([
      ["keyboard", 0, []],
      ["keyboard", 1, []],
      ["keyboard", 5, []],
      ["keyboard", 5, [0x4c]],
      ["keyboard", 5, []],
      ["keyboard", 1, []],
      ["keyboard", 0, []],
      ["keyboard", 2, []],
    ]);
    expect(capture.macroRunning).toBe(false);
  });

  it("types text with shift where needed", async () => {
    const { capture, sent, clock } = setup();
    const macro: Macro = { id: "t", name: "T", steps: [{ kind: "text", text: "aB" }], shortcut: null, button: true };
    const done = capture.runMacro(macro);
    await clock.advance(TYPE_MS * 10);
    await done;
    expect(sent).toEqual([
      ["keyboard", 0, [0x04]],
      ["keyboard", 0, []],
      ["keyboard", 2, []],
      ["keyboard", 2, [0x05]],
      ["keyboard", 2, []],
      ["keyboard", 0, []],
    ]);
  });

  it("stops when everything is released", async () => {
    const { capture, sent, clock } = setup();
    const done = capture.runMacro(ctrlAltDelete);
    capture.releaseAll();
    await clock.advance(HOLD_MS + GAP_MS);
    await done;
    expect(sent.at(-1)).toEqual(["keyboard", 0, []]);
    expect(capture.macroRunning).toBe(false);
  });
});

describe("pointer", () => {
  it("collects motion briefly and keeps fractions", async () => {
    const { capture, sent, clock } = setup({ pointerSpeed: 0.5 });
    capture.move(3, 1);
    capture.move(3, 1);
    expect(sent).toEqual([]);
    await clock.advance(MOTION_MS);
    expect(sent).toEqual([["mouse", 0, 3, 1, 0, 0]]);
    capture.move(1, 0);
    await clock.advance(MOTION_MS);
    expect(sent.length).toBe(1);
    capture.move(1, 0);
    await clock.advance(MOTION_MS);
    expect(sent.at(-1)).toEqual(["mouse", 0, 1, 0, 0, 0]);
  });

  it("sends pending motion before a button or key change", () => {
    const { capture, sent } = setup();
    capture.move(5, 0);
    capture.buttonDown(0);
    capture.move(0, 2);
    capture.keyDown("ShiftLeft");
    capture.buttonUp(0);
    capture.buttonDown(2);
    capture.buttonDown(1);
    expect(sent).toEqual([
      ["mouse", 0, 5, 0, 0, 0],
      ["mouse", 1, 0, 0, 0, 0],
      ["mouse", 1, 0, 2, 0, 0],
      ["keyboard", 2, []],
      ["mouse", 0, 0, 0, 0, 0],
      ["mouse", 2, 0, 0, 0, 0],
      ["mouse", 6, 0, 0, 0, 0],
    ]);
  });

  it("converts wheel pixels and lines to 1/120 of a detent", async () => {
    const { capture, sent, clock } = setup();
    capture.wheel(0, 100, 0, false);
    capture.wheel(10, 0, 0, false);
    capture.wheel(0, -1, 1, false);
    await clock.advance(MOTION_MS);
    expect(sent).toEqual([["mouse", 0, 0, 0, -120 + 40, 12]]);
  });

  it("reverses scrolling when asked", async () => {
    const { capture, sent, clock } = setup({ reverseScroll: true, scrollSpeed: 2 });
    capture.wheel(0, 50, 0, false);
    await clock.advance(MOTION_MS);
    expect(sent).toEqual([["mouse", 0, 0, 0, 120, 0]]);
  });

  it("sends a pinch as Ctrl with scrolling and releases Ctrl afterwards", async () => {
    const { capture, sent, clock } = setup();
    capture.wheel(0, -10, 0, true);
    capture.wheel(0, -10, 0, true);
    expect(sent).toEqual([
      ["keyboard", 1, []],
      ["mouse", 0, 0, 0, 12, 0],
      ["mouse", 0, 0, 0, 12, 0],
    ]);
    await clock.advance(PINCH_MS);
    expect(sent.at(-1)).toEqual(["keyboard", 0, []]);
  });

  it("leaves Ctrl+scroll alone when Ctrl is really held", async () => {
    const { capture, sent, clock } = setup();
    capture.keyDown("ControlLeft");
    capture.wheel(0, -100, 0, true);
    await clock.advance(PINCH_MS);
    expect(sent).toEqual([
      ["keyboard", 1, []],
      ["mouse", 0, 0, 0, 120, 0],
    ]);
  });

  it("release sends empty state for everything held", () => {
    const { capture, sent } = setup();
    capture.keyDown("KeyA");
    capture.buttonDown(0);
    capture.move(2, 2);
    sent.length = 0;
    capture.releaseAll();
    expect(sent).toEqual([
      ["mouse", 1, 2, 2, 0, 0],
      ["keyboard", 0, []],
      ["mouse", 0, 0, 0, 0, 0],
    ]);
  });
});

describe("settings", () => {
  it("fills defaults and drops invalid values", () => {
    const settings = settingsFrom({
      pointerSpeed: 99,
      scrollSpeed: "fast",
      releaseShortcut: { modifiers: ["ctrl"], code: null },
      macros: [
        { id: "a", name: "Good", steps: [{ kind: "keys", codes: ["KeyA", "Nope"] }] },
        { id: "b", name: "", steps: [{ kind: "keys", codes: ["KeyA"] }] },
        { id: "c", name: "Empty", steps: [] },
      ],
    });
    expect(settings.pointerSpeed).toBe(5);
    expect(settings.scrollSpeed).toBe(1);
    expect(settings.releaseShortcut).toEqual(DEFAULT_SETTINGS.releaseShortcut);
    expect(settings.macros).toEqual([
      { id: "a", name: "Good", steps: [{ kind: "keys", codes: ["KeyA"] }], shortcut: null, button: true },
    ]);
    expect(settingsFrom(null)).toEqual(DEFAULT_SETTINGS);
  });
});

const none = { ctrl: false, alt: false, shift: false, meta: false };

describe("platform quirks", () => {
  it("releases a key pressed with Command on macOS, which reports no release", () => {
    const { capture, sent } = setup({}, "mac");
    capture.keyDown("MetaLeft");
    capture.keyDown("KeyC");
    capture.keyUp("KeyC");
    capture.keyUp("MetaLeft");
    expect(sent).toEqual([
      ["keyboard", 8, []],
      ["keyboard", 8, [0x06]],
      ["keyboard", 8, []],
      ["keyboard", 0, []],
    ]);
  });

  it("drops the Left Ctrl that Windows adds to AltGr", async () => {
    const { capture, sent, events, clock } = setup({}, "windows");
    capture.keyDown("ControlLeft", false, 1000);
    capture.keyDown("AltRight", false, 1000);
    capture.keyDown("KeyQ", false, 1100);
    capture.keyUp("KeyQ");
    capture.keyUp("ControlLeft");
    capture.keyUp("AltRight");
    await clock.advance(ALTGR_MS);
    expect(sent).toEqual([
      ["keyboard", 0x40, []],
      ["keyboard", 0x40, [0x14]],
      ["keyboard", 0x40, []],
      ["keyboard", 0, []],
    ]);
    expect(events).toEqual([]);
  });

  it("keeps a later plain press of a key tapped with Command until its release", () => {
    const { capture, sent } = setup({}, "mac");
    capture.keyDown("MetaLeft");
    capture.keyDown("KeyC");
    capture.keyUp("MetaLeft");
    sent.length = 0;
    capture.keyDown("KeyC");
    capture.keyUp("KeyC");
    expect(sent).toEqual([
      ["keyboard", 0, [0x06]],
      ["keyboard", 0, []],
    ]);
  });

  it("repeats a key held with Command on macOS", () => {
    const { capture, sent } = setup({}, "mac");
    capture.keyDown("MetaLeft");
    capture.keyDown("KeyZ");
    capture.keyDown("KeyZ", true);
    expect(sent.filter((s) => s[0] === "keyboard" && s[2].includes(0x1d)).length).toBe(2);
  });

  it("keeps AltGr held on Linux, which reports it apart from Alt", () => {
    const { capture, sent } = setup();
    const altGraph = { ...none, altGraph: true };
    capture.keyDown("AltRight", false, 0, altGraph);
    capture.keyDown("KeyQ", false, 0, altGraph);
    expect(sent.at(-1)).toEqual(["keyboard", 0x40, [0x14]]);
  });

  it("releases both Shifts on Windows, which reports only the second release", () => {
    const { capture, sent } = setup({}, "windows");
    capture.keyDown("ShiftLeft");
    capture.keyDown("ShiftRight");
    capture.keyUp("ShiftRight", none);
    expect(sent.at(-1)).toEqual(["keyboard", 0, []]);
  });

  it("does not repeat a Command shortcut's key on macOS", () => {
    const { capture, sent, events } = setup(
      { macros: [{ id: "m", name: "M", steps: [{ kind: "keys", codes: ["KeyA"] }], shortcut: { modifiers: ["meta"], code: "KeyZ" }, button: false }] },
      "mac",
    );
    capture.keyDown("MetaLeft");
    capture.keyDown("KeyZ");
    capture.keyDown("KeyZ", true);
    expect(events).toEqual(["macro:m"]);
    expect(sent.some((s) => s[0] === "keyboard" && s[2].includes(0x1d))).toBe(false);
    capture.keyUp("KeyZ");
    capture.keyUp("MetaLeft");
    capture.keyDown("KeyZ");
    expect(sent.at(-1)).toEqual(["keyboard", 0, [0x1d]]);
  });

  it("keeps the other side of a modifier when one side is released", () => {
    const { capture, sent } = setup();
    capture.keyDown("ShiftLeft");
    capture.keyDown("ShiftRight");
    capture.keyUp("ShiftLeft", none);
    expect(sent.at(-1)).toEqual(["keyboard", 0x20, []]);
  });

  it("sends a real Left Ctrl on Windows after a moment", async () => {
    const { capture, sent, clock } = setup({}, "windows");
    capture.keyDown("ControlLeft", false, 1000);
    expect(sent).toEqual([]);
    await clock.advance(ALTGR_MS);
    expect(sent).toEqual([["keyboard", 1, []]]);
    capture.keyDown("KeyC", false, 1300);
    expect(sent.at(-1)).toEqual(["keyboard", 1, [0x06]]);
  });

  it("sends a waiting Left Ctrl before another key", () => {
    const { capture, sent } = setup({}, "windows");
    capture.keyDown("ControlLeft", false, 1000);
    capture.keyDown("KeyC", false, 1003);
    expect(sent).toEqual([
      ["keyboard", 1, []],
      ["keyboard", 1, [0x06]],
    ]);
  });

  it("holds Esc back while the keyboard is locked", () => {
    const { capture, sent } = setup();
    capture.holdEscape = true;
    capture.keyDown("Escape");
    expect(sent).toEqual([]);
    capture.keyUp("Escape");
    expect(sent).toEqual([
      ["keyboard", 0, [0x29]],
      ["keyboard", 0, []],
    ]);
    sent.length = 0;
    capture.keyDown("Escape");
    capture.releaseAll();
    expect(sent).toEqual([]);
  });

  it("releases modifiers an event reports as up", () => {
    const { capture, sent } = setup();
    capture.keyDown("ShiftLeft");
    capture.keyDown("KeyA", false, 0, none);
    expect(sent.at(-1)).toEqual(["keyboard", 0, [0x04]]);
  });

  it("lets a shortcut with Esc fire while Esc is held back", () => {
    const { capture, events } = setup({
      macros: [{ id: "m", name: "M", steps: [{ kind: "keys", codes: ["KeyA"] }], shortcut: { modifiers: ["ctrl", "shift"], code: "Escape" }, button: false }],
    });
    capture.holdEscape = true;
    capture.keyDown("ControlLeft");
    capture.keyDown("ShiftLeft");
    capture.keyDown("Escape");
    capture.keyUp("Escape");
    expect(events).toEqual(["macro:m"]);
  });
});

describe("corrections", () => {
  it("types letters correctly while the Target's Caps Lock is on", async () => {
    const { capture, sent, clock } = setup();
    capture.capsLock = true;
    const done = capture.runMacro({ id: "t", name: "T", steps: [{ kind: "text", text: "a1" }], shortcut: null, button: true });
    await clock.advance(TYPE_MS * 10);
    await done;
    expect(sent.slice(0, 4)).toEqual([
      ["keyboard", 2, []],
      ["keyboard", 2, [0x04]],
      ["keyboard", 2, []],
      ["keyboard", 0, []],
    ]);
    expect(sent.slice(4)).toEqual([
      ["keyboard", 0, [0x1e]],
      ["keyboard", 0, []],
    ]);
  });

  it("ends a pinch before a key or a click", () => {
    const { capture, sent } = setup();
    capture.wheel(0, -10, 0, true);
    capture.buttonDown(0);
    expect(sent).toEqual([
      ["keyboard", 1, []],
      ["mouse", 0, 0, 0, 12, 0],
      ["keyboard", 0, []],
      ["mouse", 1, 0, 0, 0, 0],
    ]);
  });

  it("does not release after Ctrl+Alt+click", () => {
    const { capture, events } = setup();
    capture.keyDown("ControlLeft");
    capture.keyDown("AltLeft");
    capture.buttonDown(0);
    capture.buttonUp(0);
    capture.keyUp("AltLeft");
    expect(events).toEqual([]);
  });

  it("cancels a running macro and restores held keys", async () => {
    const { capture, sent } = setup();
    capture.keyDown("ShiftLeft");
    void capture.runMacro(DEFAULT_SETTINGS.macros[0]!);
    capture.cancelMacro();
    expect(capture.macroRunning).toBe(false);
    expect(sent.at(-1)).toEqual(["keyboard", 2, []]);
  });
});
