# Phase 01 note: Core and transport

Date: 2026-11-09

## Implemented

- `nfc_core::CancelToken` with shared atomic cancellation and
  `NfcError::Cancelled` checks.
- Explicit `NfcError::NoReader` and `NfcError::Timeout` variants.
- Object-safe `Transport` factory trait with `enumerate` and `open`, plus
  `NoTransport` for callers without a backend.
- `MockReader` scripted through a shared `MockHandle`:
  - card presence and card replacement;
  - block contents, sector-trailer, and manufacturer-block writes;
  - injected authentication failure, missing card, timeout, cancellation, and
    ignored-write faults per call.
- `MockTransport` that creates readers sharing script state with the test.
- `DriverRegistry` for `pn532_uart:` and `acr122_usb:` connstrings. A driver
  still returning `NotImplemented` contributes no devices; other enumeration
  errors stay visible.
- `DeviceManager` with transport injection, device refresh, exact connstring
  selection, reader open/close, and selection loss handling.
- `Operation` lease guard with operation kind, cached card identity, cancellation
  checks before card I/O, explicit card-identity checks for workflow safe
  boundaries, and read-back verification for writes.
- `nfctl` global `--mock` and `--device`, implemented `list`, `scan`
  (`--watch`), and `card info`; the remaining domain commands still return
  `NotImplemented`.

## Tested

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`
- Mock tests: card change, authentication failure, timeout, cancellation,
  verified write, ignored write rejected by read-back, missing reader, object
  safety of all transport traits.
- CLI smoke:
  - `nfctl --mock list`
  - `nfctl --mock --device mock:classic-1k scan`
  - `nfctl --mock --device mock:ntag215 card info`
  - `nfctl --mock scan` without `--device` returns exit code 2 and an
    actionable message.
  - `nfctl --mock --device mock:classic-1k classic read` still returns
    `NotImplemented` and exit code 2.

## Problems and resolutions

- The phase 00 mock owned card and block state inside the reader, so a test
  could not change the card while a service held the reader. State moved into a
  shared `MockHandle`, and `MockTransport` hands the same handle to every
  opened reader.
- Enumeration initially would have failed the entire device list while either
  driver was unfinished. `DriverRegistry` now skips `NotImplemented` driver
  contributions and propagates real errors, so phase 02 can enable PN532
  enumeration without waiting for ACR122U.
- An object-safety test briefly asserted the formatted text of a mock error.
  It now matches the error variant so later wording changes do not break it.

## Deviations

- `scan --watch` polls every 250 ms and stops on process signal; wiring the
  shared `CancelToken` into the watch loop belongs with the long-running
  workflow phase.
- Plain `nfctl list` reports no readers until a driver implements enumeration;
  the mock commands are the phase 01 surface.
- Cross-process reader locking and physical-reader identity are not implemented.

## Remaining blockers

- PN532 UART and ACR122U USB/CCID protocol implementations.
- Cross-process reader lock and hardware lease validation.
- Classic, NDEF, Type 2, and recovery workflows.
