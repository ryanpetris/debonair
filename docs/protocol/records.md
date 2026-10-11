# Records

[Protocol index](README.md)

Field definitions are in [`proto/debonair.proto`](../../proto/debonair.proto). This page
explains what the records mean and how their values are chosen.

## Status

`Status` identifies the Dongle and reports its configuration, its Bluetooth state and the
connected Target. `GetStatus` and `SetConfig` return it, and a `Status` event carries it whenever
anything in it changes.

| Field | Meaning |
| --- | --- |
| `id` | The adapter ID: 16 uppercase hexadecimal digits, the same string as the USB serial number, stable across boots (see [Discovery](transport.md#discovery)). |
| `firmware_version` | The firmware's version string. |
| `board` | The board preset the firmware was built for, such as `pico_w`. |
| `development` | Development firmware, which accepts [development commands](development.md). |
| `config` | The configuration in effect; see [Config](#config). |
| `ready` | The Bluetooth stack has started. False while it starts and after it fails. |
| `link` | What the Bluetooth link is doing; see [LinkState](#linkstate). |
| `connected` | The connected Target, set exactly while `link` is `LINK_STATE_CONNECTED`. |
| `targets` | The Targets paired with the current device combination, empty until `ready`. |
| `leds` | Keyboard LEDs the connected Target set: bit 0 Num Lock, bit 1 Caps Lock, bit 2 Scroll Lock, bit 3 Compose, bit 4 Kana. Reset to 0 when a Target connects or disconnects. |
| `smooth_scrolling` | The connected Target set the wheel resolution multiplier and reads vertical scrolling in 1/120 of a detent. The pan multiplier is separate and not reported. Reset when a Target connects or disconnects, and restored from the pairing for a Target that set it before. See [Resolution multiplier](input.md#resolution-multiplier). |
| `battery` | The battery level Targets read, 0..100; 100 after the Dongle starts, until `SetBattery` changes it. |
| `max_targets` | How many Targets each device combination can hold; 0 until `ready`. |

## Config

`Config` holds the settings that change how the Dongle appears to Targets. The Dongle saves them,
and `SetConfig` changes them.

- `name` is the effective name Targets see: the saved name, or `Debonair` when none is saved.
  At most 29 bytes of UTF-8.
- `keyboard` and `mouse` say which devices the Dongle offers; at least one is on. The keyboard
  includes consumer controls such as media keys. Each combination is paired separately; see
  [Device combinations](commands.md#device-combinations).
- `security` is the pairing method; see [Security](#security).

## Security

| Value | Meaning |
| --- | --- |
| `SECURITY_JUST_WORKS` | Pairing completes without a passkey. The default. |
| `SECURITY_PASSKEY` | The Target shows a passkey that the user types into the Client, which sends it with `EnterPasskey`. |

A change applies to pairings that start afterwards; existing pairings are kept.

## LinkState

| Value | Meaning |
| --- | --- |
| `LINK_STATE_OFF` | Bluetooth has not started, or has failed. |
| `LINK_STATE_WAITING` | Waiting for a paired Target to connect. |
| `LINK_STATE_PAIRING` | In pairing mode: discoverable, waiting for a new Target to pair, with no Target connected. |
| `LINK_STATE_CONNECTED` | A Target is connected. Input reaches it only once the link is encrypted; until then it is discarded. |

`LINK_STATE_CONNECTED` takes precedence over pairing mode, which can stay on while a Target is
connected.

## Target

`Target` identifies one Bluetooth peer by its address, 6 bytes most significant byte first, and its
`AddressType`.

| Value | Meaning |
| --- | --- |
| `ADDRESS_TYPE_PUBLIC` | A public device address. |
| `ADDRESS_TYPE_RANDOM` | A random device address. |

In `Status.targets`, the address is each paired Target's identity address, which `ForgetTarget`
takes back unchanged. In `Status.connected`, a Target that connects from a private address is
reported with that address until the Dongle resolves or learns its identity address.

## Events

| Event | Meaning |
| --- | --- |
| `status` | The latest `Status` after anything in it changed. |
| `passkey_request` | A Target shows a passkey and waits for the user to enter it. Answer with `EnterPasskey`. |
| `pairing_result` | A pairing attempt finished; `success` is false when it failed or was rejected. |

See [Events](transport.md#events) for how they are delivered.
