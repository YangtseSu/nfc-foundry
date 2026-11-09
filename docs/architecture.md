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
| `nfc-classic` | Classic geometry, BCC, access bits, value blocks, dump, diff | no |
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

## Classic and NDEF boundary

Classic block geometry and NDEF are separate models:

- `nfc-classic` knows sectors, blocks, trailers, access bits, and dumps.
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
