# Phased implementation plan

Status legend: `done`, `in progress`, `pending`, `optional`, `conditional`,
`blocked`.

Phase files: phases 00-17 are planned and recorded as sections in this file,
not as separate `docs/plan/NN-*.md` files. Phases 18, 19, and 24-31 have
separate plan files because they are large multi-slice efforts. Notes exist
for phases 20-23 and are linked from their sections below.

GUI note: **GTK4-rs (`gtk4` + `libadwaita`) is the selected and only GUI
stack** as of [ADR 0006](../adr/0006-gtk4-rs.md). The GPUI Kit workspaces were
removed on `feat/gtk4-ui`. Phase 19 is the GTK4-rs workbench umbrella in
`crates/gui`, executed as slices. 19a (Classic completion/PM3 interchange),
19b (NDEF write/format), 19c (Type 2 workbench), 19d (recovery/system
surfaces), and 19e (bilingual UI, parity, packaging) are complete. There is no
separate "19" implementation step outside those slices. Phases 04/09/10/18
describe historical GUI surfaces and remain historical for domain and CLI
work; their current equivalents are the GTK4 workspaces.

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
| 09 | Type 2 / NTAG read/write/NDEF + GUI | done |
| 10 | Classic NDEF: MAD/GPB/format + GUI | done |
| 11 | Type 4 / ISO-DEP NDEF adapter | optional; PN532 validated 2026-11-12, ACR122U pending |
| 12 | Packaging, CI, hardware certification, release | done |
| 13 | RF and transport completion: Type B, high rate, auto-poll, diagnostics | pending |
| 14 | Read-only protocol adapters: Type 4B/Type 3, EMV, T-Union, FMCOS/Calypso | optional |
| 15 | Read-only eMRTD reader (passport, compatible permits) | optional |
| 16 | Chinese resident ID read-only (authorized SAM) | blocked |
| 17 | Amiibo / NTAG215 read, decrypt, encrypt, write (user keys/data) | optional |
| 18 | Workbench UI redesign, bilingual UI, icon and desktop entry | done (GPUI; GUI superseded by 19) |
| 19 | GTK4 workbench: shell, Classic slice, then parity screens | done (19a–19e) |
| 19a | Classic completion: PM3 interchange, keys, diff/restore, safety, UI tests | done |
| 19b | NDEF write/format: composer, Classic NDEF, read-only gates | done |
| 19c | Type 2 / NTAG workbench and hardware-gated Type 4 read | done |
| 19d | Recovery, keyscan, diagnostics, activity, settings | done |
| 19e | Bilingual UI, parity audit, UI tests, packaging | done |
| 20 | All-sector autopwn, host-driven darkside, recovery UI | done |
| 21 | Wi-Fi WSC decoding, selectable card facts, Activity copy | done |
| 22 | Grouped primary navigation and header menu | done (superseded by 23) |
| 23 | Card-family workspaces and NDEF placement | done |
| 24 | Parity recovery: verified keys, workbench, UID, reader lifecycle, diagnostics | in progress (S1, S2, S3a, S3b, S4, S6 done) |
| 25 | Owned-card support: 4K clone, Ultralight EV1, FM11RF08S recovery, DESFire EV3 NDEF, UID NTAG213 | done (PN532; ACR122U rows pending) |
| 26 | DESFire workbench: read/write, app/file/key management, secure messaging, NDEF | in progress (26.1–26.9 done; 26.10–26.13 pending) |
| 27 | NDEF workbench: multi-record editor, RTD forms, `.ndef` load/save, Classic/Type 2 write | done |
| 28 | ACR122U PC/SC transport (ACR122U only, not generic PC/SC) | pending |
| 29 | CLI machine contract, cross-process reader ownership, cancellation/tasks, developer raw diagnostics | pending |
| 30 | Workbench/file parity: sector grouping/ASCII, dump sidecars, export all candidates | pending |
| 31 | Conditional scope register: card emulation, Jewel/Topaz | conditional |

## Rules

- Phase 28 adds **ACR122U PC/SC only**. Generic PC/SC readers and PN53x USB
  stay out of scope; ADR 0007 supersedes the ACR122U PC/SC exclusion in ADR
  0002.
- Phase 29 developer raw tools (`pn532 shell`, arbitrary `apdu --raw`) are
  non-default features and never ship in release packages.
- Phase 31 records conditional directions only; neither card emulation nor
  Jewel/Topaz is in the support matrix until a phase is accepted.
- One phase at a time unless work is genuinely independent.
- A CLI operation is incomplete until the GUI exposes the same operation with
  the same safety rules.
- `nfctl` is English-only. The GUI is English-base with a complete Simplified
  Chinese (`zh-CN`) translation and an in-app language switch.
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
- Phase 18 is the historical GPUI workbench redesign and bilingual UI. Its GUI
  implementation was removed on `feat/gtk4-ui`; phase 19 is the current GTK4
  workbench. Phase 19 is independent of phases 13-17 and can proceed before
  them. The application icon and desktop entry are installed when the GTK
  application is packaged.
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
and static encrypted nonce detection are implemented. `nfctl recover
hardnested` and `recover fm11rf08s` are available.
Simulated fixtures and the full filtering/solver fixture recover the known key.
Hardware on 2026-11-10: ACR122U collected 1840 frames and recovered sector 0
Key A `A0A1A2A3A4A5` in 4m3s; PN532 UART collected 1816 frames and recovered
sector 5 Key A `FBBECDF6304E` in 5m29s. Both keys were verified with fresh
authentication. Update 2026-11-12: phase 25.3 replaced the detection-only
FM11RF08S strategy with native backdoor acquisition and one-nonce recovery on
PN532 UART. The without-backdoor distance path remains unclaimed because
host-driven timing did not make it reliable. A hard-PRNG card and a 4K EV1 card
were not available in phase 07. See `docs/notes/07-hardnested.md` and
`docs/notes/25c-fm11rf08s.md`.

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

Status: software complete; PN532 hardware record 2026-11-12 with an NTAG 424
DNA, ACR122U pending. `nfc-transport` adds
`ApduCapabilities`/`ApduResponse` and `Reader::transceive_apdu`; both PN532
UART and ACR122U carry command APDUs over `InDataExchange` without retrying
state-changing commands. `nfc-type4` implements the mapping-version 2.0 CC and
TLV parser, NLEN/message reads with MLe chunking, verified writes with MLc and
transport chunking, clear, and project JSON. `nfc-app` adds backups,
read-only refusal, confirmation for clear, read-back verification, and card
identity checks. Unit, property, driver, and scripted workflow tests pass. The
gated read/write tests are in `crates/nfctl/tests/hardware_type4.rs`; the
PN532 UART read/write/restore record exists, the ACR122U path does not. Type 4
is not advertised because no CLI or GUI command is added in this phase. See
`docs/specs/11-type4-ndef.md` and `docs/notes/11-type4-ndef.md`.

### 12: Release

Deliver:

- AUR source package and GitHub CI binary tarball.
- udev rules, license/notices, GitHub-generated source archives.
- Hardware matrix and validation records.

Acceptance:

- Clean install on Arch Linux.
- PN532 UART and ACR122U pass the documented matrix.
- `cargo fmt`, clippy, tests, GPUI UI tests, REUSE, packaging checks pass.

Status: complete. `packaging/aur` contains the `nfc-foundry` source package and
`.SRCINFO`; the release workflow builds the relocatable binary tarball and
license notices inline. It
builds on Ubuntu 26.04 for Linux amd64 and publishes the assets on a
`v*` tag. The check suite runs on branches and pull requests, not on the
release tag. The AUR package remains outside GitHub
CI. `docs/hardware-matrix.md` records the certified PN532 UART and
ACR122U scope; ACR122U writes for Classic data blocks, Type 2 NDEF, and Classic
NDEF, one-way locks, passwords, trailer writes, and Classic 4K remain
unrecorded and are not advertised; Type 4 read/write is validated on PN532 UART
and not recorded on ACR122U. Version reporting is unified
in `Cargo.toml`, `nfctl --version`, `nfctl diag`, and the GUI window title.
Local package build, package file list, tarball extraction, `namcap`, and
`.SRCINFO` checks passed on 2026-11-10. The GitHub release is
published from the `v*` tag; the AUR upload remains a separate manual step. See
`docs/specs/12-release.md` and `docs/notes/12-release.md`.

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

### 18: Workbench UI redesign and bilingual UI

Goal: replace the current per-workspace shells with the persistent workbench
in `docs/design/03-workbench.html`, provide an English-base UI with a complete
Simplified Chinese translation, and ship an application icon and desktop file.
The CLI remains English-only. The GUI must expose every non-test `nfctl`
operation with the same safety rules. This phase adds no card operation or
protocol.

Deliver:

- Persistent shell: grouped navigation, reader/card header, reader menu,
  domain switch, status bar, session activity, and language switch.
- Screens: Classic blocks, Classic NDEF, diff, keys, recovery; Type 2 memory,
  NDEF, protection; diagnostics.
- English base catalog and complete `zh-CN` catalog; persisted language
  preference; system-locale detection with English fallback.
- Recovery GUI over the existing native recovery services: strategy, target,
  progress, cancel, and verified result. Read-only and exclusive-lease rules
  are unchanged.
- Diagnostics without card I/O and a session-only activity log.
- Application identity: vector icon with final PNG exports, `.desktop` file
  with English and `zh_CN` metadata, package and tarball installation, and
  icon/desktop validation.
- Full GUI parity with every non-test `nfctl` operation: dump/restore
  formats, key import, raw NDEF and URI writes, restore include options, and
  all recovery strategies.
- Updated design contracts, UI tests, i18n tests, and packaging checks.

Acceptance:

- All listed screens match the design sample and retain current safety rules.
- English and Chinese UI copy is complete; switching is immediate and
  persisted; missing keys fall back to English.
- `nfctl` remains English-only under a Chinese system locale.
- The installed desktop file validates, resolves its icon, and launches the
  GUI.
- `cargo fmt`, clippy, workspace tests, GUI tests, `reuse lint`, and packaging
  checks pass.

Detailed plan: [18-workbench-ui.md](18-workbench-ui.md).

Status: complete for the removed GPUI implementation. The shell, bilingual
catalog, recovery screen, diagnostics/activity/settings, GUI parity, and
icon/desktop installation were implemented and tested. The GUI code was
removed on `feat/gtk4-ui`; the GTK4 replacement is phase 19. See
`docs/specs/18-workbench-ui.md` and `docs/notes/18-workbench-ui.md`.

### 19: GTK4 workbench

Goal: one GTK4-rs frontend for the workbench, replacing the removed GPUI
workspaces. The first slice covers the shell, real reader selection, Classic
read/table/inspector/verified block write, dump save/load, and a read-only
Classic NDEF view. Keys, diff, recovery, NDEF mutations, Type 2/Type 4, the
`zh-CN` catalog, GUI integration tests, and packaging remain.

Acceptance: real Classic read; partial reads explicit; block 0 read-only;
trailer confirmation; backup and read-back visible; reader change invalidates
writes; `cargo fmt`, clippy, workspace tests, and `reuse lint` pass.

Detailed plan: [19-gtk-ui.md](19-gtk-ui.md).

Status: complete 2026-11-11. The shell and all parity screens landed through
19a-19e. See `docs/specs/19-gtk-ui.md`, `docs/notes/19-gtk-ui.md`, and the
slice notes.

### 19a: Classic completion and Proxmark3 interchange

Goal: finish the Classic workbench and make dump/key interchange compatible
with Proxmark3 without losing the lossless partial-image path.

Scope: remove legacy `nfcx` naming; default complete dumps to PM3 raw `.bin`
and partial dumps to project `.nfc-foundry.json`; add PM3 `.dic` key
import/export; add the Keys, Diff, and Restore screens; complete the
inspector; add the service-level write identity guard; add mock/Xvfb GTK UI
tests.

Non-goals: NDEF writes, Type 2/Type 4, localization, packaging, Windows/macOS,
and a second frontend.

Detailed plan: [19a-classic-pm3.md](19a-classic-pm3.md). Spec:
[19a-classic-pm3.md](../specs/19a-classic-pm3.md).

Status: complete 2026-11-11. See `docs/specs/19a-classic-pm3.md` and
`docs/notes/19a-classic-pm3.md`.

### 19b: NDEF write and format

Goal: close the NDEF write path for MIFARE Classic before Type 2/Type 4
screens. The existing read-only NDEF tab gains a Text/URI/raw composer, capacity
and read-only gates, verified write with backup and identity check, and
formatting with an explicit destructive confirmation. The same draft
abstraction is reused by Type 2 later.

Non-goals: Type 2/Type 4 screens, full RTD catalog (vCard/Wi-Fi/etc.), i18n,
and packaging.

Detailed plan: [19b-ndef-write.md](19b-ndef-write.md). Spec:
[19b-ndef-write.md](../specs/19b-ndef-write.md).

Status: complete 2026-11-11. Note:
[19b-ndef-write.md](../notes/19b-ndef-write.md).

### 19c: Type 2 / NTAG and Type 4 read

Goal: add Type 2 user-memory and Advanced protection flows, reuse the 19b NDEF
composer, and gate Type 4 read on a hardware record.

Detailed plan: [19c-type2-type4.md](19c-type2-type4.md). Spec:
[19c-type2-type4.md](../specs/19c-type2-type4.md). Status: complete 2026-11-11.
The service read returns card identity, all Type 2 mutations take an expected
`CardInfo`, and `Service::read_card` dispatches Classic/Type 2 from one
selection. The GUI adds Type 2 Memory/Protection, Advanced page and destructive
confirmations, lock/password forms, dump/restore, and Type 2 NDEF write/format.
CC writes keep the product and refresh the parsed CC. Type 4 remains out of the
UI and CLI; the PN532 hardware record landed on 2026-11-12, after this phase, so
a read surface is unblocked but not built. See
[docs/notes/19c-type2-type4.md](../notes/19c-type2-type4.md).

### 19d: Recovery and system surfaces

Goal: expose keyscan, native recovery with progress/cancel, diagnostics,
activity, and persisted settings without adding card writes.

Detailed plan: [19d-recovery-system.md](19d-recovery-system.md). Spec:
[19d-recovery-system.md](../specs/19d-recovery-system.md). Status: complete
2026-11-11. The GUI adds the Key scan and Recovery pages with worker progress,
elapsed time, cancellation, explicit key saving, and engine limitations shown
verbatim; Diagnostics, Activity, and persisted Settings; `nfctl` gains keyscan
dictionary candidates, `--known-type`, and reader/build facts in `diag`. See
[docs/notes/19d-recovery-system.md](../notes/19d-recovery-system.md).

### 19e: Completion

Goal: complete the English/`zh-CN` catalog, close the CLI/GUI parity matrix,
expand the Xvfb UI scenarios, and package both binaries with the icon and
desktop entry.

Detailed plan: [19e-completion.md](19e-completion.md). Spec:
[19e-completion.md](../specs/19e-completion.md). Status: complete 2026-11-11.
The GUI has the complete English/`zh-CN` catalog with an immediate, persisted
switch; `docs/design/parity.md` closes the CLI/GUI audit; the Xvfb scenario
covers every shell page, navigation, focus, dialogs, Chinese reads and
writes; and the AUR package and release tarball install both binaries, the
hicolor icons, and the desktop entry. See
[docs/notes/19e-completion.md](../notes/19e-completion.md).

### 20: All-sector autopwn and host-driven darkside

Status: done. Note: [20-autopwn.md](../notes/20-autopwn.md). This follow-up to
19e replaced the single-target auto strategy with an all-sector scan and a
host-driven darkside sequence, then consolidated Recovery into one scrollable
log.

### 21: NDEF Wi-Fi WSC and copyable card facts

Status: done. Note: [21-ndef-wsc.md](../notes/21-ndef-wsc.md). Media record
`application/vnd.wfa.wsc` is decoded as Wi-Fi authentication/encryption/SSID/
key, non-text payloads render as hex, and card/key/activity values are
selectable or copyable.

### 22: Grouped primary navigation and header menu

Status: done, superseded by phase 23. Note:
[22-navigation.md](../notes/22-navigation.md). The eleven peer tabs became six
grouped pages with system pages in the header menu.

### 23: Card-family workspaces

Status: done. Note: [23-workspaces.md](../notes/23-workspaces.md). The header
workspace selector switches the Classic and Type 2 stacks, moves the shared
NDEF view into the selected workspace, and hides family-inapplicable pages.

### 24: Parity recovery

Goal: port the remaining old-service capabilities that have no Rust
equivalent, in vertical slices: verified-key capture and dump key metadata,
editable workbench, selective restore and write preflight, UID/block-0 writes,
reader polling/reconnect and capability reporting, then details, key-catalog,
and diagnostics gaps.

Detailed plan: [24-parity-recovery.md](24-parity-recovery.md). Status:
complete 2026-11-12, with the S6 ACR122U hardware record pending. S1
(verified-key capture and dump key metadata), S2 (editable
workbench and selective restore), S3a (standalone UID / block-0 write), and
S3b (restore with UID) are complete. S4 (reader lifecycle and capability model)
is complete. S5 (surface and diagnostic gaps) is complete: shared card health,
key catalog built-ins/deletion, MIFARE Mini identity, self-check JSON and GUI
diagnostics, and recovery authorization. S6 (Type 2 authenticated access) is
implemented and validated on PN532 UART with an owned NTAG216; the ACR122U
record is pending. Each slice has a spec in `docs/specs/` and a note in
`docs/notes/`.

### 25: Owned-card support

Goal: make the five owned test cards usable through the existing domains:
FM11RN32N Classic 4K, FM11RF08S recovery, Ultralight EV1, DESFire EV3 8K NDEF,
and the UID-changeable NTAG213 clone.

Detailed plan: [25-card-support.md](25-card-support.md). Status: complete
2026-11-12 on PN532 UART; the ACR122U rows remain unrecorded. 25.1–25.4 are
complete (25.2 MF0UL11 only). 25.5 implements inventory, factory-key
authentication, AN11004 NDEF provisioning, the guarded `DeleteApplication`
undo, `nfctl desfire format-ndef|delete-ndef`, and the GTK `NDEF` tab. The
PN532 UART format/Type 4 write/delete record passed. Slice order: 4K
validation, Ultralight EV1 profiles and `GET_VERSION`, native FM11RF08S
recovery, guarded NTAG213 UID writes, then DESFire EV3 inventory and AN11004
NDEF formatting. The plan names every operation that is irreversible; DESFire
formatting is the only unconditional mutation without a byte-for-byte restore,
but it is undoable by deleting the NDEF application with the PICC master key.
Ultralight EV1 formatting is conditional on a blank OTP CC. Each slice adds
one spec and one note before implementation.

### 26: DESFire workbench

Goal: make DESFire a generic workbench card family: inspect, read, write,
provision applications/files, and manage keys. It is not an application
builder; access-control, wallet, transit, loyalty, and issuance templates stay
out. The baseline (legacy DES/2K3DES authentication, plain Standard/Backup
read, CLI and GTK `DESFire` workspace) is complete.

Detailed plan: [26-desfire-workbench.md](26-desfire-workbench.md). Status: in
progress; 26.1–26.9 are done (baseline, file settings decode, value/record
read, authentication key families, secure-messaging read, Type 4 NDEF read,
cryptographic verification, generic write and transactions, application/file
management). 26.10 value/record/transaction writes, 26.11 key management,
26.12 NDEF provisioning/format, and 26.13 advanced write gates are pending.
The 26.8 writes are recorded on PN532 UART and ACR122U; 26.9 is recorded on
ACR122U because the DESFire card was not on the PN532. Each slice adds a spec
and a note before implementation.
