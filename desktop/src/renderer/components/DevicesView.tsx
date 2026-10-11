// Paired devices: pairing a new one, naming, disconnecting and forgetting.
import { useState } from "react";
import type { AppState, Target } from "../../shared/state.ts";
import { api, perform, type Feedback as Result } from "../api.ts";
import { Dialog, Feedback, Row } from "./common.tsx";

export function targetName(state: AppState, target: Target, index: number) {
  return state.settings.targetNames[target.address] ?? `Device ${index + 1}`;
}

export function DevicesView(props: { state: AppState }) {
  const { state } = props;
  const adapter = state.adapter;
  const [result, setResult] = useState<Result>(null);
  const [renaming, setRenaming] = useState<Target | null>(null);
  const [forgetting, setForgetting] = useState<Target | null>(null);
  if (!adapter) return <div className="page empty">No adapter is connected.</div>;
  const pairing = adapter.link === "pairing";
  const connected = adapter.connected?.address;

  const pairingFeedback: Result =
    state.pairingResult === "succeeded"
      ? { ok: true, message: "This device is paired." }
      : state.pairingResult === "failed"
        ? { ok: false, message: "This device couldn't pair. Try pairing again." }
        : null;

  return (
    <div className="page">
      <div className="page-actions">
        <button
          type="button"
          className={pairing ? "" : "suggested"}
          disabled={!adapter.ready}
          onClick={async () => setResult(await perform({ type: "pairing", enabled: !pairing }, ""))}
        >
          {pairing ? "Stop Pairing" : "Pair New Device"}
        </button>
        {connected ? (
          <button type="button" onClick={async () => setResult(await perform({ type: "disconnect" }, ""))}>
            Disconnect
          </button>
        ) : null}
      </div>
      <Feedback result={result ?? pairingFeedback} />
      {adapter.targets.length ? (
      <section className="section">
        <h2>Paired Devices</h2>
        <div className="card">
          {adapter.targets.map((target, index) => (
            <Row
              key={target.address}
              title={targetName(state, target, index)}
              subtitle={target.address === connected ? "Connected" : "Not Connected"}
            >
              <div className="row-actions">
                <button type="button" className="flat" onClick={() => setRenaming(target)}>
                  Rename
                </button>
                <button type="button" className="flat destructive" onClick={() => setForgetting(target)}>
                  Forget
                </button>
              </div>
            </Row>
          ))}
        </div>
      </section>
      ) : null}
      {renaming ? (
        <RenameDialog state={state} target={renaming} onClose={() => setRenaming(null)} />
      ) : null}
      {forgetting ? (
        <Dialog title="Forget Device?" onClose={() => setForgetting(null)}>
          <div className="dialog-actions">
            <button type="button" onClick={() => setForgetting(null)}>
              Cancel
            </button>
            <button
              type="button"
              className="suggested"
              onClick={async () => {
                const target = forgetting;
                setForgetting(null);
                setResult(await perform({ type: "forget", target }, ""));
              }}
            >
              Forget
            </button>
          </div>
        </Dialog>
      ) : null}
    </div>
  );
}

function RenameDialog(props: { state: AppState; target: Target; onClose(): void }) {
  const { state, target } = props;
  const [name, setName] = useState(state.settings.targetNames[target.address] ?? "");
  const save = async () => {
    const names = { ...state.settings.targetNames };
    if (name.trim()) names[target.address] = name.trim();
    else delete names[target.address];
    await api().act({ type: "settings", settings: { ...state.settings, targetNames: names } });
    props.onClose();
  };
  return (
    <Dialog title="Rename Device" onClose={props.onClose}>
      <label className="field">
        <span>Name</span>
        <input type="text" autoFocus value={name} maxLength={64} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => e.key === "Enter" && void save()} />
      </label>
      <div className="dialog-actions">
        <button type="button" onClick={props.onClose}>
          Cancel
        </button>
        <button type="button" className="suggested" onClick={() => void save()}>
          Save
        </button>
      </div>
    </Dialog>
  );
}

/** Asks for the passkey a device shows while pairing. */
export function PasskeyDialog() {
  const [passkey, setPasskey] = useState("");
  const valid = /^\d{1,6}$/.test(passkey);
  const answer = (value: number | null) => void api().act({ type: "passkey", passkey: value });
  return (
    <Dialog title="Enter the Passkey Shown on the Device" onClose={() => answer(null)}>
      <input
        type="text"
        className="passkey"
        inputMode="numeric"
        autoFocus
        maxLength={6}
        value={passkey}
        onChange={(e) => setPasskey(e.target.value.replace(/\D/g, ""))}
        onKeyDown={(e) => e.key === "Enter" && valid && answer(Number(passkey))}
      />
      <div className="dialog-actions">
        <button type="button" onClick={() => answer(null)}>
          Cancel
        </button>
        <button type="button" className="suggested" disabled={!valid} onClick={() => answer(Number(passkey))}>
          Pair
        </button>
      </div>
    </Dialog>
  );
}
