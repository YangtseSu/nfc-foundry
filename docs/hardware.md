# Hardware scope and transport requirements

## Supported targets

| Target | Connstring | Backend | First-release scope |
|---|---|---|---|
| PN532 UART | `pn532_uart:/dev/ttyUSB0` | `serialport` + PN532 framing | Classic + Type 2 + raw bits/parity; nested/static-nested recovery |
| ACR122U | `acr122_usb:...` | `nusb` + CCID + ACR122 pseudo-APDU | Classic + Type 2 + raw bits/parity + Gen1A; nested/static-nested recovery |

No other reader is advertised in the first release.

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
- explicit rejection of non-PN532 firmware, missing cards, timeouts, and serial
  disconnects.

Hardware tests:

- list/select a Classic 1K and an NTAG215;
- read one Classic block with a known key;
- read Type 2 pages;
- same-value Classic write with explicit confirmation and read-back;
- raw transceive after driver stabilization;
- full Classic dump and per-sector key scan;
- temporary data-block write and restore with backups.

Current status: firmware identification, Classic 1K selection/authentication/
read, Type 2 selection/read, raw 7-bit REQA (16-bit ATQA response), full Classic
dump, key scan, temporary data-block write with backup and read-back, and
restore are validated on a PN532 (CH340 UART adapter, firmware 1.6). Trailer
writes and a 4K card run remain future hardware checks. See
`docs/notes/04-classic-workflows.md`.

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
`docs/notes/05-acr122-usb.md`.

Raw operations after a selected card require an RF field cycle to return the
card to IDLE; `InRelease` alone did not produce a REQA response on this reader.
`Acr122Usb::cycle_rf_field` performs that transition.

Recovery parity framing is validated on both readers: the PN532 and ACR122U
return a 32-bit tag nonce from a parity-disabled partial authentication and
complete full software Crypto1 authentication. Nested and static-nested
recovery are validated on both transports. A known response size avoids the
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
