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

## Acceptance

- Each enabled reader has a recorded hardware test.
- The GUI/CLI show unsupported operations as unavailable, not failed.
- PC/SC and direct USB do not claim the same reader at once.
- Packaging dependencies match the enabled drivers.

## Non-goals

- No SPI/I2C or legacy serial drivers on desktop by default.
- No claim that "libnfc listed it" means full support.
