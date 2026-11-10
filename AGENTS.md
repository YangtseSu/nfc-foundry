# NFC Foundry Rust development guide

This file applies to the whole repository. It records the product, stack, and
constraints for the Rust rewrite.

## Product identity

NFC Foundry is a Linux-only NFC reader workbench for MIFARE Classic and NFC
Forum Type 2 / NDEF cards.

- Product and GUI: **NFC Foundry**
- GUI binary: `nfc-foundry`
- CLI: `nfctl`
- License: GPL-3.0-or-later

## Business domains

Two first-class domains share the transport and application layers:

1. **MIFARE Classic**: reader/card selection, authentication, read/write,
   dump/restore, keys, diff, clone, factory format, UID safety, and native key
   recovery.
2. **NDEF**: NDEF message/record/TLV core; Type 2 / NTAG and MIFARE Classic
   adapters; read, write, format, and protection. Type 4 NDEF reuses the same
   NDEF core later.

NDEF must not be implemented as an afterthought inside a Classic or Type 2 page
editor. It has its own codec crate and family-specific adapters.

## Fixed technical stack

- Rust latest stable, edition 2024, `rust-toolchain.toml` channel `stable`.
- One GUI dependency: **GPUI Kit**. Use `gpui_kit::*`, `component`, `base`,
  `assets`, and the GPUI Kit component system. Do not introduce MyGo, Wails,
  Electron, Tauri, WebView2, or a browser frontend.
- GUI view code is presentation only. Domain logic lives in `nfc-*` crates and
  `nfc-app`; the GUI calls application services.
- Transport is pure Rust. PN532 UART uses `serialport`. ACR122U uses direct
  USB/CCID via `nusb`. Do not add a C shim, CGO, `libnfc`, or `libnfc-ng`.
- Only two hardware targets in the first release:
  - `pn532_uart`
  - `acr122_usb` (direct USB/CCID)
- PN53x USB, PC/SC-only readers, and Proxmark3 device control are out of scope.
- Key recovery is native Rust in `nfc-recover`. Do not add external
  `mfoc`/`mfcuk`/`mfoc-hardnested` processes to the shipped product.
- Proxmark3 is not a device backend. Users run PM3's own client; NFC Foundry
  exchanges dumps and key files only.

## Version policy

- Track the latest stable release of the Rust toolchain, crates, GitHub
  Actions, GitHub-hosted runner images, and system libraries. Use the newest
  stable version, not an older LTS or a pre-release, unless a phase explicitly
  validates that pre-release and records why.
- Update `Cargo.lock` with the dependency change instead of deferring it. Do
  not pin an old version; when an incompatibility forces a pin, record the
  reason next to it and revisit it with the next update.
- Dependabot checks Cargo and GitHub Actions daily. Treat its pull requests as
  normal dependency work and land them promptly.
- Keep `rust-toolchain.toml` on the stable channel and keep the locked
  dependency set current.

## Recovery source policy

The source of truth for MIFARE Classic recovery behavior is the current
`RfidResearchGroup/proxmark3` tree, not the old `mfoc-hardnested` fork.

Port behavior from these areas:

- `common/crapto1/` for Crypto1/LFSR primitives.
- `client/src/mifare/mifarehost.c` for darkside, nested, staticnested, and
  card/nonce classification.
- `client/src/cmdhfmfhard.c` and `client/deps/hardnested/` for hardnested.
- `client/resources/hardnested_tables/` for hardnested tables.
- `client/src/mifare/mfkey.c` for mfkey32/mfkey64 variants.
- `client/src/cmdhfmf.c`, `client/src/cmdhfmfsen.c`, and
  `tools/mfc/card_only/` for card detection, static encrypted nonce,
  FM11RF08S, and attack sequencing.
- `doc/mfc_notes.md` for the card categories and which attack applies.

Do not port the PM3 firmware `CMD_HF_*` protocol. Implement equivalent
partial-auth/nonce acquisition over the Rust transport. The old C engines may
be used only as differential test oracles.

All PM3-derived code is GPL-3.0-or-later. Keep upstream notices and do not
relicense the derived work.

## Driver reference policy

`libnfc-ng` and the previous `nfc-foundry` implementation are behavioral
references for reader protocol work, not runtime dependencies. Use them for
PN532 UART and ACR122U framing, command parameters, register setup, and error
codes:

- `libnfc-ng`: `libnfc/drivers/` and `libnfc/chips/` for serial framing, PN532
  commands, register values, and bit/parity handling.
- previous `nfc-foundry`: `internal/nfc/libnfc/shim.c` and the validated
  Python scripts for the command sequences that worked on real hardware,
  including authentication and card-state handling.
- `nfc-research`: Proxmark3, mfoc/mfcuk, and card-family references for recovery
  and attack behavior; it is not a reader UART protocol source.

When hardware does not answer:

1. Run the known-good reference against the same exact port first
   (`LIBNFC_DEVICE=<connstring> nfc-list -v`, or the matching validated
   script).
2. Capture the reference host frames and compare them byte-for-byte with the
   Rust driver; check `LIBNFC_LOG_LEVEL=3` traces when available.
3. Only after the reference path works on that port, classify the Rust failure
   as a protocol, timing, or state bug.

Never report a reader as unsupported, missing, or blocked before the reference
path has been verified on that port. Record the reference trace and the Rust
trace in the phase note.

## Layer rules

1. GUI and CLI call `nfc-app` services. They never touch USB, serial, native
   pointers, or OS process APIs directly.
2. Card operations go through `nfc-transport` traits.
3. Recovery attacks go through `nfc-recover` trait boundaries and an exclusive
   reader lease.
4. `nfc-core`, `nfc-classic`, `nfc-ndef`, `nfc-type2`, and `nfc-classic-ndef`
   contain no hardware I/O.
5. `nfc-drivers` contains only transport/protocol mechanics; no Classic or
   NDEF workflow logic.
6. Do not let a lower crate depend on `nfc-app`, a GUI feature crate, or a
   sibling feature.

## Interface parity

- GUI and CLI are two surfaces over the same `nfc-app` services. Every card or
  file operation exposed by `nfctl` must have a GUI path with the same
  semantics, safety preconditions, and confirmation requirements.
- A CLI command without a matching GUI path is incomplete. Do not add a
  CLI-only card operation unless the plan explicitly marks it as a
  developer-only diagnostic outside the product workflow.
- Test fixtures (`--mock`), output formatting, and host/device selection flags
  may differ between surfaces; card and file semantics may not.
- The GUI may present an option differently, but it must not hide a capability
  or relax a gate.

## Hardware scope

### PN532 UART

- Serial framing, ACK, retries, timeouts, and reader recovery.
- Raw bytes and bits/parity required for Classic authentication and recovery.
- Device selection is an exact connstring; never write `/etc/nfc`.

### ACR122U

- Direct USB/CCID, not PC/SC, for the first release.
- Implement the CCID transport and ACR122 escape/raw commands needed for
  ISO/IEC 14443A bytes and bits.
- PC/SC is a future optional path only after raw-bit validation.

No driver is advertised as supported until it has an explicit hardware test
record for the operations it claims.

## Card and write safety

Every Classic write path must include:

- capacity and dump-length checks;
- block 0 BCC validation;
- sector-trailer access-bit complement validation;
- authentication before writing;
- sector trailers after ordinary data blocks;
- block 0 writes disabled by default; Advanced mode may expose supported magic
  card UID paths;
- explicit confirmation for UID, trailer, lock, configuration, and format
  actions, including Advanced actions;
- immediate read-back verification;
- stop on the first unrecoverable failure unless the user explicitly selected a
  continue policy;
- abort if UID, ATQA, or SAK changes mid-operation.

Type 2 rules:

- Default to user-memory access only.
- Configuration pages, lock bytes, password/PACK, and one-way locks are
  dangerous and Advanced-only.
- Back up the full tag before the first change.
- Every Advanced action shows the exact pages, old/new values, backup path, and
  read-back plan; destructive actions require typed confirmation.
- Genuine Type 2 UID pages are read-only. Magic-clone UID paths are a separate
  validated capability.

NDEF rules:

- Validate TLV lengths, capacity, record flags, chunking, and padding before a
  write.
- Preserve unknown/proprietary TLVs unless the user explicitly reformats.
- Read-only and lock state must be visible; do not silently overwrite a
  protected tag.
- A partial read is never reported as complete.

Advanced mode is default off and session-scoped. It resets on card removal,
device change, and application restart. It is a confirmation/discoverability
layer, not a protocol bypass.

## Testing

- `cargo test --workspace` must never touch real hardware or write a card.
- Hardware tests are feature-gated and environment-gated; see
  `docs/testing.md`.
- Pure codecs and workflows get unit/property tests.
- Reader drivers get mock transports plus explicit hardware tests.
- Recovery gets golden nonce/key fixtures and differential tests against the
  old C engines during development.
- NDEF gets malformed-input, capacity, chunking, TLV, lock-state, and hardware
  round-trip tests.
- GPUI Kit views get UI integration tests for interaction, focus, overlays, and
  state transitions.

Required before a change is complete:

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- GPUI Kit UI tests for changed screens
- `reuse lint`
- Hardware test for the changed reader/card path when applicable

## Documentation workflow

- `docs/plan/`: phased plan and acceptance criteria.
- `docs/specs/`: per-change specs.
- `docs/notes/`: implementation records, problems, and blockers.
- `docs/adr/`: architectural decisions.
- `docs/design/`: UI design references and GPUI Kit screen contracts.

When a driver interface, recovery source, packaging, or NDEF format changes,
update the plan, spec, notes, and README in the same change.

## Non-goals

- No Proxmark3 device integration.
- No external recovery engine in the shipped product.
- No `libnfc`/C shim.
- No Windows/macOS builds, installers, signing, or notarization.
- No telemetry, website, auto-updater, or account system.
- No browser-first UI, Wails, Electron, Tauri, or WebView2.
- No unrestricted bulk write outside Advanced mode.
- No claims of hardware support without validation.
