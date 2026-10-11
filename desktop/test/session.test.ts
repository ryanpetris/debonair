import { create, fromBinary, toBinary, type MessageInitShape } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";
import {
  ClientMessageSchema,
  DongleMessageSchema,
  ErrorCode,
  FrameDecoder,
  encodeFrame,
  type ClientMessage,
} from "@debonair/protocol";

type Kind = MessageInitShape<typeof DongleMessageSchema>["kind"];
import { AdapterError, Session, SessionError, type ByteStream } from "../src/main/session.ts";

class FakeStream implements ByteStream {
  written: ClientMessage[] = [];
  raw: Uint8Array[] = [];
  #decoder = new FrameDecoder();
  #data: ((chunk: Uint8Array) => void) | null = null;
  #close: ((error: Error | null) => void) | null = null;
  closed = false;
  onData(listener: (chunk: Uint8Array) => void) {
    this.#data = listener;
  }
  onClose(listener: (error: Error | null) => void) {
    this.#close = listener;
  }
  write(bytes: Uint8Array) {
    this.raw.push(bytes);
    for (const frame of this.#decoder.push(bytes)) {
      if (frame.ok) this.written.push(fromBinary(ClientMessageSchema, frame.bytes));
    }
  }
  async close() {
    this.closed = true;
  }
  send(message: Kind) {
    this.#data!(encodeFrame(toBinary(DongleMessageSchema, create(DongleMessageSchema, { kind: message }))));
  }
  fail() {
    this.#close!(new Error("unplugged"));
  }
}

const requestId = (stream: FakeStream, index: number) => {
  const message = stream.written[index]!;
  if (message.kind.case !== "request") throw new Error("not a request");
  return message.kind.value.id;
};

describe("session", () => {
  it("starts with a delimiter and matches responses by id", async () => {
    const stream = new FakeStream();
    const session = new Session(stream, () => {}, () => {});
    expect([...stream.raw[0]!]).toEqual([0]);
    const first = session.request({ case: "startPairing", value: {} });
    const second = session.status();
    const firstId = requestId(stream, 0);
    const secondId = requestId(stream, 1);
    expect(firstId).not.toBe(secondId);
    stream.send({ case: "response", value: { id: secondId, result: { case: "status", value: { id: "ABC" } } } });
    stream.send({ case: "response", value: { id: firstId, result: { case: undefined } } });
    expect((await second).id).toBe("ABC");
    expect((await first).result.case).toBeUndefined();
  });

  it("rejects with the adapter's error code", async () => {
    const stream = new FakeStream();
    const session = new Session(stream, () => {}, () => {});
    const request = session.request({ case: "disconnect", value: {} });
    stream.send({
      case: "response",
      value: { id: requestId(stream, 0), result: { case: "error", value: { code: ErrorCode.NOT_FOUND } } },
    });
    await expect(request).rejects.toBeInstanceOf(AdapterError);
    await expect(request).rejects.toMatchObject({ code: ErrorCode.NOT_FOUND });
  });

  it("delivers events and writes input without waiting", () => {
    const stream = new FakeStream();
    const events: string[] = [];
    const session = new Session(stream, (event) => events.push(event.kind.case ?? ""), () => {});
    session.input({ case: "keyboard", value: { modifiers: 2, keys: Uint8Array.of(4) } });
    stream.send({ case: "event", value: { kind: { case: "passkeyRequest", value: {} } } });
    expect(events).toEqual(["passkeyRequest"]);
    const input = stream.written[0]!;
    expect(input.kind.case).toBe("input");
    // Each input frame fits one USB packet.
    expect(stream.raw[1]!.length).toBeLessThanOrEqual(64);
  });

  it("fails pending requests when the port closes", async () => {
    const stream = new FakeStream();
    let closed: Error | null = null;
    const session = new Session(stream, () => {}, (error) => (closed = error));
    const request = session.status();
    stream.fail();
    await expect(request).rejects.toBeInstanceOf(SessionError);
    expect(closed).toBeInstanceOf(Error);
    expect(session.closed).toBe(true);
    await expect(session.status()).rejects.toBeInstanceOf(SessionError);
  });

  it("times out when the adapter does not answer", async () => {
    const stream = new FakeStream();
    const session = new Session(stream, () => {}, () => {});
    await expect(session.request({ case: "getStatus", value: {} }, 5)).rejects.toMatchObject({ timeout: true });
  });
});
