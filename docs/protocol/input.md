# Input and reports to the Target

[Protocol index](README.md)

`Input` carries keyboard, consumer control and mouse input for the connected Target. It is never
answered and is applied in the order it arrives, interleaved with requests. The Dongle turns it
into HID reports and queues them for the Target in the same order, so a modifier pressed before a
click reaches the Target before the click.

Input reaches a Target only once its link is encrypted. `Status.link` is
`LINK_STATE_CONNECTED` as soon as a Target connects, but until the link is encrypted, and while no
Target is connected, input is discarded. Input for a device that `SetConfig` turned off is ignored:
keyboard and consumer input when the keyboard is off, mouse input when the mouse is off. An `Input`
with no kind set is ignored.

## Keyboard

`KeyboardState` carries the complete set of held keys, so a lost or repeated message cannot leave a
key held. Each message replaces the previous state, and the Dongle sends a report only when the
state changes.

- `modifiers` is the HID keyboard modifier byte: bit 0 Left Control through bit 7 Right GUI.
  Higher bits are ignored.
- `keys` lists HID Keyboard page usages in the order they were pressed. Modifier usages
  `0xE0`..`0xE7` are accepted here and folded into the modifier byte. Usages `0x04`..`0xDF` are
  keys; a usage listed twice counts once. Every other usage is ignored.
- The report carries six keys (6-key rollover). With more than six distinct keys held, the report
  holds ErrorRollOver (`0x01`) in every key slot, with the modifiers still set, until no more than
  six are held.

## Consumer controls

`ConsumerState` carries the complete set of held consumer controls, such as media and volume keys,
as HID Consumer page usages. The report holds at most two: the first two distinct usages in
`0x001`..`0x3FF` are sent and the rest are ignored, as are usages outside that range. A report is
sent only when the set changes.

## Mouse

`MouseInput` carries the held buttons and the motion and scrolling since the previous `MouseInput`.

- `buttons` holds bit 0 primary, bit 1 secondary, bit 2 middle, bit 3 back and bit 4 forward.
  Higher bits are ignored. A report is sent when the buttons change or there is motion or
  scrolling.
- `x` and `y` are in Target pixels, positive right and down. `wheel` and `pan` are in 1/120 of a
  wheel detent, positive up and right.
- Each value is clamped to -131068..131068 (four times 32767) per message, so a corrupt or extreme
  message cannot flood the report queue. Within that, motion and scrolling are never clamped: a
  value larger than one report carries (-32767..32767) is split across several reports.

## Resolution multiplier

The mouse report has a resolution multiplier for the wheel and another for horizontal pan. A Target
that sets a multiplier (Windows and Linux do) reads that axis in 1/120 of a detent, which makes
trackpad scrolling smooth. For an axis whose multiplier is not set, the Dongle sends whole detents
and carries the remainder to the next message; the remainder starts again when a Target connects or
the multiplier changes.

A Target sets the multipliers only when it first sets up the device, so the Dongle saves them with
each paired Target's pairing and restores them when that Target reconnects.
`Status.smooth_scrolling` reports the wheel multiplier of the connected Target; the pan multiplier
is not reported.

## Boot protocol

A Target that switches to the HID boot protocol, such as a computer's firmware setup, receives the
boot keyboard and boot mouse reports. Keyboard reports are unchanged. Mouse reports carry only the
first three buttons, motion clamped to -127..127 per report and no scrolling. Consumer reports are
not sent.

## The report queue

Reports wait in a queue of 64 while the Target cannot keep up, and leave in order as the Bluetooth
stack accepts them.

- When the newest queued report is a mouse report with the same buttons, further motion and
  scrolling are merged into it, up to what one report carries.
- When the queue is full, the Dongle drops the oldest report that a later report of the same kind
  (keyboard, mouse or consumer) replaces, and then queues the new one. Keyboard, button and consumer
  reports carry complete state, so the last state of each kind always reaches the Target and
  nothing is left held. A dropped mouse report's motion and scrolling are lost.
- A report the Bluetooth stack refuses is dropped; later reports carry the current state.
- When a Target connects or disconnects, the queue is emptied and the held state starts from
  nothing pressed.

## Releasing input

When the Client's session ends or its USB bus suspends, the Dongle releases every held key, button
and consumer control by queuing reports with nothing held, so a Client that quits, crashes or sleeps
cannot leave input held on a Target.

## Latency

Protobuf encoding and COBS framing add a few bytes and microseconds to each message. The latency a
Target sees is set by the USB frame (1 ms), the time the Dongle spends on the message, and the
Bluetooth connection interval. Both Bluetooth stacks ask the Target for a 15 ms connection interval
(minimum and maximum 12 units of 1.25 ms) with no peripheral latency; the Target chooses the
interval it uses.
