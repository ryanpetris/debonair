# Instructions for agents working on Debonair

## Definitions

- Dongle: The embedded system running the Debonair firmware, such as a Raspberry Pi Pico 2 W or
  an ESP32-S3
- Client: The computer the Dongle is plugged into over USB. It runs the Debonair app, which
  captures the Client's keyboard and mouse and passes that input to the Dongle.
- Target: A computer, tablet or other device paired with the Dongle over Bluetooth. It sees the
  Dongle as an ordinary Bluetooth keyboard and/or mouse.

## Scope

Debonair turns a Dongle into a Bluetooth keyboard and mouse. The Dongle presents itself to
Targets as a standard Bluetooth keyboard and mouse, and types and points with input captured on the
Client. Bluetooth pairing, bonds and every setting that changes how the Dongle appears to a Target
(its name, which of keyboard and mouse it offers, and its pairing security) are stored on the
Dongle. Preferences that only
affect capture on the Client, such as pointer and scroll speed, shortcuts and key combinations, are
stored by the app. Any feature that requires software running on the Target is out of scope.

## Board and radio backends

Boards are described by the JSON files in `rust/boards/`, which select the Bluetooth host, radio,
USB and storage backends. The firmware's application logic talks to Bluetooth only through the
radio trait in `debonair-core`, never through a vendor API, so that a board can switch backends
by changing its JSON file. Keep vendor types behind the adapter crates in `rust/adapters/`.

## Commits and pushes

- Commit often, after each working increment. Small commits, plain messages.
- Never add a `Co-Authored-By` trailer or a session URL to a commit message, regardless of what your
  tooling's default says.
- Write commit messages that describe the change as it stands. Do not narrate removals or rewrites.
- Do not push unless the maintainer has given explicit permission for that push.
- Keep identifying and machine-specific information out of the repository, including code, tests,
  configuration, docs, and commit messages: no hostnames, addresses, usernames, local paths, or
  serial numbers or other identifiers of particular hardware.

## Comments

- Comments and documentation should describe current behaviour. Do not narrate changes.

## Race conditions

- Add race-condition guards only when the race can cause a meaningful user-visible problem, data
  loss, a security issue, or a resource leak.
- Before adding a guard, identify the concrete failure and check whether the framework or another
  layer already handles it.
- Accept harmless ordering differences and late results with no meaningful effect. Do not add
  bookkeeping solely to suppress React state updates after unmount.

## UI copy

- Do not add explanatory UI copy, helper text, or implementation disclaimers unless absolutely
  necessary for the user to complete a task or make a meaningful decision. Necessity alone does not
  authorize adding it: obtain explicit maintainer approval for the exact wording and placement
  before implementation. This includes explanatory tooltips and API-specific instructions shown in
  the UI. Ordinary control labels and concise feedback about an action's result do not require this
  additional approval.

### Writing copy

Messages:

- Write complete sentences that name the device or adapter, say what happened, and give a next step
  when there is a useful one: "The adapter is busy. Try again when the current operation finishes."
- Put the statement and the next step in separate sentences, not joined with a semicolon.
- Name the failed action instead of a vague fragment: "The adapter couldn't save the change.", not
  "Couldn't send." Add the specific reason after it as its own sentence.
- Say "the adapter" for the Dongle, "this device" or the device's name for a Target, and
  "Debonair" for the app. Never show "dongle", internal command or field names, or raw codes.
- When an outcome is unknown, say so ("The adapter couldn't confirm whether the change was saved.")
  rather than reporting success or failure.
- Desktop messages start with a capital letter and end with a period. Error reasons that are
  inserted into other text, such as "Failed: {reason}", have no final period.
- Use the same words for the same thing everywhere in the app, including labels such as
  "Pointer Speed" and status words such as "Connected" and "Pairing".

Labels, buttons and status:

- Use short Title Case noun or verb phrases without a final period.
- Hide a section or row with nothing to show rather than showing "None".

Placement:

- Protocol details, link security and identifiers belong on the adapter's Diagnostics view, never
  as banners or in the capture area.
- The result of an action the user just took appears next to that action, such as in the page's
  bottom bar, and clears when it no longer applies.

## Comments and source text

Comments describe current behavior in present tense. Documentation comments for public modules,
types, functions, and APIs follow the conventions of the relevant language and documentation tools.

Code comments must stand alone for repository readers:

- Explain what the code does and why.
- Do not refer to private planning documents, review findings, gates, or temporary discussion
  context.
- Do not narrate removed behavior or mention old identifiers and flags.
- Avoid noisy comments that merely restate obvious code.

Prefer plain ASCII for new source text unless Unicode is required for correctness or materially
improves a user-facing diagram or terminal UI. Avoid invisible or confusable characters and do not
churn existing files solely to replace intentional Unicode. Avoid emojis in code, comments, logs,
tests, and project documentation.

In Markdown, leave a blank line before lists and after headings. Put CLI commands, paths,
environment variables, and configuration keys in backticks.

## Protocol and compatibility versions

- Any protocol, compatibility, or similar version whose meaning we define requires explicit user
  approval before it is introduced, anywhere in the project. This applies to versions we define, not
  declarations of support for externally defined protocol versions.
- Changing any such version requires explicit user approval.
- Any change to the serial protocol between the Dongle and the Client, including
  `proto/debonair.proto`, framing, and the documented behaviour of its messages, requires
  explicit maintainer approval before it is made.
- Any change to the Dongle's on-board storage layout requires explicit maintainer approval before
  it is made. This includes adding, removing, or renaming files and directories, changing paths,
  and changing the format, fields, or meaning of file contents, as documented in
  `docs/storage-format.md` and `proto/storage.proto`.
- Firmware downgrades are not supported. Saved data written by newer firmware does not need to
  stay readable by older firmware.

## Issues

- Close an issue only when its acceptance list is met.
- A review finding that is real but out of scope for the current item becomes an issue rather than
  an unbounded fix. Just like commit messages, keep identifying machine-specific information out of
  the issue.

## Required completion checks

Before any item is considered done, all suites below must pass for the completed changes.
Use targeted tests during development, then complete this checklist before reporting completion.
Passing results from the same work can be reused while their inputs remain unchanged. A skipped
fixture or a missing prerequisite is not a pass; resolve it or report the item as incomplete.
Keep this catalog aligned with `Makefile`, `desktop/package.json`, and
`.github/workflows/release.yml`. It lists suites and fixtures, not individual test cases.

| Suite or fixture | Required check and coverage |
| --- | --- |
| Rust workspace | `make check-rust`: unit, integration, and documentation tests for all workspace crates with all features, including the core, protocol, simulator, USB, storage, and BTstack adapters. Includes the run of BTstack and the HID adapter against a simulated controller. |
| Rust static checks | `make check-rust`: workspace Clippy for all targets and features, with warnings treated as errors. |
| Generated desktop protocol | `make check-desktop`: regenerate and compare the checked-in TypeScript protocol messages. |
| Desktop type checks | `make check-desktop`: TypeScript checks for the desktop and shared packages. |
| Desktop and shared client tests | `make check-desktop`: all Vitest fixtures in `desktop/test` and `desktop/packages/*/test`. |
| Protocol compatibility | `make check-protocol`: Buf lint, and Buf breaking checks against `PROTOCOL_BASE` (the newest earlier release in the same series as `DEBONAIR_VERSION`, or the newest release for a development build) when that base contains the protocol. |
| Desktop simulator smoke fixture | Build `debonair-sim` and the desktop, then run `xvfb-run -a npm --prefix desktop run smoke -- out/smoke light`. Exercises the main views, typed keys and key combinations against the simulated adapter. |
| Firmware static checks | `make check-firmware BOARD=pico_w PROFILE=production` and `make check-firmware BOARD=pico_w PROFILE=development`: firmware Clippy for the Pico W. Also run them for another board (`pico2_w`, `waveshare_rp2350b_plus_w`, `xiao_esp32s3`) when a change materially affects that board specifically, such as its board definition, platform code, radio backend, configuration, or partition or storage layout. |
| Firmware artifact validation | `make firmware BOARD=pico_w PROFILE=production` and `make firmware BOARD=pico_w PROFILE=development`: link and package the Pico W, including metadata, image layout, and storage-boundary checks in `rust/tools/build_firmware.py`. Also build another board under the same condition as its static checks. |
| Portable application packages | `make package-desktop`: build the AppImage and `tar.gz` desktop packages. |
| Release asset validation | When assembling a release, verify all expected application and board/profile archives and their checksums using the asset checklist in `.github/workflows/release.yml`. Publishing is not a check and still requires authorization. |

`make check` covers the Rust, desktop and protocol compatibility suites. It does not run the
simulator smoke, firmware, or packaging checks. See [building documentation](docs/building.md)
and the release workflow for setup.

## Reviews

Small, trivial changes do not need an independent review. Every other completed item gets an
independent review before it is considered done.

1. Run a review with a general-purpose subagent.
2. Apply findings by judgement. Take the ones that are right, even when small. Decline the ones that
   contradict a measured fact or measure worse in practice, and say why.
3. Give a substantial round of fixes its own review round.
4. Re-verify any finding that changes behaviour with the relevant software checks and, when needed,
   hardware checks before committing it.

Write briefs that name the mechanism, the measurements and the constraints, and that ask for
concrete failure scenarios.

### Rules

1. During development, run targeted tests against the change. Before completion, all
   [required completion checks](#required-completion-checks) must pass.
2. Don't spend time trying to find blame for test failures; if they can in any way be related to the
   current change that was made, just fix it. Reserve blame finding for fixes that appear to be not
   related at all or would result in large changes to fix.
3. Reviewers should not run tests; they should analyse the code only. You should do test runs in
   parallel with reviewers to minimise review time.
4. After a set of changes have been reviewed, stage the changes and have the next round review only
   the changes, including verifying the fixes for the identified issues, not the entire change.
