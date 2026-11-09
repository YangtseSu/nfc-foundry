# Phase 01 spec: Core and transport

Date: 2026-11-09

## Goal

Implement the card-family-neutral transport boundary, a scripted mock reader and
transport, the application device manager with an exclusive operation lease, and
mock-backed `nfctl list`, `scan`, and `card info`. No hardware is accessed in
this phase.

## Deliverables

- `CardInfo`, `CardType`, `Key`, `KeyType`, and `NfcError` stay in `nfc-core`;
  `NfcError` gains explicit `Timeout` and `NoReader` variants.
- `nfc_core::CancelToken`: shared atomic cancellation checked before reader I/O
  and between write verification steps.
- `nfc-transport`:
  - object-safe `Reader`, `TrailerWriter`, `ManufacturerWriter`, and
    `Transport` traits;
  - `Transport::enumerate` and `Transport::open` as the only driver factory
    boundary; `NoTransport` for callers without a backend;
  - `MockReader` with scripted card presence, block contents, authentication
    failure, timeout, cancellation, and ignored writes;
  - `MockTransport` sharing script state with every reader it opens.
- `nfc-drivers`: `DriverRegistry` implementing `Transport` for `pn532_uart:`
  and `acr122_usb:` connstrings. Device protocols remain `NotImplemented`.
- `nfc-app`:
  - `DeviceManager` with refresh, exact connstring selection, reader open/close,
    and transport injection;
  - `Operation` guard that owns the reader lease, caches card identity, checks
    cancellation before I/O, verifies writes by read-back, and exposes an
    explicit card-identity check for safe workflow boundaries;
  - `Service` facade used by the GUI and CLI.
- `nfctl`:
  - global `--mock` transport and global `--device <connstring>` selection;
  - `list`, `scan` (including `--watch`), and `card info` implemented against
    the mock transport;
  - all unimplemented domain commands keep returning `NotImplemented`.
- Phase 01 spec, note, plan status, README, and architecture updates.

## Non-goals

- No serial or USB access, PN532 framing, CCID, or card protocol.
- No Classic geometry, dump, key, or write workflow.
- No NDEF, Type 2, or recovery implementation.
- No cross-process reader lock or physical-reader identity.
- No hardware claims. Drivers still return `NotImplemented`.

## Acceptance

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`
- Transport traits compile as trait objects.
- Mock tests cover card change, authentication failure, timeout, cancellation,
  read/write verification, and missing reader selection.
- `nfc-app` does not depend on `nfc-drivers`; driver types never cross the
  transport boundary.
- CLI smoke:
  - `nfctl --mock list` lists the scripted readers;
  - `nfctl --mock --device mock:classic-1k scan` reports the scripted Classic
    card;
  - `nfctl --mock --device mock:ntag215 card info` reports the scripted Type 2
    tag.
