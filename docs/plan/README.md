# Phased implementation plan

Status legend: `done`, `in progress`, `pending`, `optional`, `blocked`.

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
| 10 | Classic NDEF: MAD/GPB/format + GUI | done |
| 11 | Type 4 / ISO-DEP NDEF adapter | in progress |
| 12 | Packaging, CI, hardware certification, release | pending |
| 13 | RF and transport completion: Type B, high rate, auto-poll, diagnostics | pending |
| 14 | Read-only protocol adapters: Type 4B/Type 3, EMV, T-Union, FMCOS/Calypso | optional |
| 15 | Read-only eMRTD reader (passport, compatible permits) | optional |
| 16 | Chinese resident ID read-only (authorized SAM) | blocked |
| 17 | Amiibo / NTAG215 read, decrypt, encrypt, write (user keys/data) | optional |

## Rules

- One phase at a time unless work is genuinely independent.
- A CLI operation is incomplete until the GUI exposes the same operation with
  the same safety rules.
- Every phase needs a spec in `docs/specs/` and a note in `docs/notes/`.
- No driver or card family is advertised until its hardware test is recorded.
- `cargo test --workspace` must never touch hardware.
- Do not add an external recovery engine to the shipped product.
- Phase 13 is the pending RF/transport prerequisite. It is not optional:
  phases 14 and 15 depend on its Type B and high-rate paths.
- Phases 14 and 15 are optional read-only work. Phase 14 exposes no write,
  credit, purchase, PIN, or cloning operation; phase 15 only reads eMRTD data.
  Phase 16 is blocked and read-only if it ever starts.
- Phase 17 is the optional controlled Amiibo path. It never ships Nintendo
  keys or Amiibo data; the user supplies both. Writes are Advanced,
  backup-first, read-back verified, and limited to user-owned magic NTAG215
  tags. It is independent of phases 13-15 and only needs the existing Type 2
  path.
- A new card family needs its own ADR, spec, note, and hardware record. It is
  never smuggled into the Classic, Type 2, or Type 4 adapter.
- Sensitive card data is not logged or persisted by default. PAN, MRZ, document
  numbers, and personal records are redacted or user-supplied only.
- Only cards owned by the user, or covered by explicit written permission, may
  be read. Chinese resident ID work additionally requires an authorized SAM and
  written authorization; it stays blocked until then.

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

- MAD1/MAD2 detection, GPB mapping-version/access parsing, NDEF sector
  mapping.
- Classic NDEF read/write/format with access-bit checks.
- CLI and GPUI Kit Classic NDEF view.

Acceptance:

- Blank Classic 1K format + NDEF write/read on hardware.
- No NDEF write to read-only or malformed mappings.
- Backup and read-back verification.

Status: complete. The Classic mapping has no capability-container record; the
MAD and per-sector GPB supply the mapping version and access conditions.
`nfc-classic-ndef` implements MAD1/MAD2, CRC validation, NFC sector mapping,
TLV framing, and area encoding. `nfc-app` adds confirmed format, NDEF write,
backups, key/access preflight, and read-back verification. `nfctl classic ndef
info|read|write|format` and the GPUI Classic NDEF tab share those services.
Unit, workflow, CLI, property, and UI tests pass. On 2026-11-10 the PN532 UART
path formatted a blank Classic 1K (UID `6A 31 5E 28`), wrote and read back a
Text record, restored the original dump with zero failures, and matched the
pre-test dump byte-for-byte. The ACR122U Classic NDEF record remains pending
because no ACR122U was connected. See `docs/specs/10-classic-ndef.md` and
`docs/notes/10-classic-ndef.md`.

### 11: Type 4 NDEF (optional)

Deliver:

- ISO-DEP/APDU transport.
- Type 4 CC and NDEF file read/write.
- Reuse `nfc-ndef`.

Acceptance:

- Hardware validation before advertisement.

Status: software complete; hardware record pending. `nfc-transport` adds
`ApduCapabilities`/`ApduResponse` and `Reader::transceive_apdu`; both PN532
UART and ACR122U carry command APDUs over `InDataExchange` without retrying
state-changing commands. `nfc-type4` implements the mapping-version 2.0 CC and
TLV parser, NLEN/message reads with MLe chunking, verified writes with MLc and
transport chunking, clear, and project JSON. `nfc-app` adds backups,
read-only refusal, confirmation for clear, read-back verification, and card
identity checks. Unit, property, driver, and scripted workflow tests pass. The
gated read/write tests are in `crates/nfctl/tests/hardware_type4.rs`; no reader
or card record exists yet, so Type 4 is not advertised. No CLI or GUI command
is added in this phase. See `docs/specs/11-type4-ndef.md` and
`docs/notes/11-type4-ndef.md`.

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

Priority after release: phase 13 first. Phases 14 and 15 are optional and
depend on phase 13; phase 16 stays blocked; phase 17 is optional and
independent after the Type 2 hardware record. A card adapter is not started
before its RF and transport dependency is validated on real hardware.

### 13: RF and transport completion

Goal: finish the RF and transport work that every later read-only adapter
needs. This phase is the priority; it does not interpret payment, transit, or
identity applications.

Deliver:

- PN532 UART and ACR122U Type B selection at 106 kbps: ATQB parsing, protocol
  byte, CID, targeted exchange, and RF-field recovery.
- `CardType` and `CardInfo` facts for Type B; no guessing an application from
  the UID or ATS.
- High-rate ISO-DEP at 212 and 424 kbps where the reader supports it: Type A
  and Type B `InListPassiveTarget` rates, RATS/PPS, ATS/FSCI handling, and a
  fallback to 106 kbps when a card or reader refuses the rate.
- Auto-poll and mixed-card scanning: report Type A, Type B, and FeliCa (if the
  reader path is validated) without a manual mode switch.
- APDU chaining and extended-length handling. The existing per-reader
  `max_command` limit stays authoritative; a reader that cannot carry an APDU
  reports it instead of splitting it silently.
- Reader control and diagnostics: RF field on/off, power-down/wake-up,
  hot-plug recovery, firmware and register reads, ACR122U LED/buzzer, PN532
  GPIO, timeout/retry reporting, and card-presence watch.
- A read-only APDU diagnostic surface limited to SELECT, READ BINARY, and READ
  RECORD. It must refuse every other instruction; it is not a write console.
  Card-specific reads that need another instruction live in phase 14 behind
  their own read-only allowlists.
- Scripted Type A/B cards, rate-negotiation tests, chaining tests, and
  hardware gates on both readers.

Acceptance:

- A Type B card is selected and a read-only APDU round trip is recorded on both
  PN532 UART and ACR122U.
- At least one reader completes a 212 or 424 kbps ISO-DEP exchange and falls
  back cleanly to 106 kbps when the card rejects the rate.
- Type 4B APDU reads work through the same transport; phase 14 maps them to
  NDEF.
- Auto-poll reports the card family without a write path.
- No card write, credit, purchase, PIN, or cloning path is introduced.
- New ADRs cover Type B, high-rate negotiation, and the read-only APDU policy.

### 14: Read-only protocol adapters (optional)

Goal: add read-only application and NDEF adapters on top of the phase 13
transport. No adapter in this phase performs a transaction, credit, purchase,
PIN, or card write.

Deliver:

- Type 4B NDEF read: the existing Type 4 adapter over the Type B transport.
- Type 3 / FeliCa NDEF read: PN532 first; ACR122U only after its FeliCa path
  is validated. Report an unsupported reader instead of guessing.
- EMV public-data reader:
  - PPSE/PSE selection, application selection, GPO, READ RECORD, and TLV
    decoding.
  - Type A and Type B contactless cards.
  - PAN, expiry, and cardholder data redaction in logs and default output.
- China T-Union reader:
  - Type 4A selection and AID `A000000632010105`.
  - Balance, up to 10 transaction records (file `0x18`), and up to 30 travel
    records (file `0x1E`), matching the Proxmark3 `read-t-union.py` behavior.
- FMCOS read-only: `info`, balance, and history for cards that expose the
  FMCOS file system. No credit, purchase, key write, or file write.
- Calypso, CIPURSE, and ITSO read-only adapters are optional follow-ups in
  this phase; validate a card before claiming one.
- Amiibo is an NTAG215 application and is already covered by the Type 2 raw
  read path; it needs no new RF work. Its decrypt/encrypt/write workflow is
  phase 17 and uses user-supplied Nintendo key material and data. No amiibo
  keys, decrypted dumps, or cloning operations are shipped.
- `nfctl` and GUI read-only views. Fixture tests for EMV TLV, T-Union records,
  Type 3/Type 4B NDEF, and FMCOS layouts. Live card data is never committed.

Acceptance:

- One owned Type 4B or Type 3 NDEF tag is read on a reader that supports its
  RF path.
- One owned EMV card and one owned T-Union card are read on at least one
  supported reader; both readers are targeted for the release matrix.
- EMV, T-Union, and FMCOS output is read-only and redacted by default.
- No EMV transaction, PIN, online authorization, T-Union credit, T-Union
  purchase, FMCOS financial operation, or card mutation exists.
- ISO 15693 / Type 5 and Type 1 / Topaz are not implemented: the supported
  readers do not provide those RF paths.

### 15: Read-only eMRTD reader (optional)

Goal: read ICAO 9303 electronic travel documents: passports, and compatible
Hong Kong/Macau permits if they present the same eMRTD application.

Deliver:

- ICAO 9303 LDS parsing, BAC and PACE, secure messaging, and data-group read
  and dump.
- Type A and Type B documents, with MRZ or CAN input supplied by the user.
- `nfctl` and GUI read-only views; explicit warning that only the document
  owner may use them.
- BAC/PACE vectors, malformed-file tests, and hardware gates on both readers.
- No terminal authentication, EAC, document write, or cloning. Some protected
  data groups may remain unavailable; report that explicitly.

Acceptance:

- `info` and `dump` succeed on one owned passport with the correct MRZ or CAN.
- A non-eMRTD Type B document is rejected without pretending to be readable.
- Hong Kong/Macau permits are supported only when the document presents the
  ICAO 9303 eMRTD application; a proprietary SAM-based permit stays out of
  scope.
- No document data is logged or persisted by default.

### 16: Chinese resident ID read-only (authorized SAM) (blocked)

This phase stays blocked. A second-generation resident ID requires an
authorized SAM, the corresponding keys, and written authorization for the
reader deployment. `hf 14b`/APDU access alone is not enough.

Prerequisites before any work starts:

- written authorization for the exact reader and use case;
- a supported reader with an accessible SAM interface;
- the authorized SAM and key material;
- an ADR, spec, and read-only safety contract.

No deliverable is scheduled until those prerequisites exist. The phase is
read-only even if unblocked: no ID data write, duplication, or authentication
bypass.

### 17: Amiibo / NTAG215 read, decrypt, encrypt, write (optional)

Goal: read an Amiibo NTAG215 image, decrypt or encrypt it with a user-supplied
amiitool-compatible key file, and write a user-supplied encrypted image to a
user-owned magic NTAG215. No Nintendo keys or Amiibo data are shipped.

This phase is independent of phases 13-15: Amiibo uses the existing Type 2 /
NTAG215 path, so it can start after the Type 2 hardware record without waiting
for Type B or eMRTD work. Phase 16 is blocked and unrelated.

Deliver:

- Type 2 / NTAG215 Amiibo profile: 135-page raw image, UID, lock bits,
  configuration, and capacity checks. The raw read/dump reuses the existing
  Type 2 workflow.
- Key import: an amiitool-compatible key file selected by the user, held only
  in memory, never logged, never written to project files, and zeroized after
  use where practical.
- Decrypt/unpack: verify the encrypted payload and produce the plaintext
  Amiibo image. A wrong key or corrupted tag is a typed error, not garbage
  output.
- Encrypt/pack: accept a user-supplied plaintext image, validate its size and
  fields, and produce the encrypted NTAG215 image.
- Write path: Advanced-only, explicit confirmation, full pre-write image
  backup, encrypted-image capacity and lock checks, read-back verification,
  and card identity check. Refuse genuine locked NTAG215 tags; only
  user-owned magic NTAG215 tags are writable. When the user-supplied image
  requires the original UID, a magic-tag UID write is a separate Advanced
  confirmation with the same backup and verify rules; it is never attempted on
  a genuine tag.
- CLI and GUI read-only/decrypt/encrypt views, plus the Advanced write action.
  The CLI uses a separate `nfctl amiibo` command family so the Type 2 page
  editor does not grow Amiibo-specific branches.
- Golden tests built from synthetic keys and synthetic images; no real Amiibo
  key file, plaintext dump, or retail tag data is committed. If an
  amiitool-derived algorithm is used, keep its MIT notice and license.
- Hardware gates: read/decrypt on both PN532 UART and ACR122U; write/verify on
  an owned magic NTAG215.

Acceptance:

- A user-supplied key file decrypts a user-supplied Amiibo dump and
  re-encrypts it without changing the underlying plaintext.
- A read-back-verified write completes on an owned magic NTAG215; the raw
  backup is written before the first mutation.
- No key material, plaintext Amiibo image, or retail dump is written to the
  repository, logs, or default output.
- A genuine locked tag is refused before any write.
- No genuine-tag UID write, lock bypass, or third-party Amiibo copying is part
  of this phase. A magic-tag UID write is Advanced, explicit, backed up, and
  verified.

Non-goals:

- Shipping Nintendo keys, decrypted Amiibo data, or retail dumps.
- Writing genuine locked Amiibo tags.
- Cloning or distributing third-party Amiibo data.
- Online Amiibo services or account integration.
