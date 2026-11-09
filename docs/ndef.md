# NDEF domain

NDEF is one of the two first-class business domains. It is not part of the
Classic block editor or the Type 2 page editor.

## Layers

```text
nfc-ndef            message/record/TLV/templates; no hardware
nfc-type2           pages, CC, locks, config, NDEF TLV adapter
nfc-classic-ndef    MAD, Classic CC, NDEF sectors, format
nfc-type4           ISO-DEP/APDU NDEF adapter (later)
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

## Type 2 / NTAG

Cover:

- NTAG213/215/216 and MIFARE Ultralight.
- Capability Container (CC), user memory, OTP, static and dynamic lock bytes.
- Configuration pages: PWD/PACK, AUTH0, ACCESS, CFGLCK.
- GET_VERSION, READ_CNT, fast read where supported.
- NDEF TLV mapping, capacity, padding, read-only/lock state.

Safety:

- Default to user-memory reads and writes.
- Lock bytes, configuration, PWD/PACK, AUTH0/ACCESS, and CFGLCK are Advanced.
- Full-image backup before the first change.
- Show exact pages, old/new values, backup path, and read-back plan.
- Typed confirmation for one-way or potentially bricking actions.
- Genuine Type 2 UID pages are read-only.

## MIFARE Classic NDEF

Cover:

- MAD1 and MAD2 detection.
- Capability Container in the MAD sector.
- NDEF sector mapping and capacity.
- Access-bit checks before write/format.
- Blank-card format and read-only state.

Safety:

- Format is Advanced and backed up.
- Never write NDEF if the mapping is malformed or the sector access conditions
  do not permit it.
- Verify every written block/trailer by read-back.

## Type 4

Later phase:

- ISO-DEP/APDU transport.
- Type 4 CC and NDEF file read/write.
- Reuse `nfc-ndef`; no Classic page model.

## CLI

```text
nfctl type2 info|read|write|dump|restore|copy|erase|lock|password
nfctl ndef read|write|format
nfctl classic ndef read|write|format
```

All destructive commands require explicit confirmation; machine-readable
output is English-only.

## GUI

GPUI Kit feature crates:

- `gui/type2`: page table, CC, lock/config inspector, protection actions.
- `gui/ndef`: template/raw/records editor, exact byte preview, read/write.
- `gui/classic`: Classic NDEF view shares `gui/ndef` editor components.

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
- NDEF round trips on NTAG215/ACR122U and Classic 1K/ACR122U.
