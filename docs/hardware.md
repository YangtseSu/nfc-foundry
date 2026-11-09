# Hardware scope and transport requirements

## Supported targets

| Target | Connstring | Backend | First-release scope |
|---|---|---|---|
| PN532 UART | `pn532_uart:/dev/ttyUSB0` | `serialport` + PN532 framing | Classic + Type 2 + raw bits/parity |
| ACR122U | `acr122_usb:...` | `nusb` + CCID + ACR122 escape | Classic + Type 2 + raw bits/parity + Gen1A |

No other reader is advertised in the first release.

## PN532 UART

Implement:

- serial open/close, baud rate, read/write timeouts;
- PN532 frame length/checksum and ACK handling;
- `InListPassiveTarget` / card selection;
- `InCommunicateThru` for ISO/IEC 14443A bytes;
- raw bit transmission with parity/last-bit control;
- retries only with bounded counts and cancellation.

Hardware tests:

- list/select a Classic 1K and an NTAG215;
- read one Classic block with a known key;
- read Type 2 pages;
- raw transceive after driver stabilization.

## ACR122U

Target direct USB/CCID only. Do not use PC/SC in the first release.

Implement:

- USB enumeration by VID/PID and optional bus/address connstring;
- CCID bulk/interrupt endpoints, timeouts, and abort;
- ACR122 escape commands for raw ISO/IEC 14443A bytes and bits;
- reader LED/buzzer behavior only where required for protocol state;
- udev permission guidance.

Rationale: the current validated path is `acr122_usb`; PC/SC pseudo-APDUs do
not provide the same raw bit/parity control required by Classic recovery and
Gen1A.

Hardware tests:

- select a Classic 1K and NTAG215;
- read/write one Type 2 page;
- read one Classic block with a known key;
- Gen1A block 0 write only with explicit confirmation;
- disconnect/reconnect and lock recovery.

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
