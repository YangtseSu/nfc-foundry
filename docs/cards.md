# Card support and safety

This document is the support contract. A card family is not advertised until it
has a defined memory model, write preflight, read-back verification, tests, and
real-hardware validation.

## Support matrix

| Family | Identify | Read | Write | Status |
|---|---|---|---|---|
| MIFARE Classic 1K | Yes | Blocks, dump | User blocks; trailers/block 0 guarded | Current baseline |
| MIFARE Classic 4K | Yes | Blocks, dump | User blocks; trailers/block 0 guarded | Current baseline |
| MIFARE Ultralight | Yes | Pages, dump | User pages | Current; original-Ultralight fallback not hardware-tested |
| NTAG213/215/216 | Yes | Pages, dump, NDEF | User pages, NDEF | Current; NTAG215 hardware-tested on ACR122U |
| ISO/IEC 14443-4 | ATS/APDU | APDU | Application-specific | Later |
| DESFire | ISO14443-4 + auth | Files | Guarded | Later |
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

Advanced mode exposes Type 2 copy, repeated copy, erase, lock, and password
set/remove. Copy defaults to user memory; the protected path adds PWD/PACK,
configuration, and writable capability-container bits. Static lock, dynamic
lock, CC read-only, and CFGLCK are one-way. Password set writes PWD, PACK,
AUTH0, and PROT; password removal requires the current password and refuses a
CFGLCK-locked configuration. Every lifecycle action shows old/new values,
writes a full-image backup, verifies each page, and requires a typed card-UID
confirmation in the GUI.

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
card but lack the command path needed for a workflow. The capability model must
distinguish:

- enumerate;
- select ISO14443A/B;
- Classic read/write;
- Type 2 read/write;
- raw bytes;
- raw bits;
- PC/SC APDU;
- Nested/Darkside/Hardnested support (engine + firmware dependent).

Never infer all capabilities from "libnfc listed the device".
