// Runs the host adapter simulator (rust/crates/debonair-sim) as a child process in place of a
// serial port, for developing and testing without hardware.
import { spawn } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Ports } from "./manager.ts";
import type { ByteStream } from "./session.ts";

let created: string | undefined;

/** Where the simulator saves its records: DEBONAIR_SIM_STATE, or a new directory for this run of
 * the app, removed when it exits. Every simulator the app starts uses it, so settings outlast the
 * restarts the simulator makes. */
function stateDirectory(): string {
  const given = process.env.DEBONAIR_SIM_STATE;
  if (given) return given;
  if (!created) {
    const directory = mkdtempSync(join(tmpdir(), "debonair-sim-"));
    created = directory;
    process.once("exit", () => rmSync(directory, { recursive: true, force: true }));
  }
  return created;
}

export function simulatorPorts(executable: string): Ports {
  return {
    list: async () => [{ path: executable, id: "SIMULATOR" }],
    open: async (path) => {
      const child = spawn(path, [], {
        stdio: ["pipe", "pipe", "inherit"],
        env: { ...process.env, DEBONAIR_SIM_STATE: stateDirectory() },
      });
      let closed = false;
      const listeners: ((error: Error | null) => void)[] = [];
      const end = (error: Error | null) => {
        if (closed) return;
        closed = true;
        for (const listener of listeners.splice(0)) listener(error);
      };
      child.on("exit", () => end(null));
      child.on("error", (error) => end(error));
      child.stdin.on("error", (error) => end(error));
      const stream: ByteStream = {
        onData: (listener) => child.stdout.on("data", (chunk: Buffer) => listener(new Uint8Array(chunk))),
        onClose: (listener) => listeners.push(listener),
        write: (bytes) => {
          if (!closed) child.stdin.write(bytes);
        },
        close: async () => {
          closed = true;
          child.kill();
        },
      };
      return stream;
    },
  };
}
