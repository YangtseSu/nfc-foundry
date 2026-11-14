# Phase 28 note: ACR122U PC/SC transport

Date: 2026-11-14

## Implemented

- `nfc-drivers::acr122` is the shared ACR122 command layer:
  - `AcrLink` (power-on, pseudo-APDU exchange, shutdown);
  - `Acr122Config` and `Acr122Protocol`, moved from `acr122_usb` without
    behavior change;
  - `Acr122Reader`, the shared reader state and `Reader`/`ManufacturerWriter`
    behavior. `acr122_usb` and `acr122_pcsc` are thin wrappers that supply
    their link factory and capability profile.
- `nfc-drivers::acr122_pcsc`:
  - exact `acr122_pcsc:<reader name>` connstrings and a guard that refuses
    non-ACR122 reader names;
  - `SCARD_SHARE_DIRECT` with `SCARD_CTL_CODE(3500)` (the CCID escape control
    the ACR122 firmware accepts on Linux) for the pseudo-APDU path;
  - `SCARD_SHARE_SHARED` with T=1 as the fallback; both failures are reported
    together with a contention hint;
  - status-word splitting, `SCARD_E_NO_SMARTCARD`/`SCARD_W_REMOVED_CARD` to
    `NoCard`, `SCARD_E_TIMEOUT` to `Timeout`, sharing-violation,
    not-transacted, unavailable-reader, and stopped-service messages;
  - `SCARD_LEAVE_CARD` disconnect and context release on `shutdown`/drop;
  - enumeration filtered to reader names containing `ACR122`;
  - `diagnose()`/`pcsc_status()` for PC/SC availability and matched names.
- `DriverRegistry` classifies, opens, and enumerates the third connstring
  family. `ACR122_PCSC_ENABLED` reports the compiled feature.
- `acr122-pcsc` feature, default on. `nfctl` and the GUI forward it and drop
  their default when `--no-default-features` is used.
- `nfctl diag`, `diag --json`, and the GUI diagnostics page report PC/SC
  availability, `pcscd` reachability, and matched reader names.
- `nfctl list` and the GUI driver list include `ACR122U PC/SC` when the
  feature is compiled in.
- Packaging/CI: `pcsclite` runtime dependency, `pkgconf` makedepends,
  `libpcsclite-dev` in both workflows.
- `crates/nfc-drivers/tests/hardware_acr122_pcsc.rs` covers enumeration,
  firmware, Classic/Type 2 reads, raw bits, Gen1A, and recovery parity with
  the existing environment gates.

## Tested

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo clippy -p nfc-drivers --all-targets --no-default-features -- -D warnings`
- `cargo test --workspace`
- `cargo test -p nfc-drivers --no-default-features`
- `cargo check --workspace --no-default-features`
- Driver unit tests: 70 with the feature, 63 without.
- GTK scenario `gtk_workbench_scenarios` passes under Xvfb with
  `NFC_FOUNDRY_GUI_TESTS=1` after the DESFire translation completion. The
  scenario failed at `HEAD` before this change (verified in a clean worktree).
- Hardware suite on 2026-11-14 with `pcscd` active (socket activation) and the
  reader at `acr122_pcsc:ACS ACR122U 00 00`, internal PN532 firmware 1.6:
  - enumeration lists the exact name next to `acr122_usb:001:002`;
  - Direct-mode open, firmware `PN532 IC 32 · firmware 1.6 · support 07`;
  - Classic 1K `FA B9 CF F4`, block 4 with Key A `D3F7D3F7D3F7` read;
  - `nfctl classic keyscan`: 44 attempts, 16 verified Key A;
  - `nfctl classic dump`: 64/64 blocks, 16/16 sectors;
  - data-block write/restore with project backups (block 4);
  - sector-trailer write/restore with project backups (block 11, GPB
    `40` → `BF` → `40`);
  - guarded UID change/restore through `nfc-app`: `FA B9 CF F4` →
    `FA B9 CF F5` → `FA B9 CF F4`;
  - Gen1A manufacturer block 0 same-value write, verified by reselection;
  - raw 7-bit REQA: ATQA `04 00`, 16 bits;
  - Crypto1 parity probe: 32-bit tag nonce;
  - Classic NDEF: Text record written (3 blocks), read back, original NDEF
    restored;
  - Type 2 / NTAG215 `04 34 4C C2 83 5C 80`: 135/135 pages read, NDEF vCard
    decoded, capacity 491 bytes;
  - Type 2 NDEF format, 33-byte Text record write/read, original user memory
    restored from the exported image;
  - DESFire EV3 8K: inventory and `applications: none`;
  - Type 4 NDEF over the AN11004 `D27600` application: CC 2.0 (15 bytes),
    MLe 59/MLc 52, empty NDEF; Text record written and read back, empty state
    restored; application deleted and `applications: none` verified.
- The `nfctl` hardware tests accept the PC/SC connstring: the Classic dump,
  data-block write/restore, UID, Type 2 read/NDEF, and Type 4 read/write rows
  passed.

## Problems and resolutions

- `SCARD_CTL_CODE(1)` returns `SCARD_E_NOT_TRANSACTED` with this reader and
  pcsc-lite 2.5.3. The ACR122U firmware uses `SCARD_CTL_CODE(3500)`; the
  driver uses that code.
- A T=0|T=1 connect request logs `Protocol T=0 not supported by reader` from
  libccid and fails with `SCARD_E_UNRESPONSIVE_CARD`. Request T=1 only.
- The ACR122U reports ICC present with ATR `3B 00` when no card is inserted;
  card-mode connect is not a reliable card-presence test. Direct mode plus the
  driver's own `InListPassiveTarget` is the card path.
- Direct mode still selects cards with `InListPassiveTarget` and supports
  raw bits/parity, so recovery and Gen1A keep working over PC/SC. The T=1
  fallback is not exercised by this firmware.
- `pcscd` with `--auto-exit` keeps the CCID interface claimed until its idle
  timeout. Starting PC/SC work is immediate through socket activation; direct
  USB work needs the service stopped and the interface released.
- The GTK scenario exposed pre-existing untranslated DESFire key-management
  labels at `HEAD` (verified in a clean worktree). The DESFire key screen now
  has Simplified Chinese catalog entries, and two hint strings that carried
  copy-pasted extra whitespace match their catalog entries again.
- The generic sector-trailer hardware test assumes the trailer write
  permission is Key A. This card uses `7F 07 88`, so the full trailer write
  requires Key B and the test's masked-Key-A data is refused by the service.
  The row was validated with an explicit Key A/Key B trailer write through
  `nfctl` instead; no product behavior changed.

## Pending

- Card removal and reconnect on PC/SC.
- The trailer test's Key-A assumption (see above); the row itself is
  validated with explicit key bytes.
