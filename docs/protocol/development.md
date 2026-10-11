# Development commands

[Protocol index](README.md)

Development firmware, built with `PROFILE=development`, reports `Status.development` and accepts
`EnterBootloader`. Production firmware answers it with `ERROR_CODE_UNSUPPORTED`, and nothing at
runtime can enable it there.

| Command | Result | Behavior |
| --- | --- | --- |
| `EnterBootloader` | none | Responds, then restarts into the ROM programming mode: BOOTSEL on Pico boards, download mode on ESP32-S3. It works whether or not Bluetooth has started, and leaves saved data untouched. See [Restarts](commands.md#restarts) for when the restart happens. |

After `EnterBootloader`, the Client expects the Dongle to reappear in USB programming mode: as a
UF2 drive or to `picotool` on Pico boards, and to `esptool` on ESP32-S3. A missing response does not
prove the restart failed; check the USB mode before retrying.
