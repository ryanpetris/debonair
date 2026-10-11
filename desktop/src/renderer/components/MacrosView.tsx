// Key combinations: buttons and shortcuts that send keys this computer would keep for itself.
import { useState } from "react";
import type { Capture } from "../../shared/capture.ts";
import { KEYS, keyLabel } from "../../shared/keys.ts";
import { DEFAULT_MACROS, shortcutLabel, type Macro, type MacroStep } from "../../shared/settings.ts";
import type { AppState } from "../../shared/state.ts";
import { api } from "../api.ts";
import { Dialog, Keys, KeysRecorder, Row, ShortcutInput, Switch } from "./common.tsx";

function summary(step: MacroStep) {
  if (step.kind === "keys") return step.codes.map(keyLabel).join("+");
  if (step.kind === "text") return `"${step.text.length > 24 ? `${step.text.slice(0, 24)}...` : step.text}"`;
  return `${step.ms} ms`;
}

export function MacrosView(props: { state: AppState; capture: Capture }) {
  const { state, capture } = props;
  const macros = state.settings.macros;
  const [editing, setEditing] = useState<Macro | null>(null);
  const [restoring, setRestoring] = useState(false);
  const save = (next: Macro[]) => void api().act({ type: "settings", settings: { ...state.settings, macros: next } });
  const connected = state.adapter?.link === "connected";

  return (
    <div className="page">
      <div className="page-actions">
        <button
          type="button"
          className="suggested"
          onClick={() =>
            setEditing({ id: crypto.randomUUID(), name: "", steps: [{ kind: "keys", codes: [] }], shortcut: null, button: true })
          }
        >
          New Key Combination
        </button>
        <button type="button" onClick={() => setRestoring(true)}>
          Restore Defaults
        </button>
      </div>
      {macros.length ? (
        <section className="section">
          <h2>Key Combinations</h2>
          <div className="card">
            {macros.map((macro) => (
              <Row
                key={macro.id}
                title={macro.name}
                subtitle={[
                  macro.steps.map(summary).join(", ") === macro.name ? "" : macro.steps.map(summary).join(", "),
                  macro.shortcut ? `Shortcut ${shortcutLabel(macro.shortcut, keyLabel)}` : "",
                ]
                  .filter(Boolean)
                  .join(" · ")}
              >
                <div className="row-actions">
                  <button type="button" disabled={!connected} onClick={() => void capture.runMacro(macro)}>
                    Send
                  </button>
                  <button type="button" className="flat" onClick={() => setEditing(structuredClone(macro))}>
                    Edit
                  </button>
                  <button type="button" className="flat destructive" onClick={() => save(macros.filter((m) => m.id !== macro.id))}>
                    Delete
                  </button>
                </div>
              </Row>
            ))}
          </div>
        </section>
      ) : null}
      {editing ? (
        <MacroEditor
          macro={editing}
          onCancel={() => setEditing(null)}
          onSave={(macro) => {
            const exists = macros.some((m) => m.id === macro.id);
            save(exists ? macros.map((m) => (m.id === macro.id ? macro : m)) : [...macros, macro]);
            setEditing(null);
          }}
        />
      ) : null}
      {restoring ? (
        <Dialog title="Restore Default Key Combinations?" onClose={() => setRestoring(false)}>
          <div className="dialog-actions">
            <button type="button" onClick={() => setRestoring(false)}>
              Cancel
            </button>
            <button
              type="button"
              className="suggested"
              onClick={() => {
                save(structuredClone(DEFAULT_MACROS));
                setRestoring(false);
              }}
            >
              Restore
            </button>
          </div>
        </Dialog>
      ) : null}
    </div>
  );
}

const KEY_CHOICES = Object.entries(KEYS)
  .map(([code, usage]) => ({ code, label: usage.label }))
  .sort((a, b) => a.label.localeCompare(b.label, undefined, { numeric: true }));

function StepEditor(props: { step: MacroStep; onChange(step: MacroStep): void; onRemove(): void }) {
  const { step } = props;
  return (
    <div className="step">
      <select
        value={step.kind}
        onChange={(e) => {
          const kind = e.target.value as MacroStep["kind"];
          props.onChange(kind === "keys" ? { kind, codes: [] } : kind === "text" ? { kind, text: "" } : { kind, ms: 100 });
        }}
      >
        <option value="keys">Keys</option>
        <option value="text">Text</option>
        <option value="delay">Delay</option>
      </select>
      {step.kind === "keys" ? (
        <div className="row-actions">
          <Keys codes={step.codes} />
          <KeysRecorder onRecord={(codes) => props.onChange({ kind: "keys", codes })} />
          <select
            value=""
            aria-label="Add Key"
            onChange={(e) => e.target.value && props.onChange({ kind: "keys", codes: [...step.codes, e.target.value] })}
          >
            <option value="">Add Key</option>
            {KEY_CHOICES.map((k) => (
              <option key={k.code} value={k.code}>
                {k.label}
              </option>
            ))}
          </select>
        </div>
      ) : step.kind === "text" ? (
        <input type="text" value={step.text} onChange={(e) => props.onChange({ kind: "text", text: e.target.value })} />
      ) : (
        <div className="row-actions">
          <input
            type="number"
            min={0}
            max={10000}
            step={10}
            value={step.ms}
            onChange={(e) => props.onChange({ kind: "delay", ms: Math.max(0, Math.min(10000, Number(e.target.value) || 0)) })}
          />
          <span className="row-value">ms</span>
        </div>
      )}
      <button type="button" className="flat" onClick={props.onRemove}>
        Remove
      </button>
    </div>
  );
}

function MacroEditor(props: { macro: Macro; onSave(macro: Macro): void; onCancel(): void }) {
  const [macro, setMacro] = useState(props.macro);
  const steps = macro.steps.filter((s) => (s.kind === "keys" ? s.codes.length : s.kind === "text" ? s.text.length : true));
  const valid = macro.name.trim() !== "" && steps.length > 0;
  return (
    <Dialog title={props.macro.name ? "Edit Key Combination" : "New Key Combination"} onClose={props.onCancel}>
      <label className="field">
        <span>Name</span>
        <input type="text" autoFocus value={macro.name} maxLength={40} onChange={(e) => setMacro({ ...macro, name: e.target.value })} />
      </label>
      <div className="field">
        <span>Steps</span>
        {macro.steps.map((step, index) => (
          <StepEditor
            key={index}
            step={step}
            onChange={(next) => setMacro({ ...macro, steps: macro.steps.map((s, i) => (i === index ? next : s)) })}
            onRemove={() => setMacro({ ...macro, steps: macro.steps.filter((_, i) => i !== index) })}
          />
        ))}
        <div>
          <button type="button" onClick={() => setMacro({ ...macro, steps: [...macro.steps, { kind: "keys", codes: [] }] })}>
            Add Step
          </button>
        </div>
      </div>
      <div className="field">
        <span>Shortcut</span>
        <ShortcutInput optional value={macro.shortcut} onChange={(shortcut) => setMacro({ ...macro, shortcut })} />
      </div>
      <Row title="Show as Button">
        <Switch label="Show as Button" on={macro.button} onChange={(button) => setMacro({ ...macro, button })} />
      </Row>
      <div className="dialog-actions">
        <button type="button" onClick={props.onCancel}>
          Cancel
        </button>
        <button
          type="button"
          className="suggested"
          disabled={!valid}
          onClick={() => props.onSave({ ...macro, name: macro.name.trim(), steps })}
        >
          Save
        </button>
      </div>
    </Dialog>
  );
}
