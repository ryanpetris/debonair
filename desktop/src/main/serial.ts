// Adapter serial ports through the Node serial library.
//
// DTR follows the port's lifetime: opening the port raises DTR, starting a session on the adapter,
// and closing it lowers DTR, ending it. The operating system closes a crashed process's port, so
// the adapter releases any input that process was holding.
import { SerialPort } from "serialport";
import { USB_MANUFACTURER, USB_PRODUCT_ID, USB_VENDOR_ID } from "@debonair/protocol";
import type { ByteStream } from "./session.ts";

export interface PortInfo {
  path: string;
  /** The USB serial number, which is the adapter ID. */
  id: string;
}

const VENDOR_ID = USB_VENDOR_ID.toString(16).padStart(4, "0");
const PRODUCT_ID = USB_PRODUCT_ID.toString(16).padStart(4, "0");

/** Serial ports with the adapter's USB IDs and manufacturer. */
export async function listPorts(): Promise<PortInfo[]> {
  const ports = await SerialPort.list();
  const found = ports
    .filter(
      (p) =>
        p.vendorId?.toLowerCase().padStart(4, "0") === VENDOR_ID &&
        p.productId?.toLowerCase().padStart(4, "0") === PRODUCT_ID &&
        (p.manufacturer === undefined || p.manufacturer === USB_MANUFACTURER),
    )
    .map((p) => ({ path: p.path, id: p.serialNumber ?? p.path }));
  // macOS lists both callout and dial-in nodes for one USB serial device.
  const callouts = new Set(found.flatMap((p) => (p.path.startsWith("/dev/cu.") ? [p.path.slice(8)] : [])));
  return found
    .filter((p) => !(p.path.startsWith("/dev/tty.") && callouts.has(p.path.slice(9))))
    .sort((a, b) => a.path.localeCompare(b.path));
}

class SerialStream implements ByteStream {
  #closing = false;
  #closeListeners: ((error: Error | null) => void)[] = [];
  readonly #port: SerialPort;

  constructor(port: SerialPort) {
    this.#port = port;
    port.on("close", (error: Error | null) => this.#closed(error ?? null));
    port.on("error", (error: Error) => this.#closed(error));
  }

  #closed(error: Error | null) {
    if (this.#closing) return;
    this.#closing = true;
    for (const listener of this.#closeListeners.splice(0)) listener(error);
  }

  onData(listener: (chunk: Uint8Array) => void) {
    this.#port.on("data", listener);
  }
  onClose(listener: (error: Error | null) => void) {
    this.#closeListeners.push(listener);
  }
  write(bytes: Uint8Array) {
    if (this.#closing) return;
    this.#port.write(Buffer.from(bytes), (error) => {
      if (error) this.#closed(error);
    });
  }
  close() {
    this.#closing = true;
    this.#closeListeners = [];
    return new Promise<void>((resolve) => {
      if (!this.#port.isOpen) return resolve();
      this.#port.close(() => resolve());
    });
  }
}

/** Opens a port at the conventional 115200 8-N-1 settings, raising DTR. */
export function openSerial(path: string): Promise<ByteStream> {
  return new Promise((resolve, reject) => {
    const port = new SerialPort({ path, baudRate: 115200, hupcl: true, autoOpen: false });
    port.open((error) => (error ? reject(error) : resolve(new SerialStream(port))));
  });
}
