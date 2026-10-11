# Compatibility rules

[Protocol index](README.md)

The schema only grows. Old Clients keep working with new firmware, and new Clients with old
firmware, because every message follows the rules below. There is no protocol version number.

## Fields

Proto3 already behaves the way this API needs: a scalar field at its zero value is not sent, a
missing field reads as zero, unknown fields, `oneof` variants and enum values do not fail decoding,
and `optional` gives a field explicit presence. The schema comments mark each field as one of:

| Kind | Proto | When missing |
| --- | --- | --- |
| Required | Plain or message field marked `Required` in its comment. | The record is invalid. The Dongle refuses a request with `ERROR_CODE_INVALID_VALUE`, and always sets the Required fields of what it sends. |
| Defaulted | Plain field. | Read as the zero value, which is the field's default, such as no keys held or the first value of an enum. |
| Optional | `optional` field. | There is no value; the field's comment says what that means, such as "left unchanged" in `SetConfig` or "rejects the pairing" in `EnterPasskey`. |

Only identifiers and fields without which a record cannot be understood are Required:
`Status.id`, `Status.config`, `Target.address`, `ForgetTarget.address` and `Error.code`.
`Request.id` is Defaulted, but a Client never uses 0, which the Dongle keeps for frames it cannot
attribute to a request.

## Enums and oneofs

- The zero value of every enum is its safe default: `SECURITY_JUST_WORKS`, `LINK_STATE_OFF`,
  `ADDRESS_TYPE_PUBLIC`. `ERROR_CODE_UNSPECIFIED` is never sent.
- A Client reads an enum value it does not know as the zero value. An unknown `ErrorCode` is a
  generic failure. The Dongle refuses a `Security` value it does not know with
  `ERROR_CODE_INVALID_VALUE`.
- A `oneof` that holds a variant newer than its reader reads as unset, with a documented meaning:
  an unknown `Request.command` gets `ERROR_CODE_UNKNOWN_COMMAND`, an unknown `Input.kind` is
  ignored, an unknown `ClientMessage.kind` gets `ERROR_CODE_INVALID_REQUEST`, and a Client ignores
  an unknown `DongleMessage.kind` or `Event.kind`.
- `ErrorCode` is a wire enum, separate from the firmware's internal error types.

## Changing the schema

1. A field's number and type never change, and a field never changes meaning; a different meaning
   gets a new field.
2. A removed field, enum value or `oneof` variant is tombstoned with `reserved`, number and name,
   so neither is ever reused.
3. A new field's zero value, or its absence for an Optional field, means exactly what firmware did
   before the field existed.
4. A Required field is never removed.
5. New commands are new `Request.command` variants; old firmware answers them with
   `ERROR_CODE_UNKNOWN_COMMAND`. New events are new `Event.kind` variants, and new result types new
   `Response.result` variants; old Clients ignore them. A new kind of input is a new `Input.kind`
   variant, which old firmware ignores.
6. Old firmware ignores request fields it does not know, so a new request field must be safe to
   ignore. If ignoring it would do the wrong thing, the behavior gets a new command.
7. A command that needs more in its result than its record carries gets a new field on `Response`
   beside the `result` oneof.
8. A new error code is a new `ErrorCode` value at the end; old Clients show it as a generic
   failure.

`buf breaking` runs against the newest earlier release in the same series with the `WIRE_JSON`
rule set. It fails on changed field numbers and names, types that are incompatible on the wire or
in JSON, and fields, enum values and `oneof` variants removed without `reserved`, which covers the
mechanical part of rules 1 and 2; a changed meaning is for review. Before 1.0, a new minor version
starts a series and may break compatibility; from 1.0, only a new major version may. A development
build (`DEBONAIR_VERSION` 0.0.0) compares against the newest release. When there is no such
release, or it has no `proto/debonair.proto`, only `buf lint` runs. An invalid
`DEBONAIR_VERSION`, a `PROTOCOL_BASE` that names no commit, or a shallow clone in which no base
release is found, fails the check; a shallow clone may lack the release tags, so it needs them
fetched or `PROTOCOL_BASE` set. `buf lint` uses
its standard rules except `ENUM_ZERO_VALUE_SUFFIX`, because zero values carry the default meaning,
`PACKAGE_VERSION_SUFFIX`, because the package has no version, `PACKAGE_DIRECTORY_MATCH`, because
the schema lives in `proto/`, and `DIRECTORY_SAME_PACKAGE`, because the storage records' schema
sits beside it in its own package. Rules 3 to 8 and the rules below are review rules.

`make check-protocol` runs both checks, and `make check` includes it. `PROTOCOL_BASE` overrides the
release tag it compares against.

## Set commands

`SetConfig` is a partial update: a Client sends only the fields it wants to change, and every
missing field keeps its current value.

- Every updatable field is `optional`, never a plain scalar, so `false`, `0` or `""` cannot
  overwrite a stored value by accident.
- Because a missing field means unchanged, clearing a value needs an explicit form, such as
  `name: ""` restoring the default name.
- A success means the Dongle holds what was sent, so a Client sends a newer field only when it
  knows the firmware supports it, and a field that older firmware could not ignore safely gets a
  new command (rule 6).
- A new updatable setting is a new `optional` field on `SetConfig`, not a new command.

## Identifiers

The Dongle is identified by its adapter ID, `Status.id`. A Target is identified by its address and
address type together, as `Status.targets` reports them. Request ids belong to one session and mean
nothing outside it.
