# USB discovery, framing and sessions

[Protocol index](README.md)

## Discovery

Every Dongle has USB VID/PID `1209:0001`. The product ID is a pid.codes test ID that other test
devices can share, so automatic discovery also requires the manufacturer string `Debonair`
when the operating system reports one. An explicitly selected port bypasses this filter.

- The serial number is the adapter ID: 16 uppercase hexadecimal digits, the same string as
  `Status.id`. Pico W derives it from its flash unique ID, RP2350 boards from their chip ID, and
  ESP32-S3 from its `OPTIONAL_UNIQUE_ID` eFuse field read as a little-endian 64-bit integer, or
  from its factory MAC address when that field is blank. It is available before Bluetooth starts
  and is stable across boots.
- The product string is the board's default name, such as `Pico W`. It does not change when a
  name is set with `SetConfig`.
- The device has one configuration with one CDC-ACM function: interfaces 0 (communications) and 1
  (data), with 64-byte bulk endpoints. Keyboard and mouse input reach Targets over Bluetooth, so
  there is no USB HID interface.

Line coding has no effect: any baud rate works, and the desktop app uses the conventional 115200
8-N-1. Opening the port or changing its settings never restarts the Dongle.

## Framing

- Each message is protobuf-encoded, then COBS-encoded, then followed by one `0x00` byte. COBS
  output contains no zero byte, so a zero always ends a frame and a receiver that joins mid-stream
  resynchronizes at the next one.
- Client to Dongle, every frame is a `ClientMessage`. Dongle to Client, every frame is a
  `DongleMessage` holding either a `Response` or an `Event`.
- Empty frames (two delimiters in a row) are ignored.
- A Client frame is at most 512 bytes before COBS encoding. A longer frame is discarded through its
  delimiter and answered with `ERROR_CODE_INVALID_REQUEST`.
- Dongle frames have no length limit. The Dongle writes them in USB packets of at most 63 bytes,
  so each packet ends a transfer and reaches the Client's read at once, and never interleaves two
  frames.
- USB checksums every packet, so frames carry no checksum of their own.

## Requests and responses

The Client chooses each `Request.id` and the Dongle echoes it in the `Response`. The Dongle answers
requests one at a time, in the order they arrive, so responses arrive in request order. A Client
may send requests back to back and match responses by id.

- A frame that is not valid COBS, does not decode as a `ClientMessage`, is too long, or holds
  neither a `Request` nor `Input` gets a `Response` with id 0 and `ERROR_CODE_INVALID_REQUEST`.
  Because such a frame has no usable id, the Client never uses id 0 for its own requests. A request
  whose frame was damaged this way stays unanswered, and the Client treats it like a lost response.
- A `Request` with no command set gets `ERROR_CODE_UNKNOWN_COMMAND`. That is also what firmware sees
  for a command added after it was built, because protobuf decodes an unknown `oneof` variant as an
  unset one.
- A command with a missing Required field or a value out of range gets `ERROR_CODE_INVALID_VALUE`.
- A `Response` with no result set means success with nothing to return.

Every command answers at once. Bluetooth work that takes longer, such as pairing or disconnecting,
reports its progress through events.

`Input` is never answered. It is applied in the order it arrives, interleaved with requests, so the
Client streams it without waiting. See [Input](input.md).

## Events

An event can arrive between any two frames, including between a request and its response.

- `Status` carries the complete current status. A Client replaces what it had with the latest one.
  The Dongle keeps one pending `Status` event: after any change, it writes the event once the
  output before it has been handed to USB, with the status at that moment. Changes made in the
  meantime go out together, so intermediate states can be skipped but the latest one always
  arrives.
- `PasskeyRequest` and `PairingResult` each report one occurrence. The Dongle keeps at most one of
  each waiting, in the order they happened, and writes them as soon as fewer than 4096 bytes wait
  and no `Response` waits for memory, before the next `Status` event and before any `Response` to
  a request read after them. A new `PasskeyRequest` replaces one not yet written, since only the
  latest prompt can be answered. A `PairingResult` replaces one not yet written, since the latest
  outcome is the one that stands, and discards a `PasskeyRequest` not yet written, whose prompt it
  closes; a disconnect discards such a `PasskeyRequest` too. The latest `PairingResult` therefore
  always arrives, after any `PasskeyRequest` written before it; an earlier `PairingResult`, or a
  `PasskeyRequest` whose prompt has closed, can be skipped.

The Dongle drops no `Response` for an open session. While 4096 bytes or more wait to be written,
or while a `Response` or one of these events waits to be queued, the Dongle stops reading the
Client's frames, requests and input alike, until the Client has read enough; USB flow control
holds the rest at the Client. A Client that stops reading the port therefore holds back its own
requests and input, and loses no response. An event that cannot be queued for lack of memory stays
waiting and is written later.

## Sessions

- A session starts when the Client opens the port and raises DTR, or when data arrives on a port
  that is not open, since some hosts never raise DTR.
- It ends when the Client lowers DTR or closes the port, or when the USB bus resets or the
  Dongle's USB configuration changes.
- When a session starts or ends, the Dongle discards any partial frame from the Client and any
  output and events not yet written, so a new session starts with nothing waiting. A
  `PasskeyRequest` sent before the session started is not repeated; the Client learns how that
  pairing attempt ends from its `PairingResult`.
- When a session ends, and when the USB bus suspends, the Dongle releases every key, button and
  consumer control the Client was holding, so a Client that quits, crashes or sleeps cannot leave
  input held on a Target. A suspend does not end the session.
- The Client writes one `0x00` before its first frame, so a partial frame left by an earlier Client
  cannot merge with it. It ignores frames it cannot decode and responses whose id it did not send,
  and starts with `GetStatus`.
- Bluetooth connections, pairings and pairing mode carry on without a session.

The desktop app opens the port from closed, which raises DTR, and closing the port lowers it. The
operating system closes the port of a process that crashes, which ends its session.
