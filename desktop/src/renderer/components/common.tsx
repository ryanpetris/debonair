import { useEffect, useRef, useState, type ReactNode } from "react";
import { keyLabel, modifierKind, isModifier, type Modifier } from "../../shared/keys.ts";
import { shortcutLabel, type Shortcut } from "../../shared/settings.ts";

export function Switch(props: { on: boolean; disabled?: boolean; label: string; onChange(on: boolean): void }) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={props.on}
      aria-label={props.label}
      className={`switch${props.on ? " on" : ""}`}
      disabled={props.disabled}
      onClick={() => props.onChange(!props.on)}
    />
  );
}

export function Row(props: { title: ReactNode; subtitle?: ReactNode; children?: ReactNode }) {
  return (
    <div className="row">
      <div className="row-text">
        <span className="row-title">{props.title}</span>
        {props.subtitle ? <span className="row-subtitle">{props.subtitle}</span> : null}
      </div>
      {props.children}
    </div>
  );
}

export function Section(props: { title: string; children: ReactNode }) {
  return (
    <section className="section">
      <h2>{props.title}</h2>
      <div className="card">{props.children}</div>
    </section>
  );
}

export function Dialog(props: { title: string; children: ReactNode; onClose(): void }) {
  useEffect(() => {
    const close = (e: KeyboardEvent) => {
      if (e.key === "Escape") props.onClose();
    };
    window.addEventListener("keydown", close);
    return () => window.removeEventListener("keydown", close);
  }, [props.onClose]);
  return (
    <div className="backdrop" onMouseDown={(e) => e.target === e.currentTarget && props.onClose()}>
      <div className="dialog" role="dialog" aria-label={props.title}>
        <h2>{props.title}</h2>
        {props.children}
      </div>
    </div>
  );
}

export function Feedback(props: { result: { ok: boolean; message: string } | null }) {
  if (!props.result) return null;
  return <p className={`feedback ${props.result.ok ? "success" : "error"}`}>{props.result.message}</p>;
}

export function Keys(props: { codes: string[] }) {
  return (
    <span className="keys">
      {props.codes.map((code, i) => (
        <kbd key={`${code}-${i}`}>{keyLabel(code)}</kbd>
      ))}
    </span>
  );
}

/**
 * Records keys pressed together. Resolves when every key is released, with the keys in press
 * order. Keys the operating system keeps, such as Ctrl+Alt+Delete, never arrive here.
 */
function useRecorder(onDone: (codes: string[]) => void) {
  const [recording, setRecording] = useState(false);
  const done = useRef(onDone);
  done.current = onDone;
  useEffect(() => {
    if (!recording) return;
    const pressed: string[] = [];
    const held = new Set<string>();
    const down = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.repeat) return;
      held.add(e.code);
      if (!pressed.includes(e.code)) pressed.push(e.code);
    };
    const up = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      held.delete(e.code);
      if (held.size === 0 && pressed.length) {
        setRecording(false);
        done.current(pressed);
      }
    };
    const cancel = () => setRecording(false);
    window.addEventListener("keydown", down, true);
    window.addEventListener("keyup", up, true);
    window.addEventListener("blur", cancel);
    return () => {
      window.removeEventListener("keydown", down, true);
      window.removeEventListener("keyup", up, true);
      window.removeEventListener("blur", cancel);
    };
  }, [recording]);
  return [recording, setRecording] as const;
}

/** Records the keys of a macro step. */
export function KeysRecorder(props: { onRecord(codes: string[]): void }) {
  const [recording, setRecording] = useRecorder(props.onRecord);
  return (
    <button type="button" className={recording ? "recording" : ""} onClick={() => setRecording(!recording)}>
      {recording ? "Press Keys" : "Record"}
    </button>
  );
}

/** Turns recorded keys into a shortcut: modifiers plus at most one other key. */
export function shortcutFromCodes(codes: string[]): Shortcut | null {
  const modifiers = [...new Set(codes.map(modifierKind).filter((m): m is Modifier => m !== null))];
  const keys = codes.filter((c) => !isModifier(c));
  if (keys.length > 1) return null;
  const code = keys[0] ?? null;
  if (code === null ? modifiers.length < 2 : modifiers.length === 0) return null;
  return { modifiers, code };
}

export function ShortcutInput(props: { value: Shortcut | null; onChange(value: Shortcut | null): void; optional?: boolean }) {
  const [invalid, setInvalid] = useState(false);
  const [recording, setRecording] = useRecorder((codes) => {
    const shortcut = shortcutFromCodes(codes);
    setInvalid(shortcut === null);
    if (shortcut) props.onChange(shortcut);
  });
  return (
    <div className="row-actions">
      <span className="keys">
        {props.value ? <kbd>{shortcutLabel(props.value, keyLabel)}</kbd> : null}
        {invalid ? <span className="row-subtitle">Those keys can't be a shortcut.</span> : null}
      </span>
      <button type="button" className={recording ? "recording" : ""} onClick={() => setRecording(!recording)}>
        {recording ? "Press Shortcut" : "Change"}
      </button>
      {props.optional && props.value ? (
        <button type="button" className="flat" onClick={() => props.onChange(null)}>
          Clear
        </button>
      ) : null}
    </div>
  );
}
