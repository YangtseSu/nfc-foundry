# Phase 06 spec: reader expansion and capability matrix

Date: 2026-11-07

## Goal

Ship the five-driver application profile as one explicit, verified build
choice, and make the reader capability model visible before an operation is
attempted. A compiled driver is not the same thing as a validated reader, and a
reader that enumerates a card is not guaranteed to support every card command.

## Driver profile

The application build profile is:

```text
pn532_uart,pn53x_usb,acr122_usb,acr122_pcsc,pcsc
```

`libnfc-ng`'s own default driver set and `--with-drivers=all` remain unchanged.
The profile is recorded in `third_party/libnfc/libnfc.lock` and used by
`scripts/toolchain/libnfc.sh`, the Makefile hardware targets, and the
application package build. The runtime reports the registered drivers from the
linked library.

## Capability model

Add one profile per known driver. Each profile declares:

- enumerate;
- ISO/IEC 14443A select;
- MIFARE Classic read and write;
- Type 2 read and write;
- raw bytes and raw bits;
- ISO-DEP/PC/SC APDU;
- Nested, Darkside, and Hardnested recovery support;
- a validation level and a short note.

| Driver | Classic | Type 2 | Raw bits | ISO-DEP | Recovery | Validation |
|---|---|---|---|---|---|---|
| `pn532_uart` | yes | yes | yes | yes | nested/darkside/hardnested | partial hardware |
| `pn53x_usb` | yes | yes | yes | yes | none | none |
| `acr122_usb` | yes | yes | yes | yes | none | partial hardware |
| `acr122_pcsc` | yes | yes | yes | yes | none | none |
| `pcsc` | no | no | no | no | none | none |

Rules:

- A known driver that does not support an operation is rejected with
  `CodeUnsupported` before any hardware command is sent.
- An unknown driver keeps the implementation-capable profile so custom
  development backends and test doubles continue to work, but it is reported
  as an unverified profile, not as validated support.
- The character-device `pcsc` driver is APDU-oriented. NFC Foundry has no
  PC/SC-native APDU transport yet, so its APDU capability stays unavailable
  rather than sending PN53x frames through `SCardTransmit`.
- `acr122_pcsc` keeps the PN53x raw path and is therefore feature-equivalent to
  `acr122_usb` for ordinary card operations, but it is unvalidated.
- Recovery engines stay advertised only for the validated PN532 UART profile.
  Raw-frame support on another driver is not enough to claim engine support.

## Device selection and ownership

One physical ACR122U currently appears three times when direct USB, the ACR122
PC/SC driver, and the generic PC/SC driver are all compiled:

```text
acr122_usb:001:004
acr122_pcsc:ACS ACR122U PICC Interface 00 00
pcsc:ACS ACR122U PICC Interface 00 00
```

- Enumeration prefers direct USB and suppresses the matching ACR122 PC/SC
  entries. An explicitly passed connstring still opens.
- The cross-process lock uses a physical identity for ACR122 readers so the USB
  and PC/SC connstrings cannot be held by two processes at the same time.
- Other PC/SC readers keep their own connstring lock. A direct-USB PN53x and a
  generic PC/SC connstring cannot be correlated reliably, so the documentation
  keeps the operator rule: do not use both paths for one reader.

## Surface changes

- `libnfc.RuntimeInfo.Drivers` lists registered libnfc drivers through the C
  shim instead of a hardcoded `pn532_uart`.
- `nfctl diag` prints the compiled drivers, and `supportedReaders` is derived
  from validated capability profiles.
- `nfctl list` and `nfctl scan --json` expose driver, capability, and validation
  fields. Human output shows a compact capability line.
- The GUI device rail shows capability pills and marks experimental or
  unverified profiles. Read/write actions that the selected reader cannot
  perform are disabled, and the Type 2 editor reports the unsupported reader.
- udev rules grant desktop `uaccess` for the PN53x/ACR122 USB IDs. PC/SC access
  failures name `pcscd` and the competing-claim possibility.

## Hardware validation

Records belong in `docs/notes/06-reader-expansion.md`. Only workflows actually
run on hardware are marked validated. Missing hardware is recorded as a
limitation, not inferred from a successful build.

## Non-goals

- No SPI/I2C, legacy serial, or `pn71xx` driver in the application profile.
- No PC/SC-native APDU transport in this phase.
- No change to `libnfc-ng`'s default driver set or upstream driver sources.
- No claim that compiling a driver validates it.

## Test plan

Hardware-free:

- profile lookups for every known driver and an unknown connstring;
- unsupported operations return `CodeUnsupported` without touching a native
  call;
- compiled-driver reporting in both the stub and libnfc builds;
- ACR122 enumeration collapse and physical lock identity;
- app DTO, CLI JSON, human list, and diagnostics output;
- GUI capability pills and disabled Type 2 read actions.

Hardware:

- build the five-driver profile and run `make libnfc-ng-verify`;
- enumerate and select a card on PN532 UART and ACR122 USB;
- record the actual `acr122_pcsc` and `pcsc` results on the available ACR122U;
- run a two-process lock test across the ACR122 USB and PC/SC connstrings.

## Rollback

Capability profiles are Go data; the lock profile is one line. Reverting the
change restores the previous pn532_uart-only build and the previous
runtime-info output. No card data or native ABI changes are involved.
