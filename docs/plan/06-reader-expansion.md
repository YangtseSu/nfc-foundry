# Phase 06: reader expansion

## Goal

Add validated readers and a capability model that does not overstate support.

## Work

- Enable `pn53x_usb`, `acr122_usb`, `acr122_pcsc`, and `pcsc` in
  `libnfc-ng` one at a time.
- Add dependencies, udev rules, and `pcscd` handling.
- Add a capability matrix per driver:
  enumerate, select, Classic read/write, Type 2, raw bytes, raw bits, PC/SC
  APDU, recovery engine support.
- Add hardware validation records for each reader.
- Decide whether generic PC/SC readers need a separate transport.

## PC/SC scope decision

PC/SC is a smart-card API: contact ISO/IEC 7816, CCID readers, PIV/bank cards,
and USB tokens such as YubiKey. libnfc's `pcsc` driver is only a bridge that
lets libnfc select an NFC target through such a reader and exchange APDUs. It
does not provide the raw frame, parity, timing, Crypto1, or bit-level control
that the PN53x drivers expose.

Therefore:

- Generic `pcsc` remains an APDU/selection path. Classic, Type 2, Gen1A, raw
  frames, and recovery engines are not implemented over it and are not planned.
- A future PC/SC-native APDU transport is optional and would cover ISO-DEP
  APDUs, Type 4 NDEF, and DESFire reads, not Classic or Type 2.
- `acr122_pcsc` is a separate path: it wraps PN53x commands in PC/SC escapes and
  could be raw-capable if it opens reliably. On Linux the direct `acr122_usb`
  path is the supported one; fixing the PC/SC open timeout is lower priority.

## Acceptance

- Each enabled reader has a recorded hardware test.
- The GUI/CLI show unsupported operations as unavailable, not failed.
- PC/SC and direct USB do not claim the same reader at once.
- Packaging dependencies match the enabled drivers.

## Non-goals

- No SPI/I2C or legacy serial drivers on desktop by default.
- No claim that "libnfc listed it" means full support.
