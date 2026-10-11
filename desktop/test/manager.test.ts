import { create, fromBinary, toBinary, type MessageInitShape } from "@bufbuild/protobuf";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  ClientMessageSchema,
  DongleMessageSchema,
  ErrorCode,
  FrameDecoder,
  LinkState,
  StatusSchema,
  encodeFrame,
  type ClientMessage,
  type Status,
} from "@debonair/protocol";

type Kind = MessageInitShape<typeof DongleMessageSchema>["kind"];
import { Manager, describe as describeError } from "../src/main/manager.ts";
import { AdapterError, SessionError, type ByteStream } from "../src/main/session.ts";
import { DEFAULT_SETTINGS } from "../src/shared/settings.ts";
import type { AppState } from "../src/shared/state.ts";

/** An adapter that answers requests from a status record. */
class FakeAdapter implements ByteStream {
  received: ClientMessage[] = [];
  status: Status = create(StatusSchema, {
    id: "ADAPTER1",
    ready: true,
    link: LinkState.WAITING,
    config: { name: "Debonair", keyboard: true, mouse: true },
    battery: 100,
  });
  fail: ErrorCode | null = null;
  #decoder = new FrameDecoder();
  #data: ((chunk: Uint8Array) => void) | null = null;
  #close: ((error: Error | null) => void) | null = null;
  onData(listener: (chunk: Uint8Array) => void) {
    this.#data = listener;
  }
  onClose(listener: (error: Error | null) => void) {
    this.#close = listener;
  }
  write(bytes: Uint8Array) {
    for (const frame of this.#decoder.push(bytes)) {
      if (!frame.ok) continue;
      const message = fromBinary(ClientMessageSchema, frame.bytes);
      this.received.push(message);
      if (message.kind.case !== "request") continue;
      const { id, command } = message.kind.value;
      if (command.case === "setBattery") this.status.battery = command.value.level;
      const result: Kind =
        this.fail !== null
          ? { case: "response", value: { id, result: { case: "error", value: { code: this.fail } } } }
          : command.case === "getStatus" || command.case === "setConfig"
            ? { case: "response", value: { id, result: { case: "status", value: this.status } } }
            : { case: "response", value: { id, result: { case: undefined } } };
      queueMicrotask(() => this.send(result));
    }
  }
  async close() {}
  send(kind: Kind) {
    this.#data!(encodeFrame(toBinary(DongleMessageSchema, create(DongleMessageSchema, { kind }))));
  }
  unplug() {
    this.#close!(new Error("unplugged"));
  }
}

function initial(): AppState {
  return {
    version: "0.0.0",
    platform: "linux",
    connection: "searching",
    adapters: [],
    adapter: null,
    settings: structuredClone(DEFAULT_SETTINGS),
    passkeyRequested: false,
    pairingResult: null,
  };
}

afterEach(() => vi.useRealTimers());

async function connected() {
  const adapter = new FakeAdapter();
  const states: AppState[] = [];
  const manager = new Manager(
    { list: async () => [{ path: "/dev/ttyACM0", id: "ADAPTER1" }], open: async () => adapter },
    initial(),
    (state) => states.push(state),
  );
  manager.start();
  await vi.waitFor(() => expect(manager.state.connection).toBe("connected"));
  return { adapter, manager, states };
}

describe("manager", () => {
  it("connects to a plugged-in adapter and follows its events", async () => {
    const { adapter, manager } = await connected();
    expect(manager.state.adapters).toEqual(["ADAPTER1"]);
    expect(manager.state.adapter?.name).toBe("Debonair");
    adapter.send({ case: "event", value: { kind: { case: "passkeyRequest", value: {} } } });
    expect(manager.state.passkeyRequested).toBe(true);
    adapter.send({ case: "event", value: { kind: { case: "pairingResult", value: { success: true } } } });
    expect(manager.state).toMatchObject({ passkeyRequested: false, pairingResult: "succeeded" });
    await manager.stop();
  });

  it("sends input to the adapter", async () => {
    const { adapter, manager } = await connected();
    manager.input({ k: "ms", b: 1, x: 3, y: -4, w: 120, p: 0 });
    const last = adapter.received.at(-1)!;
    expect(last.kind.case).toBe("input");
    expect(last.kind.value).toMatchObject({ kind: { case: "mouse", value: { buttons: 1, x: 3, y: -4, wheel: 120 } } });
    await manager.stop();
  });

  it("reports failures as sentences", async () => {
    const { adapter, manager } = await connected();
    adapter.fail = ErrorCode.STORAGE_FAILED;
    const result = await manager.act({ type: "config", change: { name: "Desk" } });
    expect(result).toEqual({ ok: false, message: "The adapter couldn't save the change. Its storage failed." });
    adapter.fail = ErrorCode.NOT_READY;
    expect(await manager.act({ type: "config", change: { name: "Desk" } })).toEqual({
      ok: false,
      message: "The adapter isn't ready.",
    });
    expect(await manager.act({ type: "pairing", enabled: true })).toEqual({
      ok: false,
      message: "The adapter couldn't start pairing. Bluetooth is still starting. Try again in a moment.",
    });
    await manager.stop();
  });

  it("shares the battery level only when enabled", async () => {
    const { adapter, manager } = await connected();
    await manager.act({ type: "battery", level: 42 });
    expect(adapter.status.battery).toBe(42);
    await manager.setSettings({ ...manager.state.settings, shareBattery: false });
    await vi.waitFor(() => expect(adapter.status.battery).toBe(100));
    await manager.stop();
  });

  it("searches again after the adapter is unplugged", async () => {
    const { adapter, manager } = await connected();
    adapter.unplug();
    expect(manager.state.connection).toBe("searching");
    expect(manager.state.adapter).toBeNull();
    await manager.stop();
  });
});

describe("error sentences", () => {
  it("covers timeouts and closed sessions", () => {
    expect(describeError(new SessionError("x", true), "save the change")).toBe(
      "The adapter didn't respond, so it isn't known whether it could save the change.",
    );
    expect(describeError(new SessionError("x"), "start pairing")).toBe(
      "The adapter couldn't start pairing. The adapter was disconnected.",
    );
    expect(describeError(new AdapterError("x", ErrorCode.NOT_FOUND), "forget this device")).toBe(
      "The adapter couldn't forget this device. This device is no longer available.",
    );
    expect(describeError(new AdapterError("x", ErrorCode.UNKNOWN_COMMAND), "save the change")).toBe(
      "The adapter couldn't save the change. Its firmware doesn't support this.",
    );
  });
});
