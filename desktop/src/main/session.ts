// One session with an adapter over a byte stream. Requests carry an id that the adapter echoes in
// its Response; input is written without waiting for anything.
import { create, fromBinary, toBinary, type MessageInitShape } from "@bufbuild/protobuf";
import {
  ClientMessageSchema,
  DELIMITER,
  DongleMessageSchema,
  ErrorCode,
  FrameDecoder,
  encodeFrame,
  type Event,
  type InputSchema,
  type RequestSchema,
  type Response,
  type Status,
} from "@debonair/protocol";

/** An open serial port. Opening raises DTR, which starts a session on the adapter. */
export interface ByteStream {
  onData(listener: (chunk: Uint8Array) => void): void;
  /** Called once when the port closes or fails, never after close(). */
  onClose(listener: (error: Error | null) => void): void;
  /** Queues bytes for the port. A failure closes the stream. */
  write(bytes: Uint8Array): void;
  close(): Promise<void>;
}

export type Command = Exclude<MessageInitShape<typeof RequestSchema>["command"], { case: undefined } | undefined>;
export type InputKind = Exclude<MessageInitShape<typeof InputSchema>["kind"], { case: undefined } | undefined>;

/** The adapter answered with an error. */
export class AdapterError extends Error {
  readonly code: ErrorCode;
  constructor(command: string, code: ErrorCode) {
    super(`${command} failed: ${ErrorCode[code] ?? code}`);
    this.name = "AdapterError";
    this.code = code;
  }
}

/** The session ended or the adapter did not answer in time. */
export class SessionError extends Error {
  readonly timeout: boolean;
  constructor(message: string, timeout = false) {
    super(message);
    this.name = "SessionError";
    this.timeout = timeout;
  }
}

export const REQUEST_TIMEOUT_MS = 3000;

interface Pending {
  command: string;
  resolve(response: Response): void;
  reject(error: Error): void;
  timer: ReturnType<typeof setTimeout>;
}

export class Session {
  readonly #stream: ByteStream;
  readonly #decoder = new FrameDecoder(64 * 1024);
  readonly #pending = new Map<number, Pending>();
  #next = 1;
  #closed = false;
  #onEvent: (event: Event) => void;
  #onClose: (error: Error | null) => void;

  constructor(stream: ByteStream, onEvent: (event: Event) => void, onClose: (error: Error | null) => void) {
    this.#stream = stream;
    this.#onEvent = onEvent;
    this.#onClose = onClose;
    stream.onData((chunk) => this.#receive(chunk));
    stream.onClose((error) => this.#ended(error));
    // A delimiter first, so bytes left by an earlier client cannot merge with the first frame.
    stream.write(Uint8Array.of(DELIMITER));
  }

  get closed() {
    return this.#closed;
  }

  /** Sends a request and resolves with the adapter's Response, or rejects with its error. */
  request(command: Command, timeout = REQUEST_TIMEOUT_MS): Promise<Response> {
    if (this.#closed) return Promise.reject(new SessionError("session closed"));
    const id = this.#next;
    this.#next = this.#next >= 0xffffffff ? 1 : this.#next + 1;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.#pending.delete(id);
        reject(new SessionError(`${command.case} timed out`, true));
      }, timeout);
      this.#pending.set(id, { command: command.case, resolve, reject, timer });
      this.#write(create(ClientMessageSchema, { kind: { case: "request", value: { id, command } } }));
    });
  }

  async status(): Promise<Status> {
    const response = await this.request({ case: "getStatus", value: {} });
    if (response.result.case !== "status") throw new SessionError("getStatus got an unexpected response");
    return response.result.value;
  }

  input(kind: InputKind) {
    if (this.#closed) return;
    this.#write(create(ClientMessageSchema, { kind: { case: "input", value: { kind } } }));
  }

  async close() {
    if (this.#closed) return;
    this.#ended(null, false);
    await this.#stream.close();
  }

  #write(message: Parameters<typeof toBinary<typeof ClientMessageSchema>>[1]) {
    this.#stream.write(encodeFrame(toBinary(ClientMessageSchema, message)));
  }

  #receive(chunk: Uint8Array) {
    for (const frame of this.#decoder.push(chunk)) {
      if (!frame.ok) continue;
      let message;
      try {
        message = fromBinary(DongleMessageSchema, frame.bytes);
      } catch {
        continue;
      }
      if (message.kind.case === "event") this.#onEvent(message.kind.value);
      else if (message.kind.case === "response") this.#response(message.kind.value);
    }
  }

  #response(response: Response) {
    const pending = this.#pending.get(response.id);
    if (!pending) return;
    this.#pending.delete(response.id);
    clearTimeout(pending.timer);
    if (response.result.case === "error") pending.reject(new AdapterError(pending.command, response.result.value.code));
    else pending.resolve(response);
  }

  #ended(error: Error | null, notify = true) {
    if (this.#closed) return;
    this.#closed = true;
    for (const pending of this.#pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(new SessionError("session closed"));
    }
    this.#pending.clear();
    if (notify) this.#onClose(error);
  }
}
