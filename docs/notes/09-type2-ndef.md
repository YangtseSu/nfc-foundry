# Phase 09 note: Type 2 / NDEF

Date: 2026-11-10

## Implemented

- `nfc-core` / `nfc-transport`:
  - `PAGE_SIZE` 4 and `PAGE_QUAD_SIZE` 16.
  - `Reader::read_page_quad` (READ 0x30), `write_page` (WRITE 0xA2), and
    `password_auth` (PWD_AUTH 0x1B). The defaults report `Unsupported`.
  - `MockReader` stores Type 2 pages and a password/PACK pair, scripts
    `ReadPageQuad`, `WritePage`, and `PasswordAuth` faults, and returns loaded
    pages for each READ.
- `nfc-drivers`:
  - Shared Type 2 command constants, CRC-A, command framing, response CRC
    verification, and exact-length and CRC-bearing response acceptance.
  - PN532 UART and ACR122U page read/write/password methods. Each method
    checks the selected card is Type 2, enters raw framing, sends the command,
    restores the previous raw mode, and reports a non-ACK write response.
  - Driver unit tests assert the READ frame CRC (`30 04 26 EE`) and that raw
    mode is restored.
- `nfc-type2`:
  - CC model: `E1` magic, version nibbles, data size from byte 2, access
    nibbles, and NDEF capacity. The level 1/2/3 version encoding is decoded
    from CC[1].
  - Products: Ultralight (48-byte area), NTAG213 (144), NTAG215 (496), NTAG216
    (872), and generic CC-sized tags. NTAG215's 504 bytes of user memory are
    modelled separately from its 496-byte NDEF data area.
  - Page roles, static lock bit positions (page 3 at lock0 bit 3, pages 4..7 at
    bits 4..7, pages 8..15 at lock1 bits 0..7), dynamic lock positions for the
    three NTAG products, and configuration page addresses.
  - `Type2Image` with unknown pages, `ndef_area`, `set_ndef_area`, and project
    JSON.
  - Lock model, pending one-way updates, irreversible-bit validation, and
    RFUI handling for static and dynamic lock writes.
  - Configuration parsing and page builders; PWD/PACK never read back.
  - NDEF state (`Unformatted`, `Empty`, `Message`), message TLV preservation
    in place, unknown/proprietary TLV preservation, format bytes, and exact
    capacity handling without a Terminator when the area is full.
  - Read, verified page write, NDEF write, format, restore, lock, and password
    functions. Restore rejects a different layout or CC, writes user memory
    before configuration, CC, and lock bytes, and never restores PWD/PACK.
- `nfc-app`:
  - `type2` workflow module and service methods. Mutations read the tag,
    export a JSON backup through `BackupStore::save_bytes`, write, verify, and
    recheck card identity.
  - Advanced policies and explicit confirmations for lock, configuration,
    password, capability container, format, and restore.
- `nfctl`:
  - `type2 info|read|write|dump|restore|lock|password` and
    `ndef read|write|format`.
  - Advanced page writes require a destructive confirmation dialog.
  - Mock NTAG215 with a complete 135-page image and an empty NDEF TLV.
- GPUI Kit:
  - `nfc-type2-gui` with Memory, NDEF, and Protection tabs, page inspector,
    verified writes, dump/restore, Text NDEF editor, lock and password forms,
    and confirmation dialogs for one-way/Advanced actions.
  - The desktop shell switches between the Classic and Type 2 workspaces.

## Tested

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`
- `nfc-type2`: 38 unit tests covering CC, layouts, locks, configuration, NDEF,
  page images, project dumps, read coverage, verified writes, ignored writes,
  locks, password verification, and restore validation.
- `nfc-app`: 11 scripted Type 2 workflow tests covering complete/partial reads,
  page writes and backups, Advanced gating, ignored writes, NDEF
  write/read/format, unknown TLV preservation, lock denial, password
  configuration, dump/restore, and CC mismatch rejection.
- `nfctl`: 4 new CLI mock tests covering info, dump, page write, Advanced
  confirmation, restore, lock, and NDEF write/read/format.
- `nfc-type2-gui`: 5 UI integration tests covering read and page selection,
  NDEF write/format, Advanced write confirmation, lock confirmation, and
  protection controls.
- Driver tests cover the Type 2 READ CRC and raw-mode restoration on both
  transports.

Hardware on 2026-11-10, PN532 UART at `pn532_uart:/dev/ttyUSB0` (CH340,
firmware 1.6), NTAG215 UID `04 52 F5 67 D3 2A 81`:

- read-only check: 135/135 pages, CC `E1 10 3E 00`, 504-byte user memory,
  496-byte NDEF data area, 491-byte NDEF capacity, no warnings;
- original NDEF: one URI record, `https://www.m5stack.com`;
- locks `00 00` / `00 00 00`; AUTH0 `FF`, ACCESS `00`, CFGLCK off, AUTHLIM 0;
- authorized write round trip with `NFC_FOUNDRY_HARDWARE_WRITE=yes`:
  formatted NDEF, wrote a 33-byte Text record, read it back as the expected
  message, then restored the original user memory from a project backup;
- post-restore read: the original URI record and page image are back;
- backups written to
  `/tmp/nfc-phase09-backups/0452F567D32A81-NTAG215-ndef-format-*.json`,
  `...-ndef-write-*.json`, `...-restore-*.json`, and `original-type2.json`.

The ACR122U was not attached (`lsusb` had no `072f:2200`), so the Phase 09
acceptance hardware record still needs an ACR122U run. Lock and password
hardware paths were not exercised; they need a separate owned-tag
authorization. The gated tests live in `nfctl/tests/hardware_type2.rs`.

## Problems and resolutions

- The first CC parser treated `E1` as the major version and `10` as the minor
  version. The Type 2 CC stores the magic in byte 0 and the version nibbles in
  byte 1; fixed. This also corrected `to_bytes`.
- Dynamic lock byte 3 reads `BDh` on NTAG21x but RFUI must be written as zero.
  Read-back now compares only the first three bytes and keeps the observed RFUI
  byte in the image.
- PWD and PACK read back as zero. The write path stores zeros in the image and
  verifies the secret with a fresh PWD_AUTH instead of comparing memory.
- Restoring configuration cannot restore PWD/PACK because dumps cannot read
  them. Restore plan excludes those pages and only restores CFG0/CFG1.
- The initial NDEF writer rebuilt the TLV stream and moved proprietary TLVs
  after the NDEF TLV. The writer now replaces an existing NDEF TLV in place and
  inserts one after leading Lock/Memory Control TLVs otherwise.
- A GUI test fixture left every page number at zero while filling data; the
  mock only exposed pages 0..4. The fixture now enumerates pages explicitly.
- The NDEF tab test clicked format and confirmed with the dialog default OK
  button; the dialog input is only used for one-way confirmations.

## Deviations from the spec

- `type2 copy` and `type2 erase` placeholders were removed. Neither operation
  was specified with safe semantics; restore and format cover the useful
  cases.
- `GET_VERSION`, `FAST_READ`, `READ_CNT`, and `READ_SIG` are not implemented.
  The CC is sufficient for layout selection and the phase plan did not require
  these commands.
- Phase 09 exposed programming PWD/PACK/ACCESS but no PWD_AUTH prompt. Phase 24
  S6 added per-operation protected access through
  [24e-type2-auth.md](24e-type2-auth.md); the CLI and service now authenticate
  protected reads and writes.
- NDEF write uses the Text record in the GUI; the CLI additionally supports
  raw NDEF files and URI records. Template editors remain phase 10 work.
- Dynamic lock block-locking bits are preserved and displayed but not exposed
  as a separate Advanced action.

## Remaining blockers

- Hardware validation of the one-way lock path on an owned tag.
- Password configuration and authenticated reads/writes after AUTH0 are
  implemented in [24e-type2-auth.md](24e-type2-auth.md) (plan 24 S6) and passed
  on PN532 UART with an NTAG216 on 2026-11-12; the ACR122U record is pending.
- `GET_VERSION`/`FAST_READ` for tags whose CC is absent or non-standard.
- Classic MAD/CC NDEF belongs to phase 10.

## 2026-11-11 ACR122U hardware record

`pcscd` was stopped and an NTAG215 (`04 34 4C C2 83 5C 80`) was placed on
`acr122_usb:001:002`. The read test reported 135/135 pages, CC `E1 10 3E 00`,
496-byte NDEF area, 491-byte capacity, and the original 75-byte vCard NDEF
message. The write test then:

- formatted the NDEF area;
- wrote a 33-byte Text record and read it back;
- saved the original image to a project backup;
- restored the original user memory.

A follow-up `type2 info` reported the original 75-byte vCard NDEF, 135/135
pages, static lock `00 00`, dynamic lock `00 00 00`, AUTH0 `FF`, ACCESS `00`,
CFGLCK false, and AUTHLIM 0. The ACR122U Type 2 NDEF row is now validated.

The password test was not run: it changes PWD/PACK and AUTH0 without restoring
them, so it needs an explicit decision for the owned tag.
