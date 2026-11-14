# Phase 28 spec: ACR122U PC/SC transport

Date: 2026-11-14

## Goal

Add PC/SC as a second link for the ACR122U. The reader keeps the existing
ACR122 pseudo-APDU/PN532 command set; only the host link changes. A user
selects `acr122_pcsc:<reader name>` when `pcscd` owns the device, or
`acr122_usb:<bus>:<address>` when the CCID interface is free.

This is not generic PC/SC support. No other PC/SC reader is enumerated,
opened, or advertised.

## Deliverables

- `nfc-drivers::acr122` shared module:
  - `AcrLink`, the transport boundary used by both links;
  - `Acr122Config` and `Acr122Protocol` moved from `acr122_usb`;
  - `Acr122Reader`, the reader state and `Reader`/`ManufacturerWriter`
    behavior shared by both wrappers.
- `nfc-drivers::acr122_pcsc`:
  - `PcscLink` implementing `AcrLink`; `Context::establish`, exact reader
    lookup, `SCARD_SHARE_DIRECT` + CCID escape `SCARD_CTL_CODE(3500)`, and a
    `SCARD_SHARE_SHARED` T=1 fallback;
  - pseudo-APDU size checks, status-word splitting, and stable `NfcError`
    mapping for no-card, removed-card, timeout, sharing-violation,
    not-transacted, unavailable-reader, and stopped-service errors;
  - `Acr122Pcsc` implementing `Reader` and `ManufacturerWriter`;
  - exact `acr122_pcsc:<reader name>` connstring parsing;
  - enumeration filtered to `ACR122` reader names;
  - `diagnose()`/`pcsc_status()` for PC/SC availability and matched names.
- `DriverRegistry` classification, open, and enumeration for the third
  connstring family.
- `acr122-pcsc` cargo feature, default on; `--no-default-features` builds the
  direct-USB-only workspace.
- `nfctl diag`/`diag --json` and the GUI diagnostics page report PC/SC
  availability, `pcscd` reachability, and matched ACR122U names.
- `nfctl list` and the GUI driver list include `ACR122U PC/SC` in PC/SC builds.
- Hardware test `crates/nfc-drivers/tests/hardware_acr122_pcsc.rs` gated by
  `NFC_FOUNDRY_HARDWARE=1` and `NFC_FOUNDRY_DEVICE=acr122_pcsc:<name>`; write
  rows use the existing write gates.
- AUR/PKGBUILD: `pcsclite` runtime dependency, `pkgconf` makedepends. CI:
  `libpcsclite-dev`.
- Phase spec, note, ADR 0007, plan status, README, architecture, hardware,
  hardware matrix, testing, udev guidance.

## Non-goals

- Generic PC/SC readers or PC/SC-only readers.
- PN53x USB.
- Starting, stopping, or configuring `pcscd`.
- PC/SC on Windows/macOS.
- A second ACR122 protocol implementation, a C shim, or `unsafe` code in
  `nfc-drivers`.
- Automatic fallback between PC/SC and direct USB.
- New workflow, safety-gate, or confirmation logic; surfaces keep consuming
  `ReaderCapabilities`.

## Safety

- Enumeration never opens a card or sends a command.
- Only readers whose name contains `ACR122` are opened; other PC/SC readers are
  refused with `Unsupported`.
- Opening one transport never steals the interface from the other. A busy CCID
  interface still reports the stop-`pcscd` guidance; PC/SC contention reports
  the other-client guidance.
- The direct-mode failure is always reported when the T=1 fallback also fails;
  it is not hidden by a retry.
- No PC/SC card APDU is sent directly. Card commands continue through the PN532
  `InDataExchange`/`InCommunicateThru` path, so Type 4/DESFire behavior is
  shared with direct USB.
- Every advertised capability needs a hardware row for this transport.

## Acceptance

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo clippy --workspace --all-targets --no-default-features -- -D warnings`
- `cargo test --workspace`
- `cargo test --workspace --no-default-features`
- `cargo check -p nfc-foundry`
- `reuse lint`
- `acr122_pcsc` enumerates only ACR122U readers and opens the exact name.
- Direct USB behavior and tests are unchanged.
- Every advertised PC/SC capability has a hardware row.
- `nfctl diag --json` carries `pcsc.enabled`, `pcsc.available`,
  `pcsc.error`, and `pcsc.readers`.
