# Phase 06 implementation note: reader expansion and capability matrix

Date: 2026-11-07

## Implemented

- The application build profile is now recorded as
  `pn532_uart,pn53x_usb,acr122_usb,acr122_pcsc,pcsc` in
  `third_party/libnfc/libnfc.lock`. The library's own default driver set and
  `--with-drivers=all` are unchanged.
- `internal/nfc` owns the reader capability profile:
  - known drivers: `pn532_uart`, `pn53x_usb`, `acr122_usb`, `acr122_pcsc`,
    `pcsc`;
  - per-profile enumerate, ISO14443A select, Classic read/write, Type 2
    read/write, raw bytes, raw bits, ISO-DEP/APDU, recovery-engine support,
    validation level, and note;
  - unknown connstrings stay usable for development/test backends but are
    reported as unverified and never advertise recovery support.
- The C shim reports the drivers registered in the linked libnfc, instead of a
  hardcoded `pn532_uart`. `nfctl diag` prints the list.
- `libnfc.Reader` rejects operations outside the profile before sending a
  native command. Application services add family-level checks with actionable
  messages for Classic, Type 2, APDU, and UID workflows.
- `nfctl list` and `nfctl scan` expose driver, capability, and validation
  fields in JSON and a compact capability line in human output.
- The GUI device rail shows Classic, Type 2, ISO-DEP, raw, recovery,
  experimental, and unverified capability pills. Classic/Type 2 read actions
  and Type 2 writes are disabled when the selected known reader cannot perform
  them.
- Enumeration prefers direct ACR122 USB and suppresses the matching
  `acr122_pcsc` and `pcsc` aliases. An explicitly passed connstring is still
  honored.
- The cross-process lock groups all ACR122 connstrings under one physical
  identity, so direct USB and PC/SC cannot be held by two processes at once.
- `resources/udev/70-nfc-foundry.rules` grants `TAG+="uaccess"` for the
  supported ACR122 and PN53x USB IDs. `scripts/package-runtime.sh` includes it
  in the application tree and binary tarball.
- PC/SC and USB open failures add an actionable hint about `pcscd` and a
  competing claim without changing the stable error category.

## Recovery capability decision

Raw-frame support alone does not prove that an external recovery engine works
with a reader. Nested, Darkside, and Hardnested stay advertised only for the
PN532 UART profile. ACR122 USB/PC-SC and PN53x USB can run ordinary Classic and
Type 2 workflows, but recovery commands reject them until hardware validation
is recorded.

## Verification

Hardware-free:

- `go test ./...` passes.
- `make libnfc-binding-test` passes against the five-driver SDK.
- `go vet ./...` passes.
- `go tool mygo i18n check .` reports 514 messages in English and Simplified
  Chinese.
- `make ui-test` passes.
- New unit tests cover profile lookup, unknown-driver behavior, DTO/JSON
  fields, unsupported-operation gating, native-call suppression, ACR122
  duplicate collapse, cross-driver physical locking, and connection hints.

Native profile:

```text
NFCX_LIBNFC_SOURCE_DIR=/path/to/libnfc-ng make libnfc-ng-build
make libnfc-ng-verify
```

`make libnfc-ng-verify` passed with
`pn532_uart,pn53x_usb,acr122_usb,acr122_pcsc,pcsc`. The installed Makefile
contained all five `DRIVER_*_ENABLED` flags.

## Hardware validation

Reader: ACS ACR122U (USB `001:004`) and a PN532 UART adapter
(`/dev/ttyUSB0`). Cards present during the run: MIFARE Classic 1K
`FA B9 CF F4` and NTAG215 `04 34 4C C2 83 5C 80`.

`nfctl list` enumerated the ACR122U once:

```text
acr122_usb:001:004  acr122_usb:001:004
  classic read/write, type2 read/write, iso-dep, raw bits, partially hardware-validated
```

The ACR122 PC/SC and generic PC/SC aliases were suppressed as designed.
`pn532_uart` does not participate in libnfc autoscan; it is opened by exact
connstring.

Direct USB and PN532 UART selection:

```text
nfctl scan --device acr122_usb:001:004
  MIFARE Classic 1K, UID FA B9 CF F4, ATQA 00 04, SAK 08

nfctl scan --device pn532_uart:/dev/ttyUSB0
  MIFARE Classic 1K, UID FA B9 CF F4, ATQA 00 04, SAK 08
```

PN532 UART lifecycle smoke passed:

```text
make libnfc-binding-hardware-smoke LIBNFC_DEVICE=pn532_uart:/dev/ttyUSB0
```

It includes 100 open/close cycles, one selection, and 20 repeated `CardInfo`
calls.

PC/SC drivers on the same ACR122U:

```text
nfctl scan --device 'acr122_pcsc:ACS ACR122U 00 00'
  failed: nfc open: operation timed out

nfctl scan --device 'pcsc:ACS ACR122U 00 00'
  opened, selected ISO/IEC 14443-4 target
  UID 04 34 4C C2 83 5C 80, ATQA 00 00, SAK 20
```

The generic PC/SC profile correctly advertises selection only. On that reader
it saw the NTAG215 while direct USB selected the Classic 1K, so the two paths
did not observe the same physical target during the run. The point of this
test is the capability boundary, not card-family behavior.

Unsupported-operation checks on `pcsc`:

```text
nfctl classic read --block 1 --device 'pcsc:ACS ACR122U 00 00'
  nfc read block: reader "pcsc:..." does not support MIFARE Classic read operations

nfctl apdu --raw 00A4040000 --device 'pcsc:ACS ACR122U 00 00'
  nfc APDU operation: reader "pcsc:..." does not support ISO-DEP/APDU operations
```

Both exited with code 4 and did not send a card command.

Physical lock test:

```text
nfctl scan --device 'pcsc:ACS ACR122U 00 00' --watch   # held in one process
nfctl scan --device acr122_usb:001:004                 # second process
  reader "acr122_usb:001:004" is in use by nfctl (pid ...)
  exit code 4
```

The reverse direction is covered by the same identity mapping; after the PC/SC
client exited, direct USB opened again.

`acr122_usb` lifecycle smoke did not pass as a full 100-cycle test. It failed
with `Unable to claim USB interface` after a PC/SC client had used the reader,
and then with the previously recorded transient `PN532 init failed` status
words. A single `nfctl scan` succeeded before and after those runs. This is the
known ACR122U USB ownership/init instability recorded in Phases 02, 03, 04, and
05f, not a new failure mode.

`pn53x_usb` had no attached device, so it remains unvalidated.

## Problems and resolutions

- Compile-time `DRIVER_*_ENABLED` macros are private to the libnfc build and
  are not part of the installed pkg-config flags. The shim reads the exported
  `nfc_drivers` registration list, copies only the driver names, and captures
  the names once. It initializes a temporary context only when the list has not
  been created yet.
- A known driver with a partial implementation cannot share one boolean with a
  validated driver. `pcsc` therefore has explicit false card-operation flags,
  while its selection capability stays true.
- ACR122 enumeration exposed the same reader three times. Direct USB is the
  preferred path; the PC/SC aliases are hidden unless explicitly requested.
- The connstring lock could not stop a USB process and a PC/SC process from
  claiming one ACR122. The lock identity now maps both to `acr122`.
- The PC/SC ownership conflict is visible to users through the new open hint;
  the direct-USB failure after a PC/SC client is expected and recoverable.

## Deviations from the spec

- Recovery engines stay advertised only for PN532 UART. The spec originally
  left open whether every raw-capable profile should claim them; the
  conservative rule avoids advertising an engine/firmware combination with no
  hardware record.
- The generic `pcsc` driver is selection-only by scope. PC/SC is a smart-card
  and APDU API, not a raw NFC transport; Classic, Type 2, Gen1A, and recovery
  engines are not implemented over it. An optional future PC/SC-native APDU
  transport would cover ISO-DEP/Type 4/DESFire only.
- `pcsc` did select an ISO14443-4 target on the available ACR122U, unlike the
  Phase 02 note. The profile still advertises selection only and no card
  workflow, so the capability contract is unchanged.

## Limitations and follow-up

- Validate `pn53x_usb` when hardware is available. Supported devices are the
  NXP PN531/PN533 USB dongles, SCM SCL3711/SCL3712, Sony PaSoRi RC-S360, and
  ASK LoGO. ACR122U and PN532 UART adapters are different drivers.
- Validate `acr122_pcsc` on a reader/firmware combination where it opens, or
  remove it from the profile if direct USB remains the only Linux path.
- Optionally implement a PC/SC-native APDU transport for ISO-DEP/Type 4/DESFire.
  Do not implement Classic/Type 2/Gen1A/recovery over generic PC/SC.
- Recovery engines remain PN532 UART-only until validated elsewhere.
- The ACR122 physical lock is intentionally broader than one USB device, so
  two processes cannot simultaneously use two different ACR122 readers. This
  is the price of preventing a USB/PC-SC double claim without a reliable
  PC/SC-to-USB identity mapping.
- ACR122U's transient USB claim/init failures remain a hardware/firmware
  behavior; retrying the process is the current workaround.
