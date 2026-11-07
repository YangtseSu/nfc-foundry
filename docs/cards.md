# Card support and safety

This document is the support contract. A card family is not advertised until it
has a defined memory model, write preflight, read-back verification, tests, and
real-hardware validation.

## Support matrix

| Family | Identify | Read | Write | Status |
|---|---|---|---|---|
| MIFARE Classic 1K | Yes | Blocks, dump, MAD NDEF | User blocks, NDEF data; trailers/block 0 guarded | Current baseline; MAD1 NDEF hardware-tested on ACR122U |
| MIFARE Classic 4K | Yes | Blocks, dump, MAD NDEF | User blocks, NDEF data; trailers/block 0 guarded | Current baseline; 4K/MAD2 NDEF pending hardware validation |
| MIFARE Ultralight | Yes | Pages, dump | User pages | Current; original-Ultralight fallback not hardware-tested |
| NTAG213/215/216 | Yes | Pages, dump, NDEF | User pages, NDEF | Current; NTAG215 hardware-tested on ACR122U |
| ISO/IEC 14443-4 | ATS/APDU | APDU, Type 4 NDEF | Type 4 NDEF with free write access | Implemented; hardware pending |
| DESFire | ISO14443-4 + auth | Version, apps, files, standard/backup reads | Not exposed | Read-only implemented; hardware pending |
| FeliCa / NFC Forum Type 3 | IDm/PMm | TBD | TBD | Not committed |
| Jewel / Topaz | Yes | Commands | User bytes | Optional |

## MIFARE Classic

Geometry:

- Classic 1K: 16 sectors, 4 blocks each, 64 blocks, 1024 bytes.
- Classic 4K: 40 sectors; sectors 0-31 have 4 blocks, sectors 32-39 have 16
  blocks; 256 blocks, 4096 bytes.

Rules:

- Authenticate the sector containing the target block before access.
- Validate block 0 BCC (`UID[0] ^ UID[1] ^ UID[2] ^ UID[3]`).
- Validate trailer access-bit complements before any trailer write.
- Write ordinary blocks first, trailers last.
- Block 0 writes are disabled by default; require explicit confirmation and a
  backup.
- Verify every write by reading it back.
- Abort if UID, ATQA, or SAK changes mid-operation.
- Keep `.mfd`/`.bin` compatibility; metadata goes in a sidecar JSON file.

Block 0 writes are limited to 4-byte UID Classic 1K special cards. The workflow
first attempts the ordinary CUID/Gen2 manufacturer write, then the in-process
Gen1A unlock when that leaves the UID unchanged. It backs up block 0 before
either attempt, keeps the reader open, and requires the new UID and complete
block 0 readback after reselection. This path is hardware-validated on an
ACR122U (`acr122_usb`) with a Gen1A test card. FUID/UFUID cards and non-4-byte
UIDs are not supported.

### Classic NDEF

Classic NDEF uses the MIFARE Application Directory (MAD) rather than a
capability container:

- MAD1 lives in sector 0 blocks 1-2; MAD2 adds sector 16. The directory CRC is
  CRC-8 with polynomial `0x1D` and preset `0xC7`.
- AID `03 E1` marks NDEF sectors. They must be contiguous; a MAD2 run may skip
  sector 16. The NDEF TLV area is the concatenation of the NDEF sectors' data
  blocks, excluding every trailer.
- The standard public keys are `A0A1A2A3A4A5` for MAD sectors and
  `D3F7D3F7D3F7` for NDEF sectors. Reads also accept the transport default
  keys when the card still uses them.
- A message write changes only NDEF data blocks and verifies each by read-back.
  It never changes a trailer.
- Formatting is Advanced/confirmed. It accepts an existing MAD with free or
  NDEF sectors, or a blank transport-state card with zero data. It writes data
  blocks, then NDEF trailers, then MAD blocks, and the MAD sector trailer last;
  Key B is preserved and must be readable or verified when a trailer changes.
- MAD3, DESFire directories, Type 4 NDEF, and FeliCa NDEF are separate
  card-family work.

Classic NDEF read, write, and format are covered by hardware-free tests and an
explicit hardware test. MAD1 on a blank Classic 1K was validated on an ACR122U
(`acr122_usb`) on 2026-11-07: format sectors 1-2, write a Text record, read it
back, and reformat. Classic 4K/MAD2 and secret-Key-B paths remain pending.

## ISO/IEC 14443-4, DESFire, and Type 4 NDEF

ISO-DEP activation uses RATS over the raw transport. The ATS supplies FSC and
FWI; command frames are split to FSC and I-block chaining, R(ACK), and WTX are
handled. PPS, CID/NAD, Type B, and extended APDUs are not implemented.

Read-only DESFire support covers `GetVersion`, `GetApplicationIDs`,
`SelectApplication`, `GetFileIDs`, `GetFileSettings`, and standard/backup
`ReadData` over wrapped `90 INS 00 00 ...` commands. DESFire native fields are
little-endian. Authentication, key changes, formatting, protected-file reads,
and all writes are out of scope.

Type 4 NDEF reads the `D2760000850101` application, validates the `E103`
capability container, and reads the NDEF file. Writes require free write access
(`00`), zero NLEN before writing, write the message in MLc-sized chunks, write
the new NLEN, and read the complete file back. The previous raw message is
backed up under `$XDG_CONFIG_HOME/NFC Foundry/type4-backups/`.

`nfctl apdu --raw` is diagnostic only. `nfctl desfire` is read-only.
`nfctl ndef read/write` dispatches Type 4 when the card SAK has bit 5 set.
Hardware validation on a Type 4 or DESFire card is pending.

## Type 2: Ultralight and NTAG21x

Type 2 tags use 4-byte pages and plain `READ (0x30)`, `FAST_READ (0x3A)`,
`WRITE (0xA2)`, `GET_VERSION (0x60)`, and optional `PWD_AUTH (0x1B)`. There is
no Crypto1. The application sends these commands as CRC-A framed raw frames
through `RawTransceiver`; this is the hardware-validated path on the ACR122U.

NTAG21x memory map:

| Tag | Pages | User pages | User bytes | Dynamic lock | Config | PWD/PACK |
|---|---:|---|---:|---|---|---|
| NTAG213 | 45 | 4-39 | 144 | 40 | 41-42 | 43-44 |
| NTAG215 | 135 | 4-129 | 504 | 130 | 131-132 | 133-134 |
| NTAG216 | 231 | 4-225 | 888 | 226 | 227-228 | 229-230 |

Page 0-1 contain UID/BCC; page 2 contains internal/static-lock bytes; page 3 is
the capability container. Do not treat pages as Classic blocks and do not reuse
the Classic sector/trailer model.

Write rules:

- Default to user-memory pages only.
- Treat config pages, dynamic lock bytes, static lock bits, PWD/PACK, and
  authentication-protected pages as dangerous.
- Require an explicit confirmation and a backup before any config/lock write.
- Verify every page read-back.
- Keep a full 540-byte-image path for NTAG215-style analysis, but expose user
  memory as the normal dump unit.
- Never write UID pages through a normal Type 2 command; UID is read-only on
  genuine NTAG. Ordinary `type2 write` and `type2 restore` do not write pages
  0-3. The Advanced lifecycle commands may write static lock bits (page 2,
  preserving BCC1/internal bytes) and the capability-container access byte
  (page 3), under the one-way and confirmation rules below.

NDEF read/write is implemented:

- parse and validate the capability container and NDEF TLV;
- read and decode Text, URI, Smart Poster, MIME `text/plain`, vCard, JSON,
  Wi-Fi WSC, Bluetooth BR/EDR easy pairing, Bluetooth LE OOB, and Android
  Application Records;
- write Text, URL/URI, custom URI, search, payment, social, application,
  email, contact, phone, SMS, location, address, Bluetooth BR/EDR, Bluetooth
  LE, Wi-Fi, Smart Poster, and WPA3/DPP bootstrapping templates;
- expose search, social, and payment provider presets with an editable URL,
  a text language selector, and Smart Poster action/type/size metadata;
- import and export raw NDEF message files, edit raw messages, and build
  multi-record messages;
- mask Wi-Fi keys and other sensitive fields until the user reveals them;
- hide raw payload/message hex by default and show it only on request;
- refuse to overwrite lock bytes, configuration pages, or a preceding
  lock/memory-control TLV;
- report unsupported or malformed NDEF without modifying the tag.

The GUI writes one user page at a time and template, raw, or multi-record NDEF
messages after an explicit confirmation. A password field supplies PWD_AUTH to
reads and writes. WPA3-only onboarding is represented by a DPP bootstrapping
URI, because the WSC 2.0 credential format has no WPA3/SAE authentication type.
NFC Foundry writes and reads the URI only; full DPP authentication and
provisioning are out of scope.

Advanced mode exposes Type 2 copy, repeated copy, erase, and a per-field
protection editor for AUTH0, ACCESS, PWD/PACK, dynamic lock, and static lock.
Copy defaults to user memory; the protected path adds PWD/PACK, configuration,
and writable capability-container bits. Static lock, dynamic lock, CC
read-only, and CFGLCK are one-way. The protection editor writes ordinary user
pages and PWD/PACK first, then configuration, dynamic lock, and static lock
last; CFGLCK cannot be cleared and PWD/PACK/configuration writes are refused
once it is set. Every action writes a full-image backup and verifies each page.
Copy and erase require a typed card-UID confirmation. Protection writes require
the exact word `PROTECTED` and a preview token that expires after five minutes.
The CLI additionally exposes explicit `type2 lock` and `type2 password`
commands.

Type 2 dumps include PWD and PACK pages so a full restore is possible; write
them with mode 0600 and treat them as sensitive. The CLI masks those pages unless
`--show-keys` is set. JSON is the default dump format; `.bin` uses the
self-describing `NFCT2BIN` full-image format.

The NTAG215 path (full dump, NDEF write, restore, and read-back) is validated on
an ACR122U. NTAG213/216 use the same commands and geometry model but have not
been hardware-tested.

## ISO/IEC 14443-4 and DESFire

Implement APDU transport first (RATS/ATS, I-block chaining, status words). Then
add DESFire authentication and a file model. Key changes, format, and
application deletion are destructive and require explicit confirmation,
key backups, and a tested recovery path.

Do not expose a generic "send any APDU to any card" command as a write path;
raw APDU belongs in a clearly marked diagnostic command.

## FeliCa / NFC Forum Type 3

FeliCa support is not committed. It needs a defined scope before
implementation:

- Type 3 NDEF only, or raw FeliCa services?
- IDm/PMm/system-code/service-code handling;
- read-only vs write support;
- which readers and regions are validated.

Until that scope is written and accepted, do not add FeliCa to the advertised
support matrix.

## Jewel / Topaz

Optional. If added, implement as a small command-based module with read,
user-byte write, and read-back verification. Do not make it part of the initial
milestone.

## Card emulation

Card emulation is a future direction, primarily for mobile Linux devices. It is
not part of the current milestones and is not exposed by the GUI or CLI.

If it is pursued:

- define the emulated target type and the reader it must satisfy;
- validate against real readers and systems;
- keep target-mode code behind a separate capability;
- do not reuse initiator write paths for emulation state.

## Reader capabilities

Reader capability is separate from card capability. A reader may enumerate a
card but lack the command path needed for a workflow. The application profile
compiles five drivers:

```text
pn532_uart,pn53x_usb,acr122_usb,acr122_pcsc,pcsc
```

| Driver | Select | Classic | Type 2 | Raw bits | ISO-DEP | Recovery | Validation |
|---|---|---|---|---|---|---|---|
| `pn532_uart` | yes | read/write | read/write | yes | yes | Nested, Darkside, Hardnested | partial hardware |
| `pn53x_usb` | yes | read/write | read/write | yes | yes | no | none |
| `acr122_usb` | yes | read/write | read/write | yes | yes | no | partial hardware |
| `acr122_pcsc` | yes | read/write | read/write | yes | yes | no | none |
| `pcsc` | yes | no | no | no | no | no | none |

Rules:

- A known driver that cannot perform an operation returns `CodeUnsupported`
  before a hardware command is sent. The GUI disables the corresponding read
  action, and the CLI exits with the hardware/unavailable code.
- A compiled driver is not advertised as validated support. Only
  `pn532_uart` and `acr122_usb` have recorded application hardware runs; see
  the Phase 06 note.
- Recovery engines remain PN532 UART-only until another reader is validated
  with them.
- The generic `pcsc` driver selects ISO/IEC 14443A targets. PC/SC is a
  smart-card/APDU API (CCID readers, PIV/bank cards, USB tokens such as
  YubiKey), not a raw NFC transport, so Classic, Type 2, Gen1A, and recovery
  workflows are not implemented over it and are not planned. A future
  PC/SC-native APDU transport would cover ISO-DEP/Type 4/DESFire only.
- `acr122_pcsc` keeps the PN53x raw path through PC/SC escapes but is
  unvalidated. On the tested ACR122U it did not open reliably; on Linux the
  direct `acr122_usb` path is the supported one.
- An ACR122U appears as direct USB, ACR122 PC/SC, and generic PC/SC.
  Enumeration prefers direct USB and suppresses the matching PC/SC aliases.
  The cross-process lock treats all ACR122 connstrings as one physical reader;
  this conservatively serializes multiple ACR122 readers across processes
  because a PC/SC reader name cannot be mapped back to a USB bus address.
- An unclassified connstring is reported as an unverified profile. Its
  operations are still attempted for development backends; it is never
  advertised as a validated reader.
