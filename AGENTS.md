# NFC Foundry development guide

This file applies to the whole repository. It defines the product, the fixed
technical choices, and the constraints every change must follow.

## Project identity

NFC Foundry is a Linux-only NFC workbench for readers supported by the
`libnfc-ng` fork. It targets Arch Linux first: an AUR source package and a
GitHub CI binary tarball are the supported distribution channels.

The previous NFCX implementation is MIT-licensed and remains the behavioral
reference for MIFARE Classic work. Its old documentation is archived under
`docs/archive/nfcx-upstream/`. The new project is a separate product:

- Product and GUI: **NFC Foundry**
- Binary: `nfc-foundry`
- CLI: `nfctl`
- Native library fork: **libnfc-ng**
- License: GPL-3.0-or-later for this application

Only use cards, readers, and systems you own or have explicit permission to
test.

## Product scope

The initial product is a MIFARE Classic workbench:

- discover and open readers;
- select ISO/IEC 14443A cards and report UID, ATQA, SAK, and inferred type;
- MIFARE Classic 1K/4K authentication, block read/write, key scan, dump,
  restore, and read-back verification;
- key recovery through the existing `mfoc`, `mfcuk`, and `mfoc-hardnested`
  engines;
- guarded block 0 / UID handling for supported Gen1A and Gen2/CUID cards.

The next card family is NFC Forum Type 2: MIFARE Ultralight and
NTAG213/215/216 page access, user-memory read/write, lock/configuration
protection, and NDEF read/write. NDEF is a committed feature, not an optional
extra.

Later work may add ISO/IEC 14443-4 APDU handling, DESFire, FeliCa/NDEF, and
Jewel. Each family needs its own validated profile before it is advertised in
the GUI or CLI.

Card emulation is not part of the first milestones. It remains a valid future
direction for mobile Linux devices: keep target-mode and raw-frame transport
abstractions in the design so it can be added without rewriting the card
layer. Do not claim emulation support until it is implemented and validated on
real hardware.

Do not turn this repository into a general NFC research framework. Vertical,
validated workflows come before catalogue breadth.

## Fixed technical stack

- Go **latest stable**, never older than the version MyGo requires (currently
  1.27.3). MyGo is the desktop UI dependency and sets the minimum.
- Desktop UI: **MyGo native UI**. Do not reintroduce Wails, Electron, Tauri,
  WebView2, or a browser frontend as the primary GUI.
- Frontend logic lives in Go. The GUI is presentation only; it calls ordinary
  Go application services directly.
- NFC transport: the **libnfc-ng** fork through CGO and a narrow C shim.
- Card protocols and safety workflows: Go, above the transport boundary.
- Key recovery: external `mfoc`, `mfcuk`, and `mfoc-hardnested` processes.
- `nfc-mfsetuid` is replaced by native Go Gen1A code; do not ship the
  upstream utility.
- Packaging: Arch/AUR and a GitHub CI binary tarball. No Windows, macOS,
  Debian/RPM, AppImage, Flatpak, Snap, or container packaging.
- Tests: Go's standard `testing` package, mock readers, replay/fixture dumps,
  and explicit hardware tests. `go test ./...` must never touch real hardware
  or write a card.

All software uses the latest stable release: the Go toolchain, Go modules,
GitHub Actions, CI packages, and the external engines. Do not pin older
versions and do not adopt pre-releases. Dependabot checks Go modules and
GitHub Actions daily; review and apply upgrades when they are found outdated.

## Repository and package boundaries

The project is split into two independent source trees/packages:

```text
libnfc-ng       C library fork: device discovery, drivers, buses, raw transceive
nfc-foundry     Go application: GUI, CLI, card protocols, workflows, packaging
```

`libnfc-ng` is a standalone project, not a private copy inside the
application directory. It installs only the shared library, headers, and
pkg-config metadata. It does not install `nfc-*` command-line utilities.

Use a distinct library identity so it can coexist with Arch's `libnfc`:

```text
libnfcng.so.6
/usr/include/libnfc-ng/nfc/*.h
/usr/lib/pkgconfig/libnfc-ng.pc
```

The upstream `libnfc.so.6` ABI and package remain untouched. The C shim and
the external engines link through `pkg-config libnfc-ng`. If the fork ever
needs an ABI-breaking change, bump its soname instead of silently changing
the existing one.

`mfoc`, `mfcuk`, and `mfoc-hardnested` are built against `libnfc-ng` and are
redistributed as separate executables by the `nfc-foundry` package. Do not
turn them into separate AUR packages during the initial phase. They remain
out-of-process.

### Layer rules

1. The GUI and CLI call Go application services. They never touch CGO,
   libnfc, native pointers, or `os/exec` directly.
2. Ordinary NFC operations go through the `Reader`/`Transport` interfaces.
3. Key-recovery algorithms run only through `AttackEngine` implementations.
4. Native `nfc_context`, `nfc_device`, `nfc_target`, and libnfc unions stay
   inside `internal/nfc/libnfc`.
5. External engines never share a reader with the in-process transport.

## libnfc-ng rules

- The fork keeps upstream Windows/macOS source for reference and future
  optional use, but this repository builds and tests Linux only. Do not
  advertise untested platforms.
- Keep patches minimal and documented. The fork is source-of-truth, not a
  growing patch series.
- The library package must not install upstream utilities or examples.
- Keep `LIBNFC_DEVICE`-style exact device selection; never depend on or write
  `/etc/nfc`.
- Prefer system `libusb` and `pcsclite` on Linux. Do not bundle them.
- Dynamic linking only. Do not statically link libnfc-ng; LGPL obligations
  stay simple.
- The fork retains all upstream drivers and must not narrow the default driver
  set. Validation gates support claims and GUI/CLI capabilities, not
  compilation.
- The NFC Foundry package/build profile may select an explicit driver set.
  The initial profile is `pn532_uart,pn53x_usb,acr122_usb,acr122_pcsc,pcsc`;
  put that selection in `PKGBUILD`/CI, never in libnfc-ng's default.
- SPI/I2C, `pn71xx`, and legacy serial drivers remain available in the
  library. Do not add them to the initial application profile unless their
  dependencies and hardware are validated.
- Carry upstream PR versions of required patches when they exist. Never apply
  a patch that disables hardware or changes the default driver set.
- Only the conffiles/`LIBNFC_DEVICE` patch is required for Linux. macOS and
  Windows patches are optional platform maintenance; do not make them Linux
  release blockers or Linux CI requirements.

## C shim and transport

The C shim is deliberately small. It owns:

- device enumeration, open/close, and lifecycle;
- card selection and UID/ATQA/SAK retrieval;
- MIFARE Classic authentication and block access where the high-level libnfc
  API is sufficient;
- raw `transceive_bytes` and `transceive_bits` for protocols implemented in
  Go;
- raw-mode property control needed by Gen1A and other bit-level sequences;
- error mapping into stable Go error codes.

No libnfc pointer may cross the shim boundary. Return copied values only.

The Go transport interface should be card-family neutral:

```go
type Transport interface {
    ListDevices(ctx context.Context) ([]DeviceInfo, error)
    Open(ctx context.Context, connString string) error
    Close() error
    CardInfo(ctx context.Context) (CardInfo, error)
    Authenticate(ctx context.Context, block byte, keyType KeyType, key Key) error
    ReadBlock(ctx context.Context, block byte) ([16]byte, error)
    WriteBlock(ctx context.Context, block byte, data [16]byte) error
    TransceiveBytes(ctx context.Context, tx []byte) ([]byte, error)
    TransceiveBits(ctx context.Context, tx []byte, txBits int) ([]byte, int, error)
    SetRawMode(ctx context.Context, enabled bool) error
}
```

Card-specific protocols (Type 2 pages, DESFire APDUs, NDEF, Gen1A unlock) are
implemented in Go on top of this interface. Do not put card-family logic into
libnfc-ng.

## Device ownership

One reader is held by one operation at a time. The application has one global
device manager and one device lock shared by the GUI, CLI, background card
polling, and external engines.

For an external engine:

1. Stop background polling.
2. Close the in-process reader.
3. Start the engine with the exact connstring in `LIBNFC_DEVICE`, with autoscan
   and intrusive scan disabled.
4. Forward stdout/stderr live and support real process termination.
5. Validate the engine's exit status and output dump.
6. Clean temporary state on success, failure, and cancellation.
7. Reopen the reader.
8. Verify recovered keys and any written data with the in-process transport.

Never let the in-process reader and an external engine access the same device
at the same time.

Native Gen1A writing must not use this handoff. Extend the shim with the raw
primitives it needs and keep the reader in-process.

## MIFARE safety rules

Every write path must include:

- capacity and dump-length checks;
- block 0 BCC validation;
- sector-trailer access-bit complement validation;
- authentication before writing;
- sector trailers written after ordinary data blocks;
- block 0 writes disabled by default; Advanced mode may expose supported
  special-card UID paths;
- explicit confirmation for UID, trailer, lock, configuration, and format
  operations, including Advanced actions;
- immediate read-back verification;
- stop on the first unrecoverable failure unless the user explicitly chose a
  defined continue policy;
- abort if the card UID, ATQA, or SAK changes mid-operation.

A generic bulk-write entry may exist only behind Advanced mode. It must use the
family-specific preflight, full-image backup, read-back verification, and
confirmation rules; it must never bypass authentication, capacity, BCC,
access-bit, lock-bit, or card-identity checks.

For Type 2 cards, treat configuration pages, lock bytes, password/PACK, and
one-way lock bits as dangerous. Default to user-memory access only. Advanced
mode may expose configuration, dynamic-lock, static-lock, PWD/PACK, and
capability-container writes, but every one is one-way or potentially
card-bricking and requires a backup and explicit confirmation. UID/page 0-1
writes are not normal Type 2 WRITE operations: genuine NTAG pages are read-only.
A magic-clone UID path is a separate, explicitly identified backdoor and may
only be implemented after protocol validation.

## Advanced mode

Advanced mode is the user-authorized path for destructive, one-way, or
otherwise hidden operations. It is a confirmation and discoverability layer,
not a protocol bypass.

- Default off and session-scoped; reset on card removal, device change, and
  application restart.
- May expose: bulk writes, UID writes on compatible magic cards, static and
  dynamic lock bits, capability-container bits, configuration, PWD/PACK,
  formatting, and other irreversible operations.
- Every action shows the exact target, old/new values, backup path, and
  read-back plan; dangerous actions require an explicit typed confirmation.
- A full-image backup is written before the first change and the operation
  stops at the first unrecoverable failure.
- Hardware-impossible operations must fail honestly and must never report
  success. Do not emulate UID writes on genuine read-only pages.
- Advanced mode does not override authentication, capacity, BCC, access-bit,
  or card-identity checks. It only removes the default UI restriction.
- No "do not ask again" switch.

For DESFire or other ISO14443-4 cards, key changes and format operations are
destructive. Implement them only with a clear file/key model and a tested
recovery path.

## External engines

The only external engines in the initial product are:

- `mfoc`: Nested key recovery when at least one key is known;
- `mfcuk`: Darkside recovery for a first key;
- `mfoc-hardnested`: Hardnested recovery, verified through its output dump.

Run them with `exec.CommandContext` and argument arrays. Never use shell
concatenation. Keep raw output for the GUI/CLI log, but do not treat natural
language as the primary success signal:

1. check the exit code;
2. check the expected output file;
3. validate dump size, geometry, trailers, and access bits;
4. prefer keys and artifacts from validated dump files;
5. parse stable machine-readable lines only where no structured artifact
   exists.

If an upstream tool is too unstable, maintain a small fork that emits
`NFCX_RESULT`-style machine-readable records. Do not port the attack
algorithms into Go.

## GUI and CLI

MyGo native UI is the only desktop UI. The GUI must not contain card protocol
logic or process management; it renders state and invokes services.

GUI localization rules:

- English is the base language and the source of truth.
- Simplified Chinese is a first-class translation and must be kept complete.
- Every user-visible string goes through the i18n catalogue; no hard-coded
  Chinese-only UI strings and no translation lookups scattered through card
  or device logic.
- The language switch affects the GUI only. Logs, JSON payloads, and CLI
  output remain English.

The CLI is `nfctl`. It is the scripting and test surface and shares the same
services as the GUI. The CLI is **English-only**: help text, errors, progress,
logs, machine-readable keys, and documentation. Do not add locale-dependent
output or translated CLI messages. Useful commands:

```text
nfctl list
nfctl scan [--watch]
nfctl card info
nfctl classic read|write|dump|restore|keyscan
nfctl type2 read|write|dump
nfctl uid set
nfctl recover nested|darkside|hardnested
nfctl diag
nfctl pn532 shell
nfctl apdu
nfctl ndef read|write
```

Use `--device`, `--json`, `--yes`, and `--dry-run` consistently. Destructive
operations require explicit confirmation. Exit codes must distinguish usage,
validation, permission, hardware, and engine failures.

The GUI must cover every user-facing capability in the CLI. A new CLI command
is incomplete until the GUI exposes the same operation with the same preflight,
confirmation, backup, and verification rules. Machine-readable output, exit
codes, and shell automation are CLI-only; they do not excuse a missing GUI
workflow.

Do not reimplement upstream demo utilities, including their emulation examples,
relay, DEP, SAM, or barcode experiments. Card emulation as a product capability
is a conditional future phase (Phase 09), not a global non-goal; the initial
CLI does not expose it.

## Packaging and distribution

Supported outputs:

- an AUR source package for `libnfc-ng`;
- an AUR source package for `nfc-foundry`;
- a GitHub CI binary tarball built on Arch Linux.

Do not add other distribution formats without an explicit product decision.
Use system `libusb` and `pcsclite`; the `nfc-foundry` package bundles only the
application, the three recovery engines, and their notices. Install udev rules
under `/usr/lib/udev/rules.d/`. `pcscd` is an optional runtime dependency and
must produce an actionable error when it is missing.

`libnfc-ng` is a pure library package: shared library, headers, pkg-config.
No `nfc-*` utilities. `nfc-foundry` contains the GUI, `nfctl`, external
engines, and no private copy of libnfc-ng.

## Licensing

- NFC Foundry: GPL-3.0-or-later.
- libnfc-ng: LGPL-3.0-or-later (fork of libnfc; keep upstream notices).
- mfoc, mfcuk, mfoc-hardnested: GPL-2.0-or-later, distributed as separate
  executables.
- nfc-mfsetuid code, if ported: BSD-2-Clause; retain the notice in the Go port.
- NFCX reference code: MIT; retain notices for copied or translated logic.
- MyGo: MIT.

Every release must include the corresponding source, license texts, pinned
versions, and any NFC Foundry patches. Do not add restrictions that prevent
users from modifying or redistributing GPL-covered code.

The repository uses the [REUSE Specification](https://reuse.software/):

- `REUSE.toml` carries the path-based copyright and license annotations.
- `LICENSES/` contains the SPDX-named license texts.
- `reuse lint` must pass before a change is considered complete.
- New files are covered by `REUSE.toml` or carry SPDX headers. Do not add an
  unannotated file.
- `NOTICE` and `THIRD_PARTY_NOTICES.md` preserve the NFCX MIT attribution and
  third-party notices.

## Testing and verification

Required before a change is considered complete:

- `go test ./...` with the hardware-free build;
- unit tests for card geometry, BCC, access bits, dump validation, command
  framing, and cancellation;
- replay/fixture tests for external engine output and dump validation;
- explicit hardware tests for the changed reader/card path, enabled only by
  environment variables or build tags;
- a CLI smoke test for any changed command;
- a MyGo UI smoke test for any changed screen;
- `reuse lint` must pass;
- `git diff` review for accidental platform, license, or telemetry changes.

Hardware tests must never run automatically in ordinary CI and must never
write a card unless the operator explicitly enables the write test.

## Documentation workflow

- `docs/plan/`: phased product plan and acceptance criteria.
- `docs/specs/`: per-change implementation specs.
- `docs/notes/`: implementation records, including problems and blockers.
- `docs/archive/`: historical NFCX material; never update it to describe the
  new product.

When a C shim, ABI, packaging, or external-engine interface changes, update
the plan, spec, notes, and README in the same change.

## Non-goals and prohibited shortcuts

- No Windows or macOS builds, installers, signing, or notarization.
- No Wails, Electron, Tauri, or browser-first UI.
- No telemetry, website, auto-updater, or account system.
- No `libnfc` utility installation.
- No static linking of libnfc-ng or GPL engines into the application.
- No shell command construction from user input.
- No unrestricted bulk write outside Advanced mode; Advanced bulk operations
  still use family-specific preflight, backup, and read-back checks.
- No full DPP/Easy Connect protocol stack. Write and read DPP URIs as NDEF data;
  do not implement DPP authentication or provisioning.
- No claims of unvalidated card or reader support; implementation and tests may
  exist behind explicit Advanced/hardware gates.
- No unlicensed copying of upstream code or binaries.
