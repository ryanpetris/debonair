# Building Debonair

## Board presets

| Board preset | Bluetooth | Host stack and radio |
| --- | --- | --- |
| `pico_w` | Low Energy | BTstack on the CYW43439 through the Pico SDK driver |
| `pico2_w` | Low Energy | BTstack on the CYW43439 through the Pico SDK driver |
| `waveshare_rp2350b_plus_w` | Low Energy | BTstack on the CYW43439 through the Pico SDK driver |
| `xiao_esp32s3` | Low Energy | ESP-IDF's NimBLE host and controller |

Board definitions live in `rust/boards/`. They specify pins, flash allocation, clock settings and
the backends:

- `bluetooth_backend`: the Bluetooth host stack: `btstack` on Pico boards, and `esp-nimble` or
  `btstack` on ESP32-S3 boards. BTstack's license allows only non-commercial use without a
  commercial license from BlueKitchen; NimBLE, which ESP-IDF includes, is Apache-2.0 licensed.
- `radio_backend`: how the host reaches the controller. Pico boards use `pico-sdk-cyw43` (the Pico
  SDK's CYW43 driver) or `embassy-cyw43` (the Rust `cyw43` driver); ESP32-S3 boards use `esp-idf`.
- `usb_backend`: `embassy`, Embassy's USB device stack.
- `storage_backend`: `littlefs`, LittleFS in the flash after `firmware_bytes`, behind a guard block
  that holds the storage layout identity. The [storage format](storage-format.md) describes the
  layout and the records.

To use custom wiring or another backend, copy a preset, edit it and pass its path to
`python3 rust/tools/build_firmware.py`.

### Adding a Bluetooth backend

The firmware's application logic, in `rust/crates/debonair-core`, reaches Bluetooth only
through the `Radio` trait in `radio.rs`. A backend implements it: it starts the host stack with
the HID report map from `hid.rs`, advertises under the device combination's static random
address (`radio::static_address`), keeps each combination's pairings apart, sends encoded
reports, and turns the stack's callbacks into `radio::Event`s. There are two: `rust/adapters/btstack`,
and ESP-IDF's NimBLE in `rust/platforms/esp32s3/src/nimble.rs` with its C side in
`rust/platforms/esp32s3/components/platform/nimble_hid.c`. To add one:

1. Add an adapter crate under `rust/adapters/` that implements `Radio`.
2. Add the backend name to the board tools: `rust/tools/firmware_config.py` for Pico boards or
   `rust/tools/esp_config.py` for ESP32-S3 boards (with the SDK options it needs), and to
   `rust/tools/firmware_artifact.py`.
3. Add a Cargo feature with the same name to the platform crate and select the backend in its
   `main.rs`, as the platform crates already do for their radio backends.

## Build

Run commands from the repository root on Linux. Builds run in Docker, so the only prerequisites
are GNU Make, Git and [Docker](https://docs.docker.com/engine/install/) with your user allowed to
run it. `DOCKER=podman` uses Podman instead.

```sh
make desktop
make firmware BOARD=pico_w
make firmware-all
make package-desktop
```

Each build runs in a toolchain image from `docker/`, created on first use and reused afterwards.
The build copies the checkout into the image, leaving out the paths in `.dockerignore`, builds
there, and copies only its outputs back to the same places a local build would put them. Every
build downloads its dependencies and compiles from scratch.

The Pico image has Rust and ARM GCC; the ESP32-S3 image adds Rust, espup and ldproxy to
Espressif's ESP-IDF v6.1 image. Each firmware build fetches the pinned Pico SDK, CYW43 driver and
BTstack sources inside the image.

`BOARD` defaults to `pico_w`. `PROFILE` defaults to `development`; `PROFILE=production` leaves
out entering the bootloader from the app.

Firmware packages are written under
`build/firmware/<version>/<board>-<bluetooth-backend>-<radio-backend>/` for production and the
same directory with a `-debug` suffix for development firmware. They contain the ELF, binary,
artifact metadata, checksums, the license and dependency notices. Pico packages also contain a
UF2; ESP32-S3 packages contain bootloader and partition-table images plus `flash.json`, which
lists each image's flash offset. The artifact metadata records the board's storage layout identity
as `storage_identity`, and packaging checks it against the storage bounds.

### Erasing a board

Firmware updates that keep a board's storage layout identity keep its settings, Bluetooth
identities and pairings. The firmware provisions only flash whose storage range is entirely erased,
and never opens or reformats storage of another layout identity, such as when the board's flash
holds data from other firmware: Bluetooth then never starts and the adapter never becomes ready.
Flashing a board that has held other firmware or data therefore starts by erasing its whole flash:

- Pico boards: hold **BOOTSEL** while plugging the board in, run `picotool erase` (picotool 2.0 or
  later), then copy the UF2 onto the board.
- ESP32-S3 boards: hold **BOOT** while plugging the board in, run
  `esptool --chip esp32s3 erase_flash`, then flash the images with `esptool --chip esp32s3
  write_flash` at the offsets in `flash.json`.

The adapter then starts with new Bluetooth identities and nothing paired. Pair each device again,
removing the adapter from the device's Bluetooth settings first where the device keeps a pairing.

### Building without Docker

`make local-desktop` builds the desktop application with locally installed tools: Node.js 24 or
later with npm. `npm --prefix desktop run start` builds and starts it.

### Versioning

Builds use `DEBONAIR_VERSION` when supplied and otherwise `0.0.0`. The value is
`MAJOR.MINOR.PATCH`, without leading zeroes or a `v` prefix.

## Software checks

```sh
make check
make check-firmware-all
```

`make check` runs the Rust tests and Clippy, the desktop type checks and tests, and the protocol
compatibility checks, in the app image. Without Docker, `make check-rust`, `make check-desktop`
and `make check-protocol` run them with local tools. The protocol checks run `buf lint` on
`proto/`, and `buf breaking` against `PROTOCOL_BASE`: the newest earlier release tag in the same
series as `DEBONAIR_VERSION` (before 1.0 a minor version starts a series, from 1.0 a major
version), or the newest release for a development build. They skip `buf breaking` when there is
no such release or it has no `proto/debonair.proto`, except in a shallow clone, which may lack
the release tags: there they fail until the tags are fetched or `PROTOCOL_BASE` is set. See
[Compatibility rules](protocol/compatibility.md).
The Rust tests include a run of BTstack and the HID adapter against a simulated controller:
startup, a Target connecting, encryption, GATT discovery and an input report, and a paired
Target's saved bond, its Service Changed indication and a saved subscription.
`make check-firmware-all` runs Clippy on the firmware for every board and profile.

After changing `proto/debonair.proto`, regenerate the TypeScript messages:

```sh
npm --prefix desktop run generate
```

Rust code is generated from `proto/` at build time.

### Simulated adapter

`rust/crates/debonair-sim` runs the firmware's application logic on the host with a simulated
Target, speaking the serial API on standard input and output. The desktop app uses it in place
of a serial port when `DEBONAIR_SIMULATOR` names its executable:

```sh
npm --prefix desktop run simulate
```

The simulated Target pairs a moment after pairing starts and toggles Caps Lock. The desktop smoke
test launches the app against the simulator, checks that typed keys and key combinations reach
the simulated Target, and saves screenshots:

```sh
cargo build --manifest-path rust/Cargo.toml -p debonair-sim
npm --prefix desktop run build
xvfb-run -a npm --prefix desktop run smoke -- out/screenshots light
```

## Repository layout

- `rust/crates/`: the serial protocol, the board-independent firmware core and the simulator.
- `rust/adapters/`: BTstack, USB, storage and their tests.
- `rust/platforms/`: the Pico and ESP32-S3 firmware.
- `rust/boards/`: board presets. `rust/tools/`: firmware build tools.
- `desktop/`: the Electron, React and TypeScript application.
- `proto/`: the serial protocol messages and the records the adapter saves. `docs/`: protocol,
  storage format and build documentation.
- `docker/`: build images for the application and firmware.
