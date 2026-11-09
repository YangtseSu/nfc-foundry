# Phased implementation plan

Status legend: `done`, `in progress`, `pending`, `optional`.

| Phase | Scope | Status |
|---|---|---|
| 00 | Rust workspace, crate boundaries, docs, CI, REUSE | done |
| 01 | Core model, transport traits, mock reader, CLI skeleton | done |
| 02 | PN532 UART driver and reader lifecycle | in progress |
| 03 | Classic core: geometry, BCC, access, value, dump, diff | pending |
| 04 | Classic read/write/dump/restore/keyscan CLI + GUI workspace | pending |
| 05 | ACR122U USB/CCID driver and raw bits/parity | pending |
| 06 | Native recovery: Crypto1, darkside, nested, staticnested | pending |
| 07 | Native hardnested and FM11RF08S/static encrypted | pending |
| 08 | NDEF core: records, TLV, templates, validation | pending |
| 09 | Type 2 / NTAG read/write/NDEF + GUI | pending |
| 10 | Classic NDEF: MAD/CC/format + GUI | pending |
| 11 | Type 4 / ISO-DEP NDEF adapter | optional |
| 12 | Packaging, CI, hardware certification, release | pending |

## Rules

- One phase at a time unless work is genuinely independent.
- A CLI operation is incomplete until the GUI exposes the same operation with
  the same safety rules.
- Every phase needs a spec in `docs/specs/` and a note in `docs/notes/`.
- No driver or card family is advertised until its hardware test is recorded.
- `cargo test --workspace` must never touch hardware.
- Do not add an external recovery engine to the shipped product.

## Phase details

### 00: Foundation

Deliver:

- Rust workspace and crate skeleton.
- `AGENTS.md`, README, architecture, hardware, testing, packaging docs.
- GPUI Kit bootstrap window and CLI skeleton.
- GPL/REUSE metadata.
- CI job definitions for fmt, clippy, test, and REUSE.

Acceptance:

- `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace` pass.
- `cargo check -p nfc-foundry` passes.
- `reuse lint` passes.
- No `todo!`/`unimplemented!` placeholders; unimplemented operations return
  `NfcError::NotImplemented`.

### 01: Core and transport

Deliver:

- `CardInfo`, `CardType`, `Key`, `KeyType`, `NfcError`.
- `Reader`, `Transport`, `TrailerWriter`, `ManufacturerWriter` traits.
- `MockReader` with scripted card/block behavior.
- `nfc-app` device manager and exclusive operation skeleton.
- `nfctl list`, `scan`, `card info` against mock.

Acceptance:

- Transport traits are object-safe.
- Mock tests cover card change, authentication failure, timeout, cancellation,
  and read/write verification.
- No driver types leak into `nfc-app`.

### 02: PN532 UART

Deliver:

- Serial port discovery and exact connstring selection.
- PN532 command framing, ACK, retries, timeouts.
- Card selection and UID/ATQA/SAK.
- Classic authentication, block read/write.
- Raw bytes and bits/parity.

Acceptance:

- Hardware test reads a Classic card and a Type 2 tag.
- Wrong reader, missing card, timeout, and disconnect report stable errors.
- No write test without explicit environment confirmation.

Status: implementation and scripted tests complete. Hardware acceptance is open:
the attached CH340 adapter did not answer the PN532 wake-up and firmware
commands, so no card was selected. See `docs/notes/02-pn532-uart.md`.

### 03: Classic core

Deliver:

- 1K/4K geometry, access bits, BCC, value blocks, dump model, readability.
- Trailer validation, diff engine, dump project/raw/MCT/Proxmark3 exchange.
- Property tests for codecs and geometry.

Acceptance:

- Known vectors from the old implementation and independent fixtures match.
- Unknown bytes are never zero-filled.
- Diff classifies equal/changed/unknown correctly.

### 04: Classic workflows

Deliver:

- Read, write, dump, restore, keyscan, key store, backups.
- CLI `nfctl classic ...`.
- GPUI Kit Classic workspace with block table, inspector, diff, keys.

Acceptance:

- Read-back verification on every write.
- Restore stops at first failure; backup before mutation.
- GUI and CLI share the same services and safety preconditions.
- Hardware test on PN532 UART.

### 05: ACR122U USB

Deliver:

- USB enumeration and CCID transport with `nusb`.
- ACR122 escape/raw byte and bit commands.
- Card selection, Classic and Type 2 operations.
- Gen1A block 0 path.

Acceptance:

- Raw bits/parity verified against a known card.
- Gen1A UID write verified on a magic card with explicit confirmation.
- Disconnect/reconnect and PC/SC alias coexistence documented.

### 06: Recovery foundation

Deliver:

- Crypto1/PRNG from current Proxmark3.
- Darkside, nested, staticnested.
- Card/PRNG classification.
- Native `nfc-recover` engine and `nfctl recover` skeleton.

Acceptance:

- Golden nonce fixtures match PM3 outputs.
- Differential tests against old C engines during development.
- Cancellation and reader recovery work.
- No write to card during recovery beyond authentication.

### 07: Hardnested and FM11RF08S

Deliver:

- Hardnested acquisition and bit-sliced solver.
- PM3 hardnested tables with GPL notices.
- EV1 signature, static encrypted nonce, FM11RF08S paths.
- `auto` attack sequencing aligned with PM3 `autopwn`.

Acceptance:

- Hardnested recovers keys from known fixtures and hardware.
- Unsupported/static encrypted cards fail with actionable messages.
- Memory and CPU use bounded; progress and cancellation visible.
- Keys are verified in-process after recovery.

### 08: NDEF core

Deliver:

- Message/record codec: TNF, MB/ME/CF/SR/IL, chunking, type/id/payload.
- TLV parser/encoder: NULL, Lock Control, Memory Control, Proprietary,
  Terminator, short/long lengths.
- Templates: Text, URI, Smart Poster, vCard, Wi-Fi, BLE, DPP.
- Validation and capacity calculations.

Acceptance:

- Round-trip and malformed-input property tests.
- Unknown/proprietary TLVs preserved unless explicitly reformatted.
- NDEF core has no card or transport dependency.

### 09: Type 2 / NDEF

Deliver:

- NTAG213/215/216 and Ultralight page model, CC, user memory.
- Static/dynamic locks, OTP, config, PWD/PACK, AUTH0/ACCESS.
- NDEF TLV mapping, read/write/format.
- CLI `nfctl type2 ...` and `nfctl ndef ...`.
- GPUI Kit Type 2 memory and NDEF editor.

Acceptance:

- NTAG215 read/write/NDEF round trip on ACR122U.
- Lock/config/password actions are Advanced, backed up, confirmed, verified.
- Partial reads are explicit; no fake success.
- GUI and CLI parity.

### 10: Classic NDEF

Deliver:

- MAD1/MAD2 detection, CC parsing, NDEF sector mapping.
- Classic NDEF read/write/format with access-bit checks.
- CLI and GPUI Kit Classic NDEF view.

Acceptance:

- Blank Classic 1K format + NDEF write/read on hardware.
- No NDEF write to read-only or malformed mappings.
- Backup and read-back verification.

### 11: Type 4 NDEF (optional)

Deliver:

- ISO-DEP/APDU transport.
- Type 4 CC and NDEF file read/write.
- Reuse `nfc-ndef`.

Acceptance:

- Hardware validation before advertisement.
- No DESFire key/format workflow in this phase.

### 12: Release

Deliver:

- AUR source package and GitHub CI binary tarball.
- udev rules, license/notices, source archive.
- Hardware matrix and validation records.
- Migration note from the Go/C implementation.

Acceptance:

- Clean install on Arch Linux.
- PN532 UART and ACR122U pass the documented matrix.
- `cargo fmt`, clippy, tests, GPUI UI tests, REUSE, packaging checks pass.
