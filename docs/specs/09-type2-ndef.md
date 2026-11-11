# Phase 09 spec: Type 2 / NDEF

Date: 2026-11-10

## Goal

Implement the NFC Forum Type 2 / NTAG domain end to end: a hardware-free page,
capability-container, lock, configuration, and NDEF TLV model; reader-backed
read, page write, NDEF write/format, restore, lock, and password workflows with
backups and read-back verification; `nfctl type2` and `nfctl ndef`; and a GPUI
Kit workspace for memory, NDEF, and protection. The GUI and CLI call the same
`nfc-app` services with the same safety rules.

## Deliverables

- Transport primitives:
  - `PAGE_SIZE` (4) and `PAGE_QUAD_SIZE` (16) in `nfc-core`.
  - `Reader::read_page_quad` for Type 2 READ (0x30), `Reader::write_page` for
    WRITE (0xA2), and `Reader::password_auth` for PWD_AUTH (0x1B).
  - PN532 UART and ACR122U implementations that enter raw framing, append and
    verify ISO/IEC 14443A CRC-A, issue the command, and restore the previous
    raw mode.
  - Scripted Type 2 pages, password state, page-read/write faults, and
    PWD_AUTH in `nfc-transport::mock`.
- `nfc-type2`:
  - Capability Container parsing and encoding: `E1` magic, version nibbles,
    data-area size (`CC[2] * 8`), read/write access nibbles, and NDEF capacity.
  - Layouts for MIFARE Ultralight, NTAG213, NTAG215, NTAG216, and a generic
    CC-sized Type 2 tag, with page roles, user-memory ranges, static lock
    positions, dynamic lock positions, and configuration page addresses.
  - `Type2Image` with explicit unknown pages, page iteration, NDEF data-area
    extraction and replacement, and per-page roles.
  - Static and dynamic lock bytes, irreversible pending-lock calculation,
    block-locking/RFUI preservation, and locked-page checks.
  - NTAG configuration parsing: MIRROR, MIRROR_PAGE, AUTH0, ACCESS, CFGLCK,
    PROT, AUTHLIM, PWD, and PACK. PWD/PACK read back as zero.
  - NDEF Message TLV mapping: unformatted, empty, and message states; unknown,
    proprietary, Lock Control, and Memory Control TLV preservation; capacity
    checks; Terminator and zero padding; explicit format bytes.
  - Project JSON dump/load that preserves partial images and rejects a page 3 /
    CC mismatch, wrong versions, and wrong page counts.
  - Reader-backed read with mandatory manufacturer/CC pages and explicit
    warnings for later unreadable quads.
  - Verified page writes, user-memory-only NDEF writes, format, restore,
    one-way locks, and PWD/PACK/AUTH0/ACCESS programming with fresh PWD_AUTH.
- `nfc-app`:
  - Type 2 read, NDEF read, page write, NDEF write, NDEF format, dump, restore,
    lock, and password services.
  - User-memory writes and NDEF writes need no extra confirmation; lock,
    configuration, capability-container, password, format, and restore actions
    require explicit confirmation at the service boundary.
  - Every mutation reads the tag, writes a project backup atomically, applies
    the change, verifies read-back, and rechecks card identity.
  - Unknown pages are never treated as zero; NDEF writes fail when the data
    area is incomplete or the CC forbids writes.
- `nfctl`:
  - `type2 info|read|write|dump|restore|lock|password`.
  - `ndef read|write|format` with raw-message, Text, and URI sources.
  - `--yes`, or the interactive prompt, for one-way and destructive actions.
- GPUI Kit:
  - `gui/type2` workspace with Memory, NDEF, and Protection tabs.
  - Page table and inspector, verified user-page writes, Advanced page confirmation
    confirmation, dump save/load/restore, Text NDEF write/format, lock form,
    and password form.
  - `gui/nfc-foundry` shell switches between the Classic and Type 2
    workspaces.
  - GPUI UI integration tests for read/page selection, NDEF write and format,
    Advanced write confirmation, lock confirmation, and protection controls.

## Non-goals

- No Classic MAD/CC/NDEF-sector adapter. Phase 10.
- No Type 4 ISO-DEP/APDU adapter. Optional phase 11.
- No `GET_VERSION`, `FAST_READ`, `READ_CNT`, or `READ_SIG`; layout comes from
  the capability container. These are candidate follow-ups.
- No card-to-card copy command and no whole-memory erase command. The earlier
  CLI placeholders for `copy` and `erase` were removed rather than faked.
- No bulk restore of lock, configuration, or capability-container pages by
  default. Restore defaults to user memory; the other classes are explicit
  flags.
- PWD and PACK cannot be restored from a dump because they are not readable.
- No Type 2 password authentication input in the GUI beyond programming the
  configuration; protected reads after AUTH0 is set are a follow-up.
- No automatic key/secret retrieval from the network.

## Safety

- Default reads and writes stay in user memory. Page 3, lock pages, and NTAG
  configuration pages require an Advanced policy and confirmation.
- Manufacturer pages 0 and 1 and the UID bytes of page 2 are never written by
  a normal or restore path.
- Lock bits are one-way: a write may set bits and never clear them; requested
  lock values that would clear a bit are rejected.
- Dynamic lock byte 3 is RFUI: only the first three bytes are compared, and a
  write rejects a non-zero byte 3.
- PWD/PACK writes are verified with PWD_AUTH; the secret is never read back or
  logged. A failed PWD_AUTH reports a verification failure.
- PWD/PACK are written before AUTH0/ACCESS, so a protected tag is never left
  with an unknown password.
- Capability-container writes use the tag's bitwise-OR behavior; a request
  cannot clear an existing bit.
- Every mutation saves a project backup before the first page write.
- A partial read is reported with warnings and is never turned into a complete
  image. NDEF write and format refuse an incomplete data area.
- A card identity change aborts the operation after each workflow.

## Acceptance

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`
- Deterministic tests cover CC parsing, product layouts, static/dynamic lock
  positions, configuration parsing, NDEF state transitions, unknown TLV
  preservation, exact-capacity messages, project dump round trips, quad
  coverage for non-multiple-of-four page counts, CRC framing for both drivers,
  and reader-backend Type 2 commands.
- Scripted application tests cover complete and partial reads, page write
  read-back, ignored writes, Advanced gating, NDEF write/read/format, unknown
  TLV preservation, one-way locks, password verification, dump/restore, and
  layout/CC mismatch rejection.
- CLI mock tests cover `type2 info`, dump, page write, Advanced confirmation,
  restore, lock, and `ndef write|read|format`.
- GPUI UI tests cover page selection, NDEF write/format, Advanced confirmation,
  lock confirmation, and protection controls.
- Hardware: NTAG215 read/write/NDEF round trip on PN532 UART and ACR122U,
  recorded in `docs/notes/09-type2-ndef.md`. The PN532 UART read and authorized
  NDEF write/read/restore round trip passed on 2026-11-10; the ACR122U record,
  one-way locks, and password configuration remain pending.
