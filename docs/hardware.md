# Hardware scope and transport requirements

The release certificate is [Hardware matrix](hardware-matrix.md). This document
describes the transport behavior; the matrix records what each reader has
passed.

## Supported targets

| Target | Connstring | Backend | First-release scope |
|---|---|---|---|
| PN532 UART | `pn532_uart:/dev/ttyUSB0` | `serialport` + PN532 framing | Classic + Type 2 + raw bits/parity; nested/static-nested/hardnested recovery |
| ACR122U | `acr122_usb:...` | `nusb` + CCID + ACR122 pseudo-APDU | Classic + Type 2 + raw bits/parity + Gen1A; nested/static-nested/hardnested recovery |

No other reader is advertised in the first release.

Both readers have an ISO-DEP/APDU code path for NFC Forum Type 4, but it is
not part of the advertised scope until a Type 4 read/write record is added to
`docs/notes/11-type4-ndef.md`. The transport-wide limit is 247 bytes per
command APDU because the ACR122U pseudo-APDU wrapper leaves that much room.

Post-release work in plan phases 13-17 uses the same two readers for ISO
14443B, high-rate ISO-DEP, Type 3/4B NDEF, EMV public data, China T-Union,
FMCOS/Calypso, ICAO 9303 eMRTD, and Amiibo/NTAG215. Phase 13 is the pending
RF/transport prerequisite; phases 14-17 are optional. It is not advertised and
has no hardware record. Amiibo uses the existing Type 2 path; its write path
is Advanced and limited to owned magic NTAG215 tags.

## PN532 UART

The connstring is `pn532_uart:<serial-port>`. `nfctl list` reports every serial
port as a candidate; opening the exact connstring validates the PN532 firmware
before any card operation. Every host command carries the HSU wake-up preamble
so the chip works after power-down and while already awake.

Implement:

- serial open/close at 115200 8N1, read timeouts, and bounded retries;
- PN532 normal/extended framing, ACK/NACK/error frames, and checksums;
- `InListPassiveTarget` / ISO/IEC 14443A card selection;
- `InDataExchange` for MIFARE Classic authentication and 16-byte block
  read/write;
- `InCommunicateThru` for raw ISO/IEC 14443A bytes and partial-bit frames;
- raw mode that disables hardware CRC, rejects ordinary MIFARE operations, and
  leaves parity to the PN532;
- recovery parity framing for caller-supplied parity, 7-bit short frames such
  as WUPA, and fast SELECT between hardnested nonce acquisitions;
- explicit rejection of non-PN532 firmware, missing cards, timeouts, and serial
  disconnects.

Hardware tests:

- list/select a Classic 1K and an NTAG215;
- read one Classic block with a known key;
- read Type 2 pages;
- same-value Classic write with explicit confirmation and read-back;
- guarded Gen1A UID change and restore;
- raw transceive after driver stabilization;
- full Classic dump and per-sector key scan;
- temporary data-block write and restore with backups.

Current status: firmware identification, Classic 1K selection/authentication/
read, Type 2 selection/read, raw 7-bit REQA (16-bit ATQA response), full Classic
dump, key scan, temporary data-block write with backup and read-back, and
restore are validated on a PN532 (CH340 UART adapter, firmware 1.6). The
guarded Gen1A UID change and restore, sector-trailer write, and full restore
with UID passed on 2026-11-11 with an owned magic Classic 1K; see
`docs/notes/24c-uid-write.md` and `docs/notes/24d-restore-with-uid.md`. A 4K
card run remains a future hardware check.

## ACR122U

Target direct USB/CCID only. Do not use PC/SC in the first release.

The connstring is `acr122_usb:<bus>:<address>` in decimal, for example
`acr122_usb:001:017`. Enumeration reports the exact address for each attached
`072F:2200` reader; opening claims the CCID interface and validates the
internal PN532 firmware.

Implement:

- USB enumeration by VID/PID and exact bus/address connstring;
- CCID interface claim, bulk endpoints, bounded timeouts, abort, and stale
  bulk-IN draining;
- ICC power-on, PICC operating parameters, initiator mode, RF field control,
  and passive retry configuration;
- ACR122 pseudo-APDU direct transmit (`FF 00 00 00 Lc D4 ...`) for ISO/IEC
  14443A bytes and bits, with `61xx` GetResponse continuation;
- udev permission guidance;
- no LED/buzzer controls beyond protocol state.

`pcscd` claims the same CCID interface through libusb. Stop it before opening
the direct USB path:

```sh
sudo systemctl stop pcscd
```

The driver reports a busy interface with the same instruction. The PC/SC alias
is not supported; restart `pcscd` after the direct USB session when other
smart-card applications need it.

Rationale: PC/SC pseudo-APDUs do not provide the same raw bit/parity control
required by Classic recovery and Gen1A.

Hardware tests:

- select a Classic 1K and a Type 2 tag;
- read one Type 2 page through raw `READ`;
- read one Classic block with a known key;
- raw 7-bit REQA with a 16-bit ATQA response;
- Gen1A block 0 write only with explicit confirmation and read-back;
- repeated open/close cycles.

Current status: validated on 2026-11-10 with an ACR122U at
`acr122_usb:001:017` and internal PN532 firmware 1.6. The run covered firmware
identification, MIFARE Classic 1K block read with a known key, Type 2 page
read, raw 7-bit REQA, and a same-value Gen1A manufacturer-block write verified
by reselection and read-back. A physical unplug/replug was not exercised;
disconnect mapping is covered by scripted transport tests. See
`docs/notes/05-acr122-usb.md`. Phase 24c adds a guarded application service over
the same Gen1A primitive and an ordinary CUID/Gen2 path. The service path is
validated on both PN532 UART and ACR122U; phase 24d adds restore with UID,
also validated on both. Classic data-block and sector-trailer writes, full
dumps, and Classic NDEF format/write/read/restore are validated on ACR122U.

### Type 2 write and NDEF status

The Type 2 READ path is validated on both readers. On 2026-11-10 a PN532 UART
at `pn532_uart:/dev/ttyUSB0` completed an authorized NTAG215
(`04 52 F5 67 D3 2A 81`) NDEF round trip: read 135/135 pages, format, write a
33-byte Text record, verify it, then restore the original user memory from a
project backup. The original URI NDEF was present after the restore. The write
path uses READ (0x30), WRITE (0xA2), and PWD_AUTH (0x1B), CRC-A, read-back
verification, and restored raw mode. ACR122U Type 2 NDEF write/read/restore is now recorded; one-way locks and
password configuration have not been recorded yet. Run the gated checks in
`docs/testing.md` before advertising those remaining operations as
hardware-validated. The phase 09 note records the details.

Raw operations after a selected card require an RF field cycle to return the
card to IDLE; `InRelease` alone did not produce a REQA response on this reader.
`Acr122Usb::cycle_rf_field` performs that transition.

Recovery parity framing is validated on both readers: the PN532 and ACR122U
return a 32-bit tag nonce from a parity-disabled partial authentication and
complete full software Crypto1 authentication. Nested and static-nested
recovery, hardnested fast-select acquisition with 7-bit WUPA, and hardnested
key recovery are validated on both transports. A known response size avoids the
post-response trailing-bits reset and status read so the reader response is
transmitted inside the card's authentication window. Darkside requires
PRNG-scheduled authentication frames that neither host-driven reader can
produce; the solver is validated by the PM3 golden vector and the acquisition
reports no repeated-nonce sample.

## Device ownership

- One process and one operation may own a reader.
- The exact connstring is selected explicitly; never depend on `/etc/nfc`.
- Cross-process lock is keyed by physical reader identity, not just the
  connstring string.
- No external process may share the reader during an operation.

## Unsupported by design

- PN53x USB drivers
- PC/SC-only readers and the ACR122U PC/SC alias
- Proxmark3 USB device control
- Windows/macOS transport
- SPI/I2C and embedded Linux buses

Adding any of these requires an ADR, a driver crate boundary, and a hardware
validation record.
