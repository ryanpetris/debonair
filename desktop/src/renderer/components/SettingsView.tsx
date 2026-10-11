// Adapter settings saved on the adapter, capture preferences saved by the app, and diagnostics.
import { useEffect, useState } from "react";
import { SPEED_RANGE, type Settings } from "../../shared/settings.ts";
import type { AppState, ConfigChange, Security } from "../../shared/state.ts";
import { api, perform, type Feedback as Result } from "../api.ts";
import { Dialog, Feedback, Row, Section, ShortcutInput, Switch } from "./common.tsx";
import { targetName } from "./DevicesView.tsx";

const NAME_BYTES = 29;

export function SettingsView(props: { state: AppState }) {
  const { state } = props;
  const settings = state.settings;
  const save = (change: Partial<Settings>) => void api().act({ type: "settings", settings: { ...settings, ...change } });
  return (
    <div className="page">
      {state.adapter ? <AdapterSettings state={state} /> : null}
      <Section title="Pointer">
        <Row title="Pointer Speed">
          <Speed value={settings.pointerSpeed} onChange={(pointerSpeed) => save({ pointerSpeed })} />
        </Row>
        <Row title="Raw Pointer Input">
          <Switch label="Raw Pointer Input" on={settings.rawPointer} onChange={(rawPointer) => save({ rawPointer })} />
        </Row>
        <Row title="Scroll Speed">
          <Speed value={settings.scrollSpeed} onChange={(scrollSpeed) => save({ scrollSpeed })} />
        </Row>
        <Row title="Reverse Scrolling">
          <Switch label="Reverse Scrolling" on={settings.reverseScroll} onChange={(reverseScroll) => save({ reverseScroll })} />
        </Row>
        <Row title="Pinch to Zoom">
          <Switch label="Pinch to Zoom" on={settings.pinchZoom} onChange={(pinchZoom) => save({ pinchZoom })} />
        </Row>
      </Section>
      <Section title="Capture">
        <Row title="Release Shortcut">
          <ShortcutInput value={settings.releaseShortcut} onChange={(value) => value && save({ releaseShortcut: value })} />
        </Row>
        <Row title="Full Screen Capture">
          <Switch label="Full Screen Capture" on={settings.fullScreen} onChange={(fullScreen) => save({ fullScreen })} />
        </Row>
        <Row title="Share Battery Level">
          <Switch label="Share Battery Level" on={settings.shareBattery} onChange={(shareBattery) => save({ shareBattery })} />
        </Row>
      </Section>
      {state.adapters.length > 1 ? (
        <Section title="Adapters">
          {state.adapters.map((id, index) => (
            <Row key={id} title={`Adapter ${index + 1}`} subtitle={id === state.adapter?.id ? "Connected" : undefined}>
              {id === state.adapter?.id ? null : (
                <button type="button" onClick={() => void api().act({ type: "adapter.select", id })}>
                  Use
                </button>
              )}
            </Row>
          ))}
        </Section>
      ) : null}
      {state.adapter ? <Diagnostics state={state} /> : null}
      <p className="feedback selectable">Debonair {state.version}</p>
    </div>
  );
}

function Speed(props: { value: number; onChange(value: number): void }) {
  const [value, setValue] = useState(props.value);
  useEffect(() => setValue(props.value), [props.value]);
  return (
    <div className="row-actions">
      <span className="row-value">{value.toFixed(1)}x</span>
      <input
        type="range"
        min={SPEED_RANGE.min}
        max={SPEED_RANGE.max}
        step={0.1}
        value={value}
        onChange={(e) => setValue(Number(e.target.value))}
        onPointerUp={() => props.onChange(value)}
        onKeyUp={() => props.onChange(value)}
      />
    </div>
  );
}

function AdapterSettings(props: { state: AppState }) {
  const adapter = props.state.adapter!;
  const [name, setName] = useState(adapter.name);
  const [result, setResult] = useState<Result>(null);
  const [devices, setDevices] = useState<{ keyboard: boolean; mouse: boolean } | null>(null);
  useEffect(() => setName(adapter.name), [adapter.name]);
  const configure = async (change: ConfigChange, success = "") => setResult(await perform({ type: "config", change }, success));
  const bytes = new TextEncoder().encode(name.trim()).length;
  return (
    <section className="section">
      <h2>Adapter</h2>
      <div className="card">
        <Row title="Name">
          <div className="row-actions">
            <input type="text" value={name} onChange={(e) => setName(e.target.value)} />
            <button
              type="button"
              disabled={bytes === 0 || bytes > NAME_BYTES || name.trim() === adapter.name}
              onClick={() => void configure({ name: name.trim() }, "The adapter's name is saved.")}
            >
              Save
            </button>
          </div>
        </Row>
        <Row title="Keyboard">
          <Switch
            label="Keyboard"
            on={adapter.keyboard}
            disabled={adapter.keyboard && !adapter.mouse}
            onChange={(keyboard) => setDevices({ keyboard, mouse: adapter.mouse })}
          />
        </Row>
        <Row title="Mouse">
          <Switch
            label="Mouse"
            on={adapter.mouse}
            disabled={adapter.mouse && !adapter.keyboard}
            onChange={(mouse) => setDevices({ keyboard: adapter.keyboard, mouse })}
          />
        </Row>
        <Row title="Pairing Security">
          <select
            value={adapter.security}
            onChange={(e) => void configure({ security: e.target.value as Security })}
          >
            <option value="justWorks">No Passkey</option>
            <option value="passkey">Passkey</option>
          </select>
        </Row>
      </div>
      <Feedback result={result} />
      {devices ? (
        <Dialog title="Restart the Adapter?" onClose={() => setDevices(null)}>
          <p>The adapter restarts as a new device. Each device pairs with it again.</p>
          <div className="dialog-actions">
            <button type="button" onClick={() => setDevices(null)}>
              Cancel
            </button>
            <button
              type="button"
              className="suggested"
              onClick={() => {
                const change = devices;
                setDevices(null);
                void configure(change);
              }}
            >
              Restart
            </button>
          </div>
        </Dialog>
      ) : null}
    </section>
  );
}

function Diagnostics(props: { state: AppState }) {
  const { state } = props;
  const adapter = state.adapter!;
  const [result, setResult] = useState<Result>(null);
  return (
    <section className="section">
      <h2>Diagnostics</h2>
      <div className="card selectable">
        <Row title="Adapter ID">
          <span className="row-value">{adapter.id}</span>
        </Row>
        <Row title="Board">
          <span className="row-value">{adapter.board}</span>
        </Row>
        <Row title="Firmware">
          <span className="row-value">
            {adapter.firmwareVersion}
            {adapter.development ? " Development" : ""}
          </span>
        </Row>
        <Row title="Bluetooth">
          <span className="row-value">{adapter.ready ? "Ready" : "Starting"}</span>
        </Row>
        {adapter.connected ? (
          <>
            <Row title="Connected Address">
              <span className="row-value">{adapter.connected.address}</span>
            </Row>
            <Row title="Smooth Scrolling">
              <span className="row-value">{adapter.smoothScrolling ? "On" : "Off"}</span>
            </Row>
          </>
        ) : null}
        {adapter.targets.map((target, index) => (
          <Row key={target.address} title={targetName(state, target, index)}>
            <span className="row-value">{target.address}</span>
          </Row>
        ))}
        <Row title="Paired Device Limit">
          <span className="row-value">{adapter.maxTargets}</span>
        </Row>
        <Row title="Battery Level">
          <span className="row-value">{adapter.battery}%</span>
        </Row>
      </div>
      <div className="page-actions" style={{ marginTop: 12 }}>
        <button type="button" onClick={async () => setResult(await perform({ type: "restart" }, ""))}>
          Restart Adapter
        </button>
        {adapter.development ? (
          <button type="button" onClick={async () => setResult(await perform({ type: "bootloader" }, ""))}>
            Enter Bootloader
          </button>
        ) : null}
      </div>
      <Feedback result={result} />
    </section>
  );
}
