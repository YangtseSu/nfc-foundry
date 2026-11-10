# Phased implementation plan

Status legend: `done`, `in progress`, `pending`, `optional`.

| Phase | Scope | Status |
|---|---|---|
| 00 | Rust workspace, crate boundaries, docs, CI, REUSE | done |
| 01 | Core model, transport traits, mock reader, CLI skeleton | done |
| 02 | PN532 UART driver and reader lifecycle | done |
| 03 | Classic core: geometry, BCC, access, value, dump, diff | done |
| 04 | Classic read/write/dump/restore/keyscan CLI + GUI workspace | done |
| 05 | ACR122U USB/CCID driver and raw bits/parity | done |
| 06 | Native recovery: Crypto1, darkside, nested, staticnested | done |
| 07 | Native hardnested and FM11RF08S/static encrypted | done |
| 08 | NDEF core: records, TLV, templates, validation | done |
| 09 | Type 2 / NTAG read/write/NDEF + GUI | in progress |
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

Status: complete. Hardware validated firmware identification, Classic 1K and
Type 2 selection/reads, Key A authentication, and a 16-bit raw REQA response on
a PN532 (CH340 UART, firmware 1.6). Wrong-reader and missing-card errors were
checked on hardware; timeout/disconnect mapping is covered by scripted tests.
The same-value write test remains gated by explicit authorization and was not
run. See `docs/notes/02-pn532-uart.md`.

### 03: Classic core

Deliver:

- 1K/4K geometry, access bits, BCC, value blocks, dump model, readability.
- Trailer validation, diff engine, dump project/raw/MCT/Proxmark3 exchange.
- Property tests for codecs and geometry.

Acceptance:

- Known vectors from the old implementation and independent fixtures match.
- Unknown bytes are never zero-filled.
- Diff classifies equal/changed/unknown correctly.

Status: complete. The hardware-free model, codecs, dump certainty model, diff,
and four interchange formats are implemented with deterministic and property
tests. No hardware, CLI, or GUI operation is part of this phase. See
`docs/notes/03-classic-core.md`.

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

Status: complete. `nfc-app` owns dump, read, write, restore, key scan, key
store, and backup workflows; `nfctl` and the GPUI Classic workspace call the
same service methods. Scripted workflow, CLI, and GPUI UI tests pass. PN532
UART hardware validation on 2026-11-10 covered a 64/64-block dump, a 16-sector
key scan, a temporary data-block write with backups and read-back, and restore
of that block. Trailer writes and a 4K card run remain future hardware checks.
See `docs/notes/04-classic-workflows.md`.

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

Status: complete. Direct USB/CCID enumeration, the ACR122 pseudo-APDU command
layer, Classic and Type 2 selection/access, raw byte and bit exchange, and the
Gen1A `ManufacturerWriter` path are implemented. Hardware validation on
2026-11-10 used an ACR122U at `acr122_usb:001:017` with internal PN532 firmware
1.6: Classic 1K block read with a known key, Type 2 page read, raw 7-bit REQA
with a 16-bit ATQA, and a same-value Gen1A manufacturer-block write verified by
reselection and read-back. `pcscd` must be stopped because it holds the same
CCID interface. See `docs/notes/05-acr122-usb.md`.

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

Status: complete. Crypto1/LFSR/PRNG, `mfkey32`/`mfkey32nested`/`mfkey64`/
`nonce2key`, parity-disabled nonce acquisition on both drivers, software Crypto1
authentication, nested and static-nested recovery, darkside collection, card
classification, the recovery engine, and `nfctl recover
auto|darkside|nested|staticnested` are implemented. PM3 golden vectors pass.
ACR122U and PN532 UART hardware recovery recovered and verified known test keys
through nested, static-nested, and auto. Darkside reports no repeated-nonce
sample on host-driven readers. See `docs/notes/06-recovery-foundation.md`.

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

Status: complete. PM3 table loading, sum/bitflip filtering, the 64-lane
bit-sliced solver, fast-select nonce acquisition, EV1/QL88 signature detection,
static encrypted nonce detection, and FM11RF08S profile reporting are
implemented. `nfctl recover hardnested` and `recover fm11rf08s` are available.
Simulated fixtures and the full filtering/solver fixture recover the known key.
Hardware on 2026-11-10: ACR122U collected 1840 frames and recovered sector 0
Key A `A0A1A2A3A4A5` in 4m3s; PN532 UART collected 1816 frames and recovered
sector 5 Key A `FBBECDF6304E` in 5m29s. Both keys were verified with fresh
authentication. FM11RF08S key recovery remains detection-only because the
PN532 UART and ACR122U transports do not expose the reader-scheduled
acquisition; the strategy reports that limitation. A hard-PRNG card, an
FM11RF08S card, and a 4K EV1 card were not available. See
`docs/notes/07-hardnested.md`.

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

Status: complete. `nfc-ndef` implements the strict record/message codec with
short, normal, and chunked layouts; TLV framing with short and three-octet
lengths, unknown/proprietary preservation, and capacity calculation; and the
Text, URI, Smart Poster, vCard, Wi-Fi WSC, Bluetooth LE OOB, and DPP
templates. Record decode reports byte offsets, rejects reserved TNF values and
URI codes, and validates every length before slicing. NDEF TLV encoding
requires a valid NDEF message. Deterministic and property tests cover round
trips, malformed inputs, chunk ordering, unknown TLV preservation, and
capacity boundaries. See `docs/notes/08-ndef-core.md`.

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

Status: implementation complete; ACR122U hardware record pending. `nfc-type2`
owns CC/layout/lock/config parsing, project dumps, NDEF TLV preservation, and
verified read/write workflows. PN532 UART and ACR122U issue READ, WRITE, and
PWD_AUTH with CRC-A and restored raw mode. `nfc-app` adds backups,
confirmations, and identity checks; `nfctl type2|ndef` and the
`nfc-type2-gui` Memory/NDEF/Protection workspace use those services. Driver,
model, workflow, CLI, and GPUI tests pass. A PN532 UART NTAG215 read plus
authorized NDEF write/read/restore round trip passed on 2026-11-10; the
ACR122U variant remains to be recorded. See `docs/specs/09-type2-ndef.md` and
`docs/notes/09-type2-ndef.md`.

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
