# Phase 28 plan: ACR122U PC/SC transport

Status: pending 2026-11-12. Decisions: add **ACR122U PC/SC only**, not generic
PC/SC reader support. Direct USB/CCID remains the default ACR122U transport.

## Goal

Add a second transport for the same ACR122U reader: PC/SC. The reader still
speaks the existing ACR122 pseudo-APDU/PN532 command set; only the link layer
changes. A user may select `acr122_pcsc:<reader-name>` when `pcscd` owns the
device, or `acr122_usb:<bus>:<address>` when the CCID interface is free.

This is **not** a generic PC/SC abstraction. No other PC/SC reader is
enumerated, opened, or advertised.

## Decisions and constraints

- ADR 0007 supersedes the ACR122U-PC/SC exclusion in ADR 0002. Generic
  PC/SC-only readers and PN53x USB stay out of scope.
- The PC/SC path uses the `pcsc` crate over the system `libpcsclite`; this is
  a system API, not a project C shim. There is no `unsafe` code in
  `nfc-drivers`; the dependency owns the FFI.
- Reuse `Acr122Protocol`; do not implement a second ACR122 command layer.
- Direct USB and PC/SC must not be used simultaneously. `pcscd` owns the CCID
  interface for the PC/SC path; direct USB claims it with `nusb`.
- Capabilities fail closed. A capability is advertised only after its hardware
  row is recorded for this transport.
- The first release of this phase may land read-only operations before writes;
  the capability profile follows the validated rows, not the code path.

## Slices

### 28.1 Extract the shared ACR122 protocol

- Move `AcrLink`, `Acr122Config`, and `Acr122Protocol` out of
  `acr122_usb/protocol.rs` into a shared `nfc-drivers::acr122` module.
- Keep `acr122_usb` as a link implementation plus the existing `Reader` wrapper.
- No behavior change: the same pseudo-APDU framing, response parsing, retries,
  raw-bit handling, and firmware handshake must remain byte-for-byte
  equivalent.
- Re-run the existing ACR122 scripted tests against the refactor before adding
  PC/SC.

### 28.2 PC/SC link

- Add `crates/nfc-drivers/src/acr122_pcsc/` with:
  - `PcscLink` implementing `AcrLink`;
  - `Acr122Pcsc` implementing `Reader` and `ManufacturerWriter`;
  - exact connstring parsing.
- `PcscLink` responsibilities:
  - create a PC/SC context and connect to the selected reader;
  - send the ACR122 pseudo-APDU `FF 00 00 00 ...` through `SCardTransmit`;
  - keep the existing `SW 61 xx` / `GetResponse` continuation handling in
    `Acr122Protocol`;
  - map `SCARD_E_NO_SMARTCARD`, `SCARD_W_REMOVED_CARD`, `SCARD_E_TIMEOUT`,
    `SCARD_E_SHARING_VIOLATION`, and `SCARD_E_NOT_TRANSACTED` to stable
    `NfcError` values;
  - release the card/context on `shutdown`.
- Connect strategy, to validate on hardware:
  - start with `SCARD_SHARE_DIRECT` and the raw protocol for the pseudo-APDU
    path, which can open/read the reader without a card;
  - if the ACR122U firmware rejects card commands in direct mode, use
    `SCARD_SHARE_SHARED` with T=1 for card operations and direct mode for
    reader/firmware commands;
  - record the chosen mode in the phase note. Do not hide a failed direct-mode
    attempt.
- No PC/SC card APDU is sent directly. `transceive_apdu` continues through the
  existing PN532 `InDataExchange` path, so Type 4/DESFire behavior is shared
  with direct USB.
- Operation timeouts: PC/SC transmit is blocking. Use the existing
  `CancelToken` and bounded operation deadline; map a stuck transmit to a
  timeout only when the platform reports one. Do not spawn a watchdog thread
  that can close a handle under another transmit.

### 28.3 Enumeration and registry

- Enumerate PC/SC readers through the `pcsc` context.
- Filter to ACR122U only:
  - case-insensitive reader-name match for `ACR122`;
  - optionally confirm ACS vendor attributes when available;
  - never list another PC/SC reader, even if it is a contactless reader.
- Connstring: `acr122_pcsc:<exact reader name>`.
- `DeviceInfo.name` is `ACR122U PC/SC`; the exact reader name is the target.
- Add `acr122_pcsc:` to `DriverRegistry::classify/open`.
- `nfctl list`/GUI may show both `ACR122U USB` and `ACR122U PC/SC` when both
  are visible. Label the transport; do not auto-fallback between them.
- If `pcscd` is not running, PC/SC enumeration returns no ACR122U devices with
  a diagnostic, not an error that hides the direct USB path.
- If direct USB open fails because `pcscd` owns the interface, the existing
  error remains. If PC/SC open fails because direct USB owns it, report the
  same actionable guidance.

### 28.4 Capabilities and surfaces

- Add a `TransportKind` or equivalent diagnostic fact so `nfctl diag`, the GUI
  reader rail, and the hardware matrix can distinguish USB vs PC/SC.
- `ReaderCapabilities` starts with the operations already hardware-validated
  for this transport. Do not copy the direct-USB profile blindly. Candidate
  order:
  1. classic read;
  2. Type 2 read;
  3. classic write, Type 2 write, NDEF;
  4. raw bytes/raw bits;
  5. recovery parity;
  6. manufacturer write/Gen1A;
  7. APDU/Type 4 and DESFire.
- GUI/CLI gates already consume `ReaderCapabilities`; no new workflow logic.
- `nfctl diag --json` reports PC/SC availability, `pcscd` status if cheaply
  observable, and the matched ACR122U reader names.

### 28.5 Packaging, docs, hardware

- Cargo: add `pcsc` as a default-on feature (`acr122-pcsc`) so release builds
  include it; `--no-default-features` builds direct-USB-only. The feature is
  not a runtime toggle.
- AUR/package: add `pcsclite` runtime dependency and `pkgconf` makedepends.
- CI: install `libpcsclite-dev` for Linux builds.
- README, `docs/architecture.md`, `docs/hardware.md`,
  `docs/hardware-matrix.md`, and `docs/testing.md` list the second transport
  and its caps.
- Hardware test `crates/nfc-drivers/tests/hardware_acr122_pcsc.rs` gated by
  `NFC_FOUNDRY_HARDWARE=1` and `NFC_FOUNDRY_DEVICE=acr122_pcsc:<name>`; write
  rows require the existing write gates.
- Record rows for:
  - enumeration/open/firmware;
  - Classic read and same-value write;
  - Type 2 read/write and NDEF round trip;
  - raw 7-bit REQA/parity;
  - recovery parity if an owned card is available;
  - Gen1A same-value write;
  - Type 4 APDU/NDEF if hardware is available;
  - card removal and reconnect;
  - pcscd/direct-USB contention errors.

## Acceptance

- `acr122_pcsc` enumerates only ACR122U readers and opens the exact named
  reader.
- The same `Acr122Protocol` commands run over PC/SC; no duplicated PN532
  protocol code.
- Direct USB behavior and tests are unchanged.
- `cargo test --workspace` is hardware-free and passes with the feature on and
  off.
- Every advertised PC/SC capability has a hardware row; unvalidated operations
  are absent from `ReaderCapabilities`.
- No generic PC/SC reader, PN53x USB, or PC/SC write path for another device is
  added.
- `reuse lint` passes and no new `unsafe` is added to the workspace.

## Non-goals

- Generic PC/SC readers.
- PN53x USB.
- Starting/stopping `pcscd` or changing system smart-card policy.
- PC/SC support on Windows/macOS.
- A second ACR122 protocol implementation or a C shim.

## First files

- `crates/nfc-drivers/src/acr122/` (shared protocol/link).
- `crates/nfc-drivers/src/acr122_pcsc/`.
- `crates/nfc-drivers/src/lib.rs`.
- `crates/nfc-drivers/Cargo.toml`.
- `crates/nfc-drivers/tests/hardware_acr122_pcsc.rs`.
- `docs/adr/0007-acr122u-pcsc.md`.
- `AGENTS.md`, `docs/architecture.md`, `docs/hardware-matrix.md`.
