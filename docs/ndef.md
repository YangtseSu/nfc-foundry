# NDEF domain

NDEF is one of the two first-class business domains. It is not part of the
Classic block editor or the Type 2 page editor.

## Layers

```text
nfc-ndef            message/record/TLV/templates; no hardware
nfc-type2           pages, CC, locks, config, NDEF TLV adapter
nfc-classic-ndef    MAD, GPB mapping/access, NDEF sectors, format
nfc-type4           ISO-DEP/APDU CC and NDEF file adapter
nfc-app             read/write/format workflows and safety gates
```

## NDEF core

Required types:

- `NdefMessage` and `NdefRecord`.
- TNF values, MB/ME/CF/SR/IL flags, chunked records.
- Type, ID, and payload length encoding.
- TLV: NULL, Lock Control, Memory Control, Proprietary, Terminator.
- Short and three-byte length forms.
- Templates: Text, URI, Smart Poster, vCard, Wi-Fi, Bluetooth LE, DPP.
- Validation with exact error locations; no silent repair.

Unknown and proprietary TLVs are preserved unless the user explicitly chooses
a full format.

## Implemented core

- `Tnf::{Empty,WellKnown,Media,AbsoluteUri,External,Unknown,Unchanged}` with
  value 0x07 rejected.
- `NdefRecord` validates the TNF field rules, 255-octet TYPE and ID fields,
  and the 32-bit payload limit.
- `NdefMessage::decode` and `decode_prefix` accept short and normal layouts,
  reassemble initial/middle/terminating chunks, and enforce MB/ME/CF
  placement. `encode` emits one record per logical record;
  `encode_chunked(max_chunk)` emits explicit chunks.
- `Tlv::{Null,LockControl,MemoryControl,Ndef,Proprietary,Unknown,Terminator}`
  covers short and three-octet lengths, with 0xFF reserved as the long-form
  marker. `TlvStream` preserves unknown/proprietary TLVs and post-terminator
  padding byte for byte.
- `ndef_capacity(user_bytes)` returns the largest NDEF message that fits with
  the NDEF TLV header and a Terminator.
- Templates: Text (`T`), URI (`U`), Smart Poster (`Sp`), vCard
  (`text/vcard`), Wi-Fi WSC (`application/vnd.wfa.wsc`), Bluetooth LE OOB
  (`application/vnd.bluetooth.le.oob`), and DPP (`U` or
  `application/vnd.wfa.dpp`).
- Decode errors include the record index and byte offset. Reserved TNF values,
  reserved URI identifier codes, bad chunk order, missing MB/ME, and truncated
  fields are errors; no silent repair is performed.

## Type 2 / NTAG

Implemented in `nfc-type2`:

- NTAG213/215/216, MIFARE Ultralight, and generic CC-sized Type 2 layouts.
- Capability Container (CC) parsing and encoding, user memory, static and
  dynamic lock bytes, and OTP/CC page roles.
- NTAG configuration pages: PWD/PACK, AUTH0, ACCESS, CFGLCK, PROT, AUTHLIM,
  MIRROR, and MIRROR_PAGE. PWD and PACK never read back.
- NDEF TLV mapping with unformatted, empty, and message states; capacity,
  Terminator and padding handling; unknown/proprietary TLV preservation; and
  read-only state from the CC.
- Project JSON that preserves partial images and rejects a page 3/CC mismatch.
- Verified page, NDEF, format, restore, lock, and password writes over
  `nfc-transport`. Read and write work through the DRIVER page primitives, not
  a raw caller-managed command stream.
- `nfctl type2|ndef` and the GPUI Kit Type 2 workspace.

Not implemented yet:

- `GET_VERSION`, `READ_CNT`, `FAST_READ`, and `READ_SIG`.
- Password authentication for reads/writes after AUTH0 is set. The current
  service programs and verifies PWD/PACK but does not expose an
  authenticate-only request.
- Card-to-card copy and whole-memory erase.

Safety:

- Default to user-memory reads and writes.
- Lock bytes, configuration, PWD/PACK, AUTH0/ACCESS, CFGLCK, and the CC are
  Advanced and need typed confirmation.
- Full-image project backup before the first change.
- A lock update may set bits but never clear them.
- PWD/PACK writes are verified with PWD_AUTH; secrets are not logged.
- Genuine Type 2 UID pages are read-only.
- A partial image is explicit and is never written as if complete.

## MIFARE Classic NDEF

Implemented in `nfc-classic-ndef`:

- MAD1 in sector 0 and MAD2 in sectors 0 and 16, with CRC-8/MIFARE-MAD.
- NFC AID `03 E1`, contiguous NFC sectors, and the MAD2 sector-16 exception.
- Mapping version and read/write access from each NFC sector GPB. The Classic
  mapping has no capability-container record; the GPB is the capability.
- Classic TLV framing and the mandatory NDEF Message TLV with capacity and
  offset reporting.
- Data-area encoding that preserves leading proprietary TLVs and writes the
  Terminator; `03 00 FE` for a formatted empty area.
- Read-only state from GPB access condition `11b` and access bits
  `07 8F 0F`.
- Blank format with MAD1 (1K) or MAD2 (4K), public NFC Key A, and read/write
  or read-only sector trailers.

Safety:

- Format is confirmed, backed up, complete-read-only, and verified.
- Never write NDEF if the mapping is malformed, non-contiguous, partially
  readable, read-only, or the current access bits do not permit the write.
- Write the NDEF length field as zero, then the value and Terminator, then the
  final length; verify every written block by read-back.
- Never write manufacturer block 0 or a trailer during an NDEF message write.

## Type 4

Implemented in `nfc-type4`:

- ISO 7816-4 APDU helpers for the NDEF Tag Application, SELECT FILE, READ
  BINARY, and UPDATE BINARY, with status-word mapping.
- Mapping version 2.0 CC parsing: CCLEN, MLe, MLc, NDEF File Control TLVs,
  Proprietary File Control TLVs, unknown TLV preservation, and short and
  three-octet TLV lengths.
- NDEF file state (empty or decoded message), NLEN capacity checks, MLe-sized
  reads, and MLc/transport-sized verified writes.
- Read-back verification after NLEN and message writes; clear writes NLEN zero
  and verifies it.
- Project JSON with raw CC and canonical NLEN+message bytes.
- `nfc-app` workflows with pre-mutation backups, read-only refusal,
  confirmation for clear, and card identity checks.

Both drivers carry ISO-DEP APDUs: PN532 UART through `InDataExchange` and
ACR122U through the `D4 40 01` pseudo-APDU. The shared 247-byte command limit
comes from the ACR122U wrapper. The adapter uses an existing Type 4 NDEF
application; it does not create one.

Not implemented yet:

- Mapping version 3.0 and Extended NDEF File Control TLVs.
- Writing non-mandatory NDEF files or proprietary files.
- CC modification, read-only transitions, or lock operations.
- CLI and GUI exposure. The Phase 11 plan delivers the adapter and service
  only; adding a command requires the same-rule GUI parity work.

## CLI

```text
nfctl type2 info|read|write|dump|restore|lock|password
nfctl ndef read|write|format
nfctl classic ndef read|write|format
```

All destructive commands require explicit confirmation; machine-readable
output is English-only.

## GUI

GPUI Kit feature crates:

- `gui/type2`: page table, CC, lock/config inspector, protection actions, and
  the Text/records NDEF view. Its NDEF view replaces the earlier separate
  `gui/ndef` plan until a second family needs shared editor components.
- `gui/classic`: Blocks, NDEF, Diff, and Keys tabs; the NDEF tab shows the
  Classic mapping and records and exposes Text write and confirmed format.

Use `DataTable` for memory, `TextArea`/`Input` for records, `Select` for
templates, `AlertDialog` for destructive actions, and `Diff` for before/after
payload comparison.

## Tests

- TLV length forms and terminator handling.
- Chunked and oversized records.
- Capacity/padding boundaries.
- Unknown TLV preservation.
- Lock/config transitions and read-only enforcement.
- Malformed messages and error locations.
- NDEF round trips on NTAG215/ACR122U and Classic 1K/PN532 UART. The Classic
  1K/PN532 UART round trip passed on 2026-11-10; the ACR122U Classic record
  remains pending.
