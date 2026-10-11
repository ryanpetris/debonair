# Limits and errors

[Protocol index](README.md)

## Limits

| Limit | Value |
| --- | --- |
| Client frame | 512 bytes before COBS encoding |
| Response and event frames | No limit |
| Output waiting for the Client | Reading from the Client pauses while 4096 bytes or more wait; no response is dropped |
| Pairing events waiting for the Client | One `PasskeyRequest` and one `PairingResult`; a newer one replaces an unwritten one (see [Events](transport.md#events)) |
| Name | 1..29 bytes of UTF-8 without control characters or leading or trailing whitespace |
| Passkey | 0..999999 |
| Battery level | 0..100 |
| Held keyboard keys | 6 per report; more report ErrorRollOver |
| Held consumer controls | 2 per report |
| Motion and scrolling | -131068..131068 per value and message; -32767..32767 per report, split across reports |
| Reports waiting for the Target | 64 |
| Paired Targets | `Status.max_targets` per device combination |

## Errors

`Error` carries one `ErrorCode`. The comments on `ErrorCode` in
[`proto/debonair.proto`](../../proto/debonair.proto) say what each code means; the Client
supplies the words a user sees. A Client treats a code it does not know as a generic failure.

| Code | Meaning | Returned by |
| --- | --- | --- |
| `ERROR_CODE_UNSPECIFIED` | Never sent. | None |
| `ERROR_CODE_INVALID_REQUEST` | The frame was not valid COBS, did not decode as a `ClientMessage`, was over 512 bytes, or held neither a `Request` nor `Input`. | Any such frame, in a `Response` with id 0 |
| `ERROR_CODE_INVALID_VALUE` | A Required field is missing or a value is out of range. | `SetConfig`, `EnterPasskey`, `ForgetTarget`, `SetBattery` |
| `ERROR_CODE_NOT_READY` | Bluetooth has not started, or has failed. For `SetConfig`, the Dongle's storage did not open. | `SetConfig`, `StartPairing`, `StopPairing`, `EnterPasskey`, `Disconnect`, `ForgetTarget` |
| `ERROR_CODE_BUSY` | The Bluetooth stack is busy with another operation (NimBLE). | `EnterPasskey`, `Disconnect` |
| `ERROR_CODE_NOT_FOUND` | No passkey prompt is open, no Target is connected, or the Target is not paired. | `EnterPasskey`, `Disconnect`, `ForgetTarget` |
| `ERROR_CODE_STORAGE_FAILED` | Changing the Dongle's storage failed. After `SetConfig`, the previous settings stay in effect. After `ForgetTarget`, the Target is disconnected and gone from `Status.targets`, but its saved pairing stays until a later try removes it, so it can return after the Dongle restarts. | `SetConfig`, `ForgetTarget` |
| `ERROR_CODE_UNSUPPORTED` | This firmware does not support the command. | `EnterBootloader` on production firmware |
| `ERROR_CODE_RADIO_FAILED` | The Bluetooth stack rejected the operation (BTstack). | `EnterPasskey`, `Disconnect` |
| `ERROR_CODE_UNKNOWN_COMMAND` | The `Request` has no command, or one newer than the firmware. | Any such `Request` |

A lost response, such as one cut off by the Client closing the port or the Dongle being unplugged,
leaves the outcome of a command uncertain. After reconnecting, a Client reads `Status` before
retrying.

## Client pre-checks

A Client does not send a request it can tell will be refused, and does not offer the action. The
Dongle checks every request regardless; when a Client's view was out of date and the Dongle
refuses, the Client shows the error and does nothing else, since a `Status` event brings its view
up to date.

| Refusal | Client checks |
| --- | --- |
| Commands that need Bluetooth: `NOT_READY` | `Status.ready` |
| Saving settings when the Dongle's storage did not open: `NOT_READY` | None; `Status.ready` stays false while storage did not open, but is also false while Bluetooth starts, when `SetConfig` works |
| Answering a passkey prompt: `NOT_FOUND` | A `PasskeyRequest` arrived and no `PairingResult` or disconnect has followed |
| A passkey out of range: `INVALID_VALUE` | Six decimal digits |
| Disconnecting: `NOT_FOUND` | `Status.connected` |
| Forgetting a Target: `NOT_FOUND`, `INVALID_VALUE` | The Target is in `Status.targets`, with its address and type unchanged |
| A name too long, with control characters, or with leading or trailing whitespace: `INVALID_VALUE` | The name itself |
| Turning off both keyboard and mouse: `INVALID_VALUE` | The resulting `keyboard` and `mouse` |
| A battery level over 100: `INVALID_VALUE` | The level itself |
| Entering the bootloader on production firmware: `UNSUPPORTED` | `Status.development` |
| A command the firmware does not know: `UNKNOWN_COMMAND` | None; the Client reports that the firmware does not support the action |
