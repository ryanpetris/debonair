# Serial API

The Dongle's USB serial port (CDC-ACM) carries a protobuf API. The Client sends `ClientMessage`
frames that hold either a `Request`, which the Dongle answers with exactly one `Response` carrying
the same `id`, or `Input` for the connected Target, which is never answered. The Dongle also sends
an `Event` whenever its status changes or a Target needs something from the user. There are no
subscriptions, heartbeats or revisions, and no protocol version number.

One Client process owns the port at a time. The Dongle stores its pairings and every setting that
changes how it appears to Targets, and keeps working without a Client: paired Targets connect, and
pairing mode stays as it was. The Client stores its own capture preferences, such as pointer speed
and shortcuts, and sends the Dongle only the resulting input.

## Documents

- [USB discovery, framing and sessions](transport.md)
- [Compatibility rules](compatibility.md)
- [Commands](commands.md)
- [Input and reports to the Target](input.md)
- [Records: status, configuration and Targets](records.md)
- [Limits and errors](limits-and-errors.md)
- [Development commands](development.md)

## Definitions

[`proto/debonair.proto`](../../proto/debonair.proto) defines every message. Its comments say
what each field means, which fields are Required, what a missing field means, the range of each
value and which error codes each command returns.

`buf lint` checks the schema, and `buf breaking` compares it with the newest earlier release in
the same series (see [Compatibility rules](compatibility.md)).

## Libraries

The protocol is a library on each side, so other projects can use it too:

| | Rust | TypeScript |
| --- | --- | --- |
| Messages, framing and USB constants | `debonair-protocol` | `@debonair/protocol` |

The protocol libraries do no I/O. The Rust one is `no_std` and shared with the firmware, and
generates its messages from the schema at build time. The TypeScript messages are generated with
`protoc-gen-es` and checked in; `npm --prefix desktop run generate` regenerates them after a schema
change, and `make check-desktop` fails when they are out of date.
