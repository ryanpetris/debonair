# Storage format

The adapter keeps its settings, Bluetooth identities and pairings in upstream LittleFS through
`littlefs2` 0.8.1 on Pico and ESP32-S3 boards. The layout identity is `littlefs-1`. Each file holds
one protobuf message from [`proto/storage.proto`](../proto/storage.proto). The firmware never
reformats storage: storage of another layout identity stays unopened until the board's flash is
erased, as [building](building.md#erasing-a-board) describes.

## Flash allocation

Every erase block is 4096 bytes. The board configuration's `firmware_bytes` reserves the start of
flash for firmware; storage occupies the rest of flash after platform reservations. Its first block
is the guard, and LittleFS fills the others. Firmware linking and artifact inspection reject an
image that overlaps storage.

| Board | Flash | Firmware reservation | LittleFS blocks | LittleFS bytes |
| --- | ---: | ---: | ---: | ---: |
| Pico W | 2 MiB | 1 MiB | 255 | 1,044,480 |
| Pico 2 W | 4 MiB | 1 MiB | 766 | 3,137,536 |
| Waveshare RP2350B-Plus-W | 16 MiB | 1 MiB | 3,838 | 15,720,448 |
| XIAO ESP32-S3 | 8 MiB | 2 MiB | 1,535 | 6,287,360 |

RP2350 boards also reserve their final flash block for the ROM's E10 workaround. On the ESP32-S3,
the firmware reservation holds the bootloader, partition table, PHY data and application image, and
the partition table is:

| Name | Type | Subtype | Offset | Size |
| --- | --- | --- | ---: | ---: |
| `phy_init` | data | `phy` | `0xf000` | `0x1000` |
| `factory` | app | `factory` | `0x10000` | firmware reservation minus `0x10000` |
| `layout` | data | `0x40` | firmware reservation | `0x1000` |
| `storage` | data | `0x83` | firmware reservation plus `0x1000` | remaining flash |

The bootloader starts at `0x0` and the partition table at `0x8000`. There is no NVS partition: the
PHY is calibrated in full at every boot, and NimBLE keeps nothing in NVS.

## Layout identity

The guard's first 32 bytes hold the storage layout identity: the SHA-256 of the compact JSON
object, with sorted keys, of `format` (`littlefs-1`), `storage_backend` (`littlefs`), `offset` and
`bytes` (the storage range, guard included), `erase_bytes` (4096) and `guard_bytes` (4096). The
build tools compute it from the board configuration, `rust/tools/firmware_artifact.py` defines it,
and each firmware image embeds it and records it as `storage_identity` in its manifest. A board
whose storage moves or changes size therefore has another identity.

At startup the firmware reads the guard:

- An erased guard over an erased range is provisioned. The firmware checks that the entire guard
  and filesystem range is erased, writes the identity into the guard and reads it back, and only
  then formats LittleFS. Claiming the guard before formatting means an interrupted provisioning
  leaves either a mountable filesystem or a failure, never permission to format again.
- A guard holding this firmware's identity is mounted. A mount failure, including after an
  interrupted provisioning, leaves storage unavailable; the firmware never formats it.
- An erased guard over a range that is not erased, and a guard holding anything else, such as
  another layout's identity, leave storage unavailable and the flash untouched.

Once mounted, the firmware reads `/format.pb`. A filesystem that holds nothing else, or only an
empty `/format.pb`, gets `format: 1`. A root record that is not `format: 1`, or that is missing
beside other files, leaves storage unavailable.

Storage that is unavailable, for any of these reasons or because the storage range cannot be
found or read, reports its error for every access until the board restarts. Bluetooth does not
start, since the identity roots and pairings cannot be read: the adapter's status is not ready, its
link state is off and its status light stays off, and `SetConfig` changes are refused with
`ERROR_CODE_NOT_READY`. Erasing the board's flash and flashing it again recovers it.

## Files

| Path | Message | Contents |
| --- | --- | --- |
| `/format.pb` | `Format` | `format: 1` |
| `/adapter.pb` | `Adapter` | How the adapter appears to Targets; without the file, the name `Debonair`, a keyboard and a mouse, and Just Works pairing |
| `/combinations/<combination>/identity.pb` | `Identity` | The combination's identity roots |
| `/combinations/<combination>/bonds/<id>/bond.pb` | `Bond` | A Target paired with the combination; this file commits the bond's existence |

`<combination>` is `keyboard`, `mouse` or `keyboard-mouse`, the devices the adapter offers. Each
combination is a separate Bluetooth device to Targets, with its own static random address derived
from the board's unique ID, its own identity roots and its own pairings. IDs are positive decimal
integers of at most 32 bits, without padding or prefixes.

The schema's comments document each message and its fields. A field marked Required must be set; a
plain field reads its zero value as its default, and `optional` scalars and message fields have
explicit presence. Later firmware adds fields whose zero value or absence keeps the earlier meaning,
so records gain fields without a new layout identity, and the firmware ignores unknown fields. Keys
and addresses are bytes, most significant byte first.

Every record has a field writers always set, so no record encodes as an empty file: `format` in
the format record, at least one of `keyboard` and `mouse` in the adapter record, the roots of an
identity and the address of a bond.

## Adapter

`name` is a trimmed string of 1..29 UTF-8 bytes without control characters; an absent name uses
the name `Debonair`. `keyboard` and `mouse` select the devices the adapter offers, at least one
of them. `security` is Just Works or passkey pairing. A value that breaks its rule reads as its
default.

## Identity

The identity holds the combination's 16-byte identity root `ir`, encryption root `er` and the
identity resolving key `irk` the adapter distributes, which is AES-128 under `ir` of the block whose
last byte is 1, as BTstack derives it. BTstack uses the roots; NimBLE uses `irk`. A combination's
identity is created the first time the combination starts, with random roots:

- With BTstack, the security manager makes each root from the controller's random numbers once
  the controller is running, and the firmware saves the identity before it saves any bond.
- With NimBLE, the firmware makes the roots from the ESP32-S3's hardware random number generator
  with its SAR ADC entropy source enabled, since the radio is not yet running, and saves the
  identity before Bluetooth starts.

An identity whose `irk` does not match its `ir`, or a missing identity beside saved bonds, is
corrupt: a new identity would not match what Targets keep, so Bluetooth does not start.

## Bonds

A bond holds a Target's identity address and address type, the pairing's key size and whether it
was authenticated, authorized and used LE Secure Connections, and the keys each side distributed:
`local` for what the adapter distributed and `peer` for what the Target distributed. Each side holds
its LTK with EDIV and RAND, its IRK and its CSRK with the signing counter, each empty when not
distributed. The bond also holds Debonair's state for the Target:

- `feature`: the mouse feature report the Target last set, its wheel and pan resolution
  multipliers. Targets set it only when they first see the device, so the adapter restores it when
  the Target reconnects.
- `database`: identifies the GATT database the Target last discovered. When it differs from the
  running firmware's database, the adapter indicates Service Changed when the Target reconnects,
  and saves the new identifier once the Target confirms.
- `subscriptions`: the Client Characteristic Configuration values the Target wrote, by attribute
  handle, valid in the database `database` identifies.

The format does not depend on a Bluetooth stack; each stack saves what it keeps. BTstack keeps one
set of keys, in `peer`: the Target's keys after legacy pairing, or the shared key after LE Secure
Connections. Its `database` is the 16-byte GATT Database Hash, and it saves subscriptions keyed by
each configuration descriptor's handle. NimBLE keeps both sides' keys; its `database` is a 4-byte
hash of the report map and device combination, and its subscriptions stay in RAM.

A bond directory's ID orders the combination's bonds from the oldest pairing to the newest: a new
pairing, including a paired Target pairing again, is saved under an ID above every saved one, and
the record it replaces is removed once it is saved. Each combination keeps at most 8 bonds. A
pairing beyond them replaces the oldest.

A table entry whose pairing has not written all its keys holds an incomplete pairing, which is not
saved. The record the entry held stays until the entry holds a complete pairing, which is saved
and then replaces it, or until the entry is emptied, which removes it. BTstack writes an entry's
address and keys together when a pairing completes, so its entries are never incomplete. NimBLE
writes a pairing's two sides one after the other, and its bond is complete once the Target's keys
are written. When a paired Target pairs again, NimBLE deletes its keys before the new pairing
starts; the entry keeps the Target as an incomplete pairing, so its saved record stays. An entry
still incomplete when its Target disconnects is emptied, which removes the record.

## Native bond tables

The Bluetooth stacks never read or write storage. Their bond tables live in RAM and are views of
the saved bonds:

- BTstack's LE device database is a RAM table of 8 entries in `rust/adapters/btstack/c/bond_db.c`.
  Its TLV answers the security manager's identity and encryption roots from the identity record,
  accepts each root once when the combination has none yet, and maps the ATT server's subscription
  tags to the subscriptions of each entry. Other tags hold nothing, writing them fails, and saved
  roots are never replaced.
- NimBLE's security records are a RAM table of 8 entries served through `ble_hs_cfg`'s store
  callbacks in `rust/platforms/esp32s3/components/platform/nimble_hid.c`, and its key generation
  callback answers the identity's `irk`.

When Bluetooth starts, the firmware reads the combination's identity, then its bonds in ascending
ID order, and fills the table. Subscriptions whose `database` differs from
the running firmware's are left out. The stack reports the entries it changes, and the firmware
saves each entry as its bond's record.

## When files are written

- `SetConfig` writes `/adapter.pb` before it responds, so a successful response means the change is
  on flash. A change of the device combination then restarts the adapter.
- A combination's identity is written the first time the combination starts.
- A new pairing, changed keys and a removed bond are saved as soon as the stack reports them,
  before the application hears of the pairing. A pairing counts as made only once its bond, and
  with BTstack the identity roots its keys depend on, are saved. When that save fails, the firmware
  removes the pairing from the stack's table, which disconnects the Target, and reports the pairing
  as failed; the record of a bond that pairing replaced is then removed.
- Forgetting a Target removes its record before the response, which reports
  `ERROR_CODE_STORAGE_FAILED` when the removal fails; the removal is then tried again like any
  other. BTstack removes the Target from its table first. NimBLE's host task removes it from its
  table just after the record, without the firmware waiting; until then its entry is never saved.
- A Target sets its feature report, its database identifier and its subscriptions in bursts while it
  sets up its connection. A bond with such changes is saved once it has had no change for 250 ms,
  and at most 2 s after its first change, so one burst is one write.
- Everything waiting is saved before the adapter restarts or enters its bootloader.
- A save or removal that fails is tried again after 5 s; a later change to the same bond is tried
  at once. Each change is lost if power is removed before it reaches flash.

Saving the bytes a file already holds writes nothing.

## Commit and recovery

A file is saved in place: it is opened truncated, written and closed. LittleFS keeps an existing
file's old contents until the close commits the new contents in one metadata commit, so an
interrupted save leaves either the old or the new contents. Creating a file commits it empty before
its contents are written, so an interrupted creation can leave an empty file. Every record is a
nonempty message, and an empty file holds no record: it reads as missing.

Before saving, the firmware compares the file with the new bytes, first by length and then in
512-byte parts, and skips the write when they are equal. Old contents of the same length that
cannot be read are written over. A failed write of the contents leaves the file errored, so its
close commits none of them, and it is a definite failure. Other failures are resolved by mounting
again and comparing the file with the new bytes: equal bytes count as saved, and an empty file left
by a failed creation is removed, with its bond directory when that is left empty. An outcome that
cannot be read back is reported as unknown and treated as a failure, so the change is tried again.

Removing a bond's `bond.pb` commits its removal, and its now empty directory is removed after it.
When Bluetooth starts, the firmware removes, as far as it can, bond directories without a
nonempty `bond.pb`, bonds that do not decode or break the schema's rules, older bonds of an address
saved again, and the oldest bonds beyond 8. This cleanup is best effort: what stays is found and
removed again the next time.

LittleFS supplies its normal power-loss recovery and wear levelling, with block cycles of 500. The
number of records is bounded: 3 identities and at most 24 bonds beside the format and adapter
records, which take a small part of the smallest board's storage.

## RAM and execution

LittleFS's read and program caches are 512 bytes each and its lookahead is 16 bytes; they and the
library state stay allocated while the filesystem is mounted. The filesystem stays mounted between
operations, because a mount reads every metadata pair. A failed write, or a read error other than
a missing path, unmounts it, so the next operation mounts again from flash.

LittleFS reads flash in units of at least 128 bytes: its directory and metadata lookups ask for a
few bytes at a time, and each flash read has a fixed cost, so whole units take far fewer reads for
about the same number of bytes. A record is read into one buffer of the file's length and encoded
into one buffer of its exact encoded length; both reserve memory fallibly. The comparison before a
save uses a 512-byte buffer on the stack.

Filesystem operations are synchronous and run on the task that owns the application and the
Bluetooth stack's state, one at a time. A flash write blocks that task until it completes.
