# Hardware scope and transport requirements

The release certificate is [Hardware matrix](hardware-matrix.md). This document
describes the transport behavior; the matrix records what each reader has
passed.

## Supported targets

| Target | Connstring | Backend | First-release scope |
|---|---|---|---|
| PN532 UART | `pn532_uart:/dev/ttyUSB0` | `serialport` + PN532 framing | Classic + Type 2 + raw bits/parity; nested/static-nested/hardnested recovery |
| ACR122U direct USB | `acr122_usb:<bus>:<address>` | `nusb` + CCID + ACR122 pseudo-APDU | Classic + Type 2 + raw bits/parity + Gen1A; nested/static-nested/hardnested recovery |
| ACR122U PC/SC | `acr122_pcsc:<reader name>` | `pcsc` + `libpcsclite` + ACR122 pseudo-APDU | Same ACR122 command set; capabilities follow the PC/SC hardware rows |

No other reader is advertised in the first release.

Both readers have an ISO-DEP/APDU code path for NFC Forum Type 4, but it is
not part of the advertised scope until a Type 4 read/write record is added to
`docs/notes/11-type4-ndef.md`. The transport-wide limit is 247 bytes per
command APDU because the ACR122U pseudo-APDU wrapper leaves that much room.

Post-release work in plan phases 13-16 uses the same two readers for ISO
14443B, high-rate ISO-DEP, Type 3/4B NDEF, China T-Union, FMCOS/Calypso, and
ICAO 9303 eMRTD. The EMV public-data profile is implemented on Type A; Type B
EMV still depends on phase 13. Phase 13 is the pending RF/transport
prerequisite; phases 14-16 are optional. Phase 17 implements the Amiibo /
NTAG215 workflow on the existing Type 2 path; its file path is validated and
its write path is Advanced and limited to owned magic NTAG215 tags. PN532 UART
completed a full write/read-back on 2026-11-15; the ACR122U record is pending.

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
- read-only Gen1A probe and UFUID seal preflight;
- environment-gated irreversible UFUID seal;
- raw transceive after driver stabilization;
- full Classic dump and per-sector key scan;
- temporary data-block write and restore with backups.

Current status: firmware identification, Classic 1K selection/authentication/
read, Type 2 selection/read, raw 7-bit REQA (16-bit ATQA response), full Classic
dump, key scan, temporary data-block write with backup and read-back, and
restore are validated on a PN532 (CH340 UART adapter, firmware 1.6). The
guarded Gen1A UID change and restore, sector-trailer write, and full restore
with UID passed on 2026-11-11 with an owned magic Classic 1K; see
`docs/notes/24c-uid-write.md` and `docs/notes/24d-restore-with-uid.md`. The
guarded UID change/restore also passed on 2026-11-15 with an owned UFUID
`47 D5 87 A9`. Its seal preflight and dry run passed, the seal command was then
applied with owner approval, and the card became permanently sealed; a 64/64
dump and block 0 verified unchanged and a new seal preflight was refused. The
first automated verification saw a false negative while the backdoor still
answered immediate probes; the workflow now polls for two consecutive negative
probes. See
`docs/notes/33-ufuid-seal.md`. On
2026-11-12 a Xiaomi wallet full MIFARE Classic 1K emulation was read 64/64
blocks in 1.2 s and a data block was written, verified, and restored; the
phone dump matched the source card byte for byte. The phone UID is copy-only;
an external reader cannot write it. See `docs/notes/26-phone-emulation.md`.
A 4K card run remains a future hardware check.

## ACR122U

The ACR122U has two links over the same ACR122 command layer. Pick the link
matching the current owner of the CCID interface:

- `acr122_usb:<bus>:<address>` in decimal, for example `acr122_usb:001:017`,
  when no PC/SC service holds the reader;
- `acr122_pcsc:<exact reader name>`, for example
  `acr122_pcsc:ACS ACR122U 00 00`, when `pcscd` owns the reader.

Direct USB enumeration reports the exact address for each attached `072F:2200`
reader; opening claims the CCID interface and validates the internal PN532
firmware. The PC/SC link enumerates only reader names containing `ACR122` and
connects to the exact name.

Implement:

- USB enumeration by VID/PID and exact bus/address connstring;
- CCID interface claim, bulk endpoints, bounded timeouts, abort, and stale
  bulk-IN draining;
- PC/SC context, exact reader lookup, `SCARD_SHARE_DIRECT` with the CCID
  escape control, and a T=1 fallback;
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

The driver reports a busy interface with the same instruction. For the PC/SC
path the service must be running (`systemctl start pcscd`, or socket
activation on the first PC/SC client). The two links never fall back to each
other.

PC/SC direct mode still gives the PN532 raw bit/parity control: the escape
control carries the same pseudo-APDU as the USB path, and the card stays
inactive until the driver selects it. The T=1 fallback is only for firmware
that rejects direct mode; its raw-bit and recovery rows must be validated
before being advertised on such a unit.

Hardware tests:

- select a Classic 1K and a Type 2 tag;
- read one Type 2 page through raw `READ`;
- read one Classic block with a known key;
- raw 7-bit REQA with a 16-bit ATQA response;
- Gen1A block 0 write only with explicit confirmation and read-back;
- repeated open/close cycles;
- PC/SC enumeration, exact-name open, firmware identification, Classic read
  and write, raw bits, recovery parity, and Gen1A;
- card removal and reconnect.

PC/SC status: validated on 2026-11-14 with an ACR122U at
`acr122_pcsc:ACS ACR122U 00 00`, internal PN532 firmware 1.6, through
`SCARD_SHARE_DIRECT` and the CCID escape control. Classic select/read/dump,
key scan, data-block and trailer writes with backups, UID change/restore,
Gen1A block 0, raw 7-bit REQA, recovery parity, and Classic NDEF
write/read/restore passed. Type 2 page/NDEF and Type 4/DESFire NDEF rows also
passed with the NTAG215 and the owned DESFire EV3.

Direct USB status: validated on 2026-11-10 with an ACR122U at
`acr122_usb:001:017` and internal PN532 firmware 1.6. The run covered firmware
identification, MIFARE Classic 1K block read with a known key, Type 2 page
read, raw 7-bit REQA, and a same-value Gen1A manufacturer-block write verified
by reselection and read-back. A physical unplug/replug was not exercised;
disconnect mapping is covered by scripted transport tests. See
`docs/notes/05-acr122-usb.md`. Phase 24c adds a guarded application service over
the same Gen1A primitive and an ordinary CUID/Gen2 path. The service path is
validated on both PN532 UART and ACR122U, and the ordinary block-0 backend is
validated on PN532 UART with a CUID/Gen2 Classic 1K card; phase 24d adds
restore with UID, also validated on both. Classic data-block and
sector-trailer writes, full dumps, and Classic NDEF format/write/read/restore
are validated on ACR122U.

### Type 2 write and NDEF status

The Type 2 READ path is validated on both readers. On 2026-11-10 a PN532 UART
at `pn532_uart:/dev/ttyUSB0` completed an authorized NTAG215
(`04 52 F5 67 D3 2A 81`) NDEF round trip: read 135/135 pages, format, write a
33-byte Text record, verify it, then restore the original user memory from a
project backup. The original URI NDEF was present after the restore. The write
path uses READ (0x30), WRITE (0xA2), and PWD_AUTH (0x1B), CRC-A, read-back
verification, and restored raw mode. ACR122U Type 2 NDEF write/read/restore is now recorded. Authenticated access
after AUTH0, including password configuration and clearing, passed on PN532
UART with an NTAG216 on 2026-11-12. One-way locks and the ACR122U protected
path have not been recorded yet. Run the gated checks in `docs/testing.md`
before advertising the remaining operations as hardware-validated. The phase 09
and phase 24e notes record the details.

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
- Generic or PC/SC-only readers (ACR122U PC/SC is supported)
- Proxmark3 USB device control
- Windows/macOS transport
- SPI/I2C and embedded Linux buses

Adding any of these requires an ADR, a driver crate boundary, and a hardware
validation record.
