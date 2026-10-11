# Commands

[Protocol index](README.md)

Each command is one `Request.command` variant; its message in
[`proto/debonair.proto`](../../proto/debonair.proto) documents its fields. This page describes
what each command does and which errors it returns. Every command answers at once, and a `Status`
event follows whenever the command changed anything in `Status`; a command that changes nothing
sends no event.

## Readiness

`StartPairing`, `StopPairing`, `EnterPasskey`, `Disconnect` and `ForgetTarget` need the Bluetooth
stack. Until it has started, and after it fails, they return `ERROR_CODE_NOT_READY`;
`Status.ready` tells the Client which applies. `GetStatus`, `SetBattery`, `Restart` and
`EnterBootloader` work at any time. `SetConfig` works at any time unless the Dongle's storage did
not open, such as when its flash holds another layout, in which case it returns
`ERROR_CODE_NOT_READY`; Bluetooth does not start then either.

## Validation

A command is refused only when carrying it out would fail or store something invalid: a missing
Required field, a value out of range, a Target, connection or prompt that does not exist, or a
Bluetooth stack that does not take the request. A command that can be carried out is accepted,
even when it changes nothing. Clients check the refusals they can predict before sending (see
[Limits and errors](limits-and-errors.md)), and show the Dongle's error when it refuses anyway.

## Command reference

| Command | Result | Behavior |
| --- | --- | --- |
| `GetStatus` | `Status` | The current status. Never fails. |
| `SetConfig` | `Status` | Partial update of the name, the keyboard and mouse and the pairing security, saved together. A name is 1..29 bytes of UTF-8 without control characters or leading or trailing whitespace; `""` restores the default name. Returns `ERROR_CODE_INVALID_VALUE` for an invalid name, an unknown `Security` value or a result with both keyboard and mouse off, `ERROR_CODE_NOT_READY` when the Dongle's storage did not open, and `ERROR_CODE_STORAGE_FAILED` when saving fails; each leaves every setting as it was. A request that changes nothing responds with `Status` and saves nothing. Otherwise the Dongle saves the settings and responds with the new `Status`. A new name applies to Targets at once, and new security to pairings that start afterwards. A change to keyboard or mouse restarts the Dongle after the response (see [Device combinations](#device-combinations)). |
| `StartPairing` | none | Enters pairing mode: the Dongle becomes discoverable and a new Target may pair. `ERROR_CODE_NOT_READY` before Bluetooth has started. |
| `StopPairing` | none | Leaves pairing mode. `ERROR_CODE_NOT_READY` before Bluetooth has started. |
| `EnterPasskey` | none | Answers the open `PasskeyRequest` with the passkey the Target shows, or rejects the pairing when `passkey` is missing. Returns `ERROR_CODE_NOT_READY` before Bluetooth has started, `ERROR_CODE_NOT_FOUND` when no `PasskeyRequest` is open and `ERROR_CODE_INVALID_VALUE` for a passkey over 999999. The prompt closes once a valid answer is given, even when the Bluetooth stack then refuses it with `ERROR_CODE_RADIO_FAILED` (BTstack) or `ERROR_CODE_BUSY` (NimBLE). |
| `Disconnect` | none | Disconnects the connected Target; a `Status` event follows once the link is gone. A paired Target can connect again on its own. Returns `ERROR_CODE_NOT_READY` before Bluetooth has started, `ERROR_CODE_NOT_FOUND` when no Target is connected, and `ERROR_CODE_RADIO_FAILED` (BTstack) or `ERROR_CODE_BUSY` (NimBLE) when the stack does not take the request. |
| `ForgetTarget` | none | Removes the pairing with one Target from the current device combination, disconnecting it if it is connected, and responds once its saved pairing is removed. BTstack removes the Target from the stack before the response; NimBLE removes it on its host task just after, and from the response on the Target is no longer listed or saved. A `Status` event with the new `targets` follows. Returns `ERROR_CODE_NOT_READY` before Bluetooth has started, `ERROR_CODE_INVALID_VALUE` for an address that is not 6 bytes, `ERROR_CODE_NOT_FOUND` for a Target that is not in `Status.targets`, `ERROR_CODE_NOT_FOUND` (BTstack) when the stack cannot remove it, and `ERROR_CODE_STORAGE_FAILED` when removing its saved pairing failed. After `ERROR_CODE_STORAGE_FAILED` the Target is disconnected and gone from `Status.targets`, and the Dongle tries the removal again every 5 s and before it restarts; until a try succeeds, the Target can return after the Dongle restarts. An `address_type` other than `ADDRESS_TYPE_RANDOM` means public. |
| `SetBattery` | none | Sets the battery level Targets read from the Battery service, 0..100. Returns `ERROR_CODE_INVALID_VALUE` for a level over 100. The level is not saved: the Dongle reports 100 after it starts. |
| `Restart` | none | Restarts the Dongle after the response (see [Restarts](#restarts)). |
| `EnterBootloader` | none | Development firmware only; production firmware returns `ERROR_CODE_UNSUPPORTED`. See [Development commands](development.md). |

A `Request` with no command, or with a command newer than the firmware, returns
`ERROR_CODE_UNKNOWN_COMMAND`.

## Pairing

When Bluetooth starts with no Target paired to the current device combination, the Dongle enters
pairing mode by itself; otherwise it starts out of pairing mode and waits for a paired Target to
connect. Pairing mode ends when a Target pairs successfully or `StopPairing` is sent. A failed
attempt leaves pairing mode on.

With `SECURITY_JUST_WORKS`, pairing completes without the user. With `SECURITY_PASSKEY`, the Target
shows a six-digit passkey and the Dongle sends a `PasskeyRequest`; the Client asks the user for the
passkey and answers with `EnterPasskey`. Each attempt ends with a `PairingResult`, which closes any
open prompt; a disconnect also closes it. A successful pairing adds the Target to
`Status.targets`.

A pairing counts as successful only once the Dongle has saved it, with the Bluetooth identity its
keys depend on. When saving fails, the Dongle removes the pairing, disconnects the Target and sends
`PairingResult` with `success` false, so a Target is never shown as paired when it would be lost
at the next restart. Pairing mode stays on, as after any failed attempt.

`Status.max_targets` is how many Targets each device combination can hold.

## Device combinations

`SetConfig` can turn off the keyboard or the mouse, but not both. Each of the three combinations
has its own Bluetooth identity address, HID report map and pairings, so a Target never sees a
cached report map that no longer matches, and pairings made with one combination are not shown or
used with another. Changing the combination saves the settings, responds with the new `Status`, and
restarts the Dongle, which then starts Bluetooth with the new combination.

Turning off the keyboard also turns off consumer controls. Input for a device that is off is
ignored.

## Restarts

`Restart`, `EnterBootloader` and a `SetConfig` that changes the device combination restart the
Dongle after their response. The Dongle restarts once that response and everything queued before
it have been handed to USB and the host has had a few milliseconds to take the last packet, or
about 300 ms after the request when the Client is slow to read. It then saves pairing changes
still waiting, such as a Target's feature report, which adds the time those writes take to flash
before it restarts. Frames handled in the meantime may lose their responses to the restart, so a
Client sends nothing more until the Dongle reappears. Held input is not released first; the
Target sees the link drop.
