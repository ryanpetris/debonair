// Converts the adapter's protobuf status into the window's plain state.
import { AddressType, LinkState, Security, type Status, type Target as ProtoTarget } from "@debonair/protocol";
import { formatAddress, type AdapterStatus, type Link, type Target } from "../shared/state.ts";

const LINKS: Record<number, Link> = {
  [LinkState.OFF]: "off",
  [LinkState.WAITING]: "waiting",
  [LinkState.PAIRING]: "pairing",
  [LinkState.CONNECTED]: "connected",
};

function target(value: ProtoTarget): Target {
  return { address: formatAddress(value.address), random: value.addressType === AddressType.RANDOM };
}

export function adapterStatus(status: Status): AdapterStatus {
  return {
    id: status.id,
    name: status.config?.name ?? "",
    firmwareVersion: status.firmwareVersion,
    board: status.board,
    development: status.development,
    ready: status.ready,
    keyboard: status.config?.keyboard ?? true,
    mouse: status.config?.mouse ?? true,
    security: status.config?.security === Security.PASSKEY ? "passkey" : "justWorks",
    link: LINKS[status.link] ?? "off",
    connected: status.connected ? target(status.connected) : null,
    targets: status.targets.map(target),
    leds: status.leds,
    smoothScrolling: status.smoothScrolling,
    battery: status.battery,
    maxTargets: status.maxTargets,
  };
}
