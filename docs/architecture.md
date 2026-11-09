# Architecture

## Shape

NFC Foundry is a Rust workspace with two first-class business domains over one
transport and one application service layer.

```text
GPUI Kit GUI / nfctl
        |
      nfc-app
   /     |      \
Classic  Recovery  NDEF
   |       |        |
 nfc-classic  nfc-recover  nfc-ndef
   \       |        /
       nfc-transport
            |
        nfc-drivers
       /           \
 pn532_uart     acr122_usb
```

Dependencies point downward. A lower crate never depends on a GUI feature, the
application shell, or a sibling feature.

## Crates

| Crate | Responsibility | Hardware |
|---|---|---|
| `nfc-core` | Card identity, card type, shared errors, ISO/IEC 14443A constants | no |
| `nfc-transport` | `Reader`, `Transport`, raw and capability traits, device facts | no |
| `nfc-drivers` | PN532 UART and ACR122U USB/CCID mechanics | yes |
| `nfc-classic` | Classic geometry, BCC, access bits, value blocks, dump, diff, exchange | no |
| `nfc-recover` | Crypto1, darkside, nested, staticnested, hardnested, FM11RF08S, auto | via transport |
| `nfc-ndef` | NDEF message/record, TLV, templates, validation | no |
| `nfc-type2` | Type 2 / NTAG pages, CC, lock/config, NDEF TLV adapter | via transport |
| `nfc-classic-ndef` | MAD1/MAD2, Classic CC, NDEF sectors, format | via transport |
| `nfc-type4` | ISO-DEP/APDU NDEF adapter (later) | via transport |
| `nfc-app` | Device manager, exclusive operations, services, events, backups | via transport |
| `nfctl` | English-only CLI and JSON contract | via `nfc-app` |
| `gui/nfc-foundry` | GPUI Kit shell and feature views | via `nfc-app` |

## Transport

The transport interface is card-family neutral:

```rust
trait Reader: Send {
    fn device(&self) -> &DeviceInfo;
    fn open(&mut self, conn_string: &str) -> Result<()>;
    fn close(&mut self) -> Result<()>;
    fn card_info(&mut self) -> Result<CardInfo>;
    fn authenticate(&mut self, block: u8, key_type: KeyType, key: Key) -> Result<()>;
    fn read_block(&mut self, block: u8) -> Result<[u8; 16]>;
    fn write_block(&mut self, block: u8, data: [u8; 16]) -> Result<()>;
    fn transceive_bytes(&mut self, tx: &[u8]) -> Result<Vec<u8>>;
    fn transceive_bits(&mut self, tx: &[u8], tx_bits: usize) -> Result<(Vec<u8>, usize)>;
    fn set_raw_mode(&mut self, enabled: bool) -> Result<()>;
}
```

Trailer and manufacturer writes are separate capability traits. Card families,
NDEF, and attack engines stay above this boundary. No OS handle, native
pointer, or driver-specific type crosses it.

Drivers are reached only through the factory trait:

```rust
trait Transport: Send + Sync {
    fn enumerate(&self) -> Result<Vec<DeviceInfo>>;
    fn open(&self, conn_string: &str) -> Result<Box<dyn Reader>>;
}
```

`DriverRegistry` in `nfc-drivers` implements it for the supported connstrings.
`nfc-app` receives an `Arc<dyn Transport>` and never names a driver type.
Trait objects are also what keeps the boundary object-safe and testable.

`Transport::open` only constructs a reader. `DeviceManager` then calls
`Reader::open` with the same exact connstring; that step performs the hardware
I/O and can fail with `Unsupported`, `Io`, or `Timeout`. The reader stays open
across operations and is closed on release or when a selected device
disappears.

### PN532 UART

`pn532_uart` is split into four layers:

- `frame`: normal and extended PN532 frames, ACK/NACK/error frames, length and
  data checksums, and resynchronization after junk.
- `link`: a small byte-transport trait, with `serialport` at 115200 8N1 for
  hardware and a scripted link for tests.
- `protocol`: wake-up, firmware validation, register setup, card selection,
  MIFARE authentication and block access through `InDataExchange`, and raw
  byte/bit exchange through `InCommunicateThru`.
- reader: exact-connstring open/close and the `Reader`/`TrailerWriter`
  implementations.

Every host command carries the HSU wake-up preamble, so the driver works after
the PN532 power-down state and while it is already awake. Card selection stores
the selected `CardInfo`; MIFARE authentication appends the last four UID bytes
after the key, as `InDataExchange` requires. `cycle_rf_field` powers the field
off and on and clears the cached selection so raw frames can start from an
IDLE card.

Raw mode disables hardware CRC and rejects ordinary MIFARE operations; ordinary
mode rejects raw transceive. The PN532 still generates and checks parity for raw
bit frames, so the transport trait carries bits and a received bit count rather
than caller-supplied parity. Enumeration lists serial ports without opening
them; only an exact open validates that the device is a PN532.

## Device ownership

One reader is held by one operation at a time. `nfc-app` owns:

- the device manager and device list;
- the cross-process reader lock;
- an exclusive reader lease for operations;
- cancellation and task events;
- pre-write backups and post-write verification.

`DeviceManager` opens the selected reader on first use and returns an
`Operation` guard that borrows it for the duration of the operation. The guard
checks a `CancelToken` before card I/O, reads every write back before reporting
success, and exposes an explicit identity check. Reselecting a card resets
Classic authentication, so workflows call the identity check at sector or
operation boundaries rather than between an authentication and its write. The
GUI, CLI, card polling, and recovery use the same manager. Recovery does not
hand the reader to another process.

Cross-process locking and physical-reader identity are added with the Classic
workflow phase; the in-process lease is already exclusive by borrow.

## Classic core

`nfc-classic` is the hardware-free Classic model. It owns:

- 1K/4K geometry: sectors, blocks, trailers, and access groups, including the
  sixteen-block large 4K sectors;
- manufacturer-block BCC validation for known four-byte UIDs;
- access-bit complement decoding, data and trailer permissions, and value
  blocks;
- `Dump` with a per-byte known mask and per-block status, so unknown bytes are
  never confused with known zero bytes;
- readability counts, trailer validation, and block diff with byte and trailer
  detail;
- dump interchange for project JSON, raw images, MCT text, and Proxmark3
  `mfc v2`/`mfc v3` JSON.

The project format is the only format that represents partial images, masked
bytes, and malformed trailers. Raw, MCT, and Proxmark3 exports require a
complete image with valid trailers; BCC is checked when the card identity is
known. Imports reject duplicate JSON keys, missing blocks, inconsistent
addresses, broken trailer complements, and Proxmark3 SectorKeys that disagree
with trailer bytes. The exchange layer works on bytes; the workflow layer owns
file paths, sidecars, and backup policy.

## Classic and NDEF boundary

Classic block geometry and NDEF are separate models:

- `nfc-classic` knows sectors, blocks, trailers, access bits, dumps, and dump
  interchange.
- `nfc-ndef` knows messages, records, TLVs, and templates; it has no card
  knowledge.
- `nfc-type2` maps NDEF TLVs onto Type 2 pages and CC/locks.
- `nfc-classic-ndef` maps NDEF TLVs onto MAD/NDEF sectors and access
  conditions.
- `nfc-app` composes family-specific workflows from these crates.

This keeps NDEF testable without a reader and prevents a Classic page editor
from becoming the NDEF implementation.

## Recovery

`nfc-recover` is a library, not a process wrapper. It owns:

- Crypto1/LFSR/PRNG primitives ported from the current Proxmark3 tree;
- attack implementations and the `auto` strategy;
- nonce acquisition through `nfc-transport`;
- progress and cancellation callbacks;
- key/dump result types.

The attack engine receives an exclusive reader lease from `nfc-app`, not a raw
device handle. It must reselect and verify the card before returning success.

## GUI

GPUI Kit is the only GUI dependency. The application shell composes feature
crates:

```text
gui/app
  gui/reader
  gui/classic
  gui/recovery
  gui/type2
  gui/ndef
  gui/keys
  gui/settings
```

- `RenderOnce` for value-like presentation.
- `Entity<T>` for input, tables, scrolling, focus, and async state.
- No domain mutation or hardware I/O in `render`.
- Theme tokens only; no raw colors or spacing literals in application code.
- Every user-visible string is localized; English is the source language and
  Simplified Chinese must stay complete.

## Errors and cancellation

- `nfc-core::NfcError` is the shared error type across hardware and domain
  boundaries.
- CLI maps errors to stable exit codes; JSON output carries a stable code and
  message.
- `nfc-core::CancelToken` is a shared atomic flag checked before reader I/O and
  between write verification steps. Long operations surface
  `NfcError::Cancelled`; hardware timeouts are explicit and bounded.
