// The Debonair serial API: protobuf messages generated from proto/debonair.proto and the
// frame encoding used on the USB serial port.
export * from "./gen/debonair_pb.ts";
export * from "./framing.ts";

/** USB vendor ID of every Dongle (pid.codes). */
export const USB_VENDOR_ID = 0x1209;
/** USB product ID of every Dongle. This is a pid.codes test ID. */
export const USB_PRODUCT_ID = 0x0001;
/** USB manufacturer string of every Dongle. */
export const USB_MANUFACTURER = "Debonair";
/** The longest client message a Dongle accepts, before frame encoding. */
export const MAX_CLIENT_MESSAGE_BYTES = 512;
/** The longest name a Dongle shows to Targets, in UTF-8 bytes. */
export const MAX_NAME_BYTES = 29;
