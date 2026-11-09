# ADR 0002: Reader scope is PN532 UART and ACR122U USB

- Status: accepted
- Date: 2026-11-09

## Context

The available hardware is a PN532 UART adapter and an ACR122U. Supporting
every libnfc driver would slow the Rust rewrite and add unvalidated claims.
ACR122U PC/SC and direct USB differ in raw command capability.

## Decision

First-release targets:

- `pn532_uart`
- `acr122_usb` direct USB/CCID

Out of scope: PN53x USB, PC/SC-only readers, ACR122U PC/SC alias, Proxmark3
device control.

## Consequences

- Drivers can be designed around two concrete protocols.
- ACR122U raw bits/parity must be implemented over CCID escape, not PC/SC
  pseudo-APDUs.
- The capability matrix is small and must be hardware-validated before
  advertising.
- Adding another reader requires a new ADR and driver crate.
