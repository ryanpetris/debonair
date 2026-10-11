// The capture surface. Clicking it captures the keyboard, and the mouse through pointer lock, and
// sends them to the connected device until the release shortcut, Esc in a window, or a lost focus.
import { useCallback, useEffect, useRef, useState } from "react";
import type { Capture, ModifierState } from "../../shared/capture.ts";
import { keyLabel } from "../../shared/keys.ts";
import { shortcutLabel, type Macro } from "../../shared/settings.ts";
import type { AppState } from "../../shared/state.ts";
import { api } from "../api.ts";

interface KeyboardLock {
  lock(keys?: string[]): Promise<void>;
  unlock(): void;
}
const keyboardLock = () => (navigator as Navigator & { keyboard?: KeyboardLock }).keyboard;
const modifiers = (e: KeyboardEvent | MouseEvent): ModifierState => ({
  ctrl: e.ctrlKey,
  alt: e.altKey,
  shift: e.shiftKey,
  meta: e.metaKey,
  altGraph: e.getModifierState("AltGraph"),
});
/** Typed text longer than this is cut short. */
const TYPE_LIMIT = 4096;

export function CaptureView(props: {
  state: AppState;
  capture: Capture;
  /** Set to this view's stop function, for the capture's release shortcut. */
  release: { current: () => void };
  onCapturing(capturing: boolean): void;
}) {
  const { state, capture } = props;
  const surface = useRef<HTMLDivElement>(null);
  const [capturing, setCapturing] = useState(false);
  const [locked, setLocked] = useState(false);
  const [typing, setTyping] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const adapter = state.adapter;
  const ready = adapter?.link === "connected";
  const mouse = adapter?.mouse ?? false;
  const settings = state.settings;

  const stop = useCallback(() => {
    capture.releaseAll();
    capture.holdEscape = false;
    setCapturing(false);
    setLocked(false);
    if (document.pointerLockElement) document.exitPointerLock();
    keyboardLock()?.unlock();
    if (document.fullscreenElement) void document.exitFullscreen().catch(() => {});
  }, [capture]);

  const start = useCallback(async () => {
    if (capturing || !ready || !surface.current) return;
    setMessage(null);
    (document.activeElement as HTMLElement | null)?.blur();
    if (mouse) {
      try {
        await surface.current.requestPointerLock({ unadjustedMovement: settings.rawPointer });
      } catch {
        try {
          // Unadjusted movement is not available on every system.
          await surface.current.requestPointerLock();
        } catch {
          setMessage("Debonair couldn't capture the pointer. Click to try again.");
          return;
        }
      }
    }
    if (settings.fullScreen) {
      try {
        await document.documentElement.requestFullscreen();
        const lock = keyboardLock();
        if (lock) {
          await lock.lock();
          // Holding Esc leaves full screen, so a single press is sent when it is released.
          capture.holdEscape = true;
          setLocked(true);
        }
      } catch {
        // Capture continues in the window.
      }
    }
    surface.current.focus();
    setCapturing(true);
  }, [capturing, ready, mouse, capture, settings.rawPointer, settings.fullScreen]);

  useEffect(() => props.onCapturing(capturing), [capturing, props.onCapturing]);

  // Release everything if the connection drops while capturing.
  useEffect(() => {
    if (capturing && !ready) stop();
  }, [capturing, ready, stop]);

  useEffect(() => {
    if (!capturing) return;
    const element = surface.current!;
    const key = (down: boolean) => (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (down) capture.keyDown(e.code, e.repeat, e.timeStamp, modifiers(e));
      else capture.keyUp(e.code, modifiers(e));
    };
    const keyDown = key(true);
    const keyUp = key(false);
    const button = (down: boolean) => (e: MouseEvent) => {
      if (!mouse) return;
      e.preventDefault();
      capture.sync(modifiers(e));
      if (down) capture.buttonDown(e.button);
      else capture.buttonUp(e.button);
    };
    const buttonDown = button(true);
    const buttonUp = button(false);
    const move = (e: MouseEvent) => {
      if (mouse && document.pointerLockElement === element) capture.move(e.movementX, e.movementY);
    };
    const wheel = (e: WheelEvent) => {
      e.preventDefault();
      if (mouse) capture.wheel(e.deltaX, e.deltaY, e.deltaMode, e.ctrlKey);
    };
    const prevent = (e: Event) => e.preventDefault();
    const lockChanged = () => {
      if (mouse && document.pointerLockElement !== element) stop();
    };
    const fullscreenChanged = () => {
      if (settings.fullScreen && !document.fullscreenElement) stop();
    };
    // Raw updates arrive at the mouse's own rate rather than once per frame.
    const moveEvent = "onpointerrawupdate" in window ? "pointerrawupdate" : "mousemove";
    window.addEventListener("keydown", keyDown, true);
    window.addEventListener("keyup", keyUp, true);
    document.addEventListener("mousedown", buttonDown, true);
    document.addEventListener("mouseup", buttonUp, true);
    document.addEventListener(moveEvent, move as EventListener, true);
    document.addEventListener("wheel", wheel, { capture: true, passive: false });
    document.addEventListener("contextmenu", prevent, true);
    document.addEventListener("auxclick", prevent, true);
    document.addEventListener("pointerlockchange", lockChanged);
    document.addEventListener("fullscreenchange", fullscreenChanged);
    window.addEventListener("blur", stop);
    return () => {
      window.removeEventListener("keydown", keyDown, true);
      window.removeEventListener("keyup", keyUp, true);
      document.removeEventListener("mousedown", buttonDown, true);
      document.removeEventListener("mouseup", buttonUp, true);
      document.removeEventListener(moveEvent, move as EventListener, true);
      document.removeEventListener("wheel", wheel, { capture: true });
      document.removeEventListener("contextmenu", prevent, true);
      document.removeEventListener("auxclick", prevent, true);
      document.removeEventListener("pointerlockchange", lockChanged);
      document.removeEventListener("fullscreenchange", fullscreenChanged);
      window.removeEventListener("blur", stop);
    };
  }, [capturing, capture, mouse, settings.fullScreen, stop]);

  useEffect(() => {
    props.release.current = stop;
  }, [props.release, stop]);

  const runMacro = (macro: Macro) => void capture.runMacro(macro);
  const typeClipboard = async () => {
    const result = await api().act({ type: "clipboard" });
    if (!result.ok || !result.text) return;
    setTyping(true);
    const text = result.text.slice(0, TYPE_LIMIT);
    await capture.runMacro({ id: "clipboard", name: "Clipboard", steps: [{ kind: "text", text }], shortcut: null, button: false });
    setTyping(false);
  };

  const release = shortcutLabel(settings.releaseShortcut, keyLabel);
  const leds = adapter?.leds ?? 0;
  let title: string;
  let detail: string | null = null;
  if (!adapter) title = state.connection === "connecting" ? "Connecting to the Adapter" : "Plug In an Adapter";
  else if (adapter.link === "off") title = "Starting Bluetooth";
  else if (adapter.link === "pairing") title = "Waiting to Pair";
  else if (adapter.link === "waiting") title = adapter.targets.length ? "Waiting for a Device" : "Pair a Device";
  else if (capturing) {
    title = "Capturing";
    detail = locked ? `Release: ${release}, or hold Esc` : mouse ? `Release: ${release} or Esc` : `Release: ${release}`;
  } else {
    title = "Click to Capture";
    detail = message;
  }

  return (
    <div className="control">
      <div
        ref={surface}
        tabIndex={-1}
        className={`surface ${capturing ? "capturing" : ready ? "idle" : "disabled"}`}
        onClick={() => void start()}
      >
        <span className="surface-title">{title}</span>
        {detail ? <span className="surface-detail">{detail}</span> : null}
        {ready && adapter?.keyboard ? (
          <div className="leds">
            <span className={leds & 1 ? "on" : ""}>Num Lock</span>
            <span className={leds & 2 ? "on" : ""}>Caps Lock</span>
            <span className={leds & 4 ? "on" : ""}>Scroll Lock</span>
          </div>
        ) : null}
      </div>
      {ready && adapter?.keyboard ? (
        <div className="toolbar">
          {settings.macros
            .filter((m) => m.button)
            .map((macro) => (
              <button key={macro.id} type="button" onClick={() => runMacro(macro)}>
                {macro.name}
              </button>
            ))}
          {typing ? (
            <button type="button" onClick={() => capture.cancelMacro()}>
              Stop Typing
            </button>
          ) : (
            <button type="button" onClick={() => void typeClipboard()}>
              Type Clipboard
            </button>
          )}
        </div>
      ) : null}
    </div>
  );
}
