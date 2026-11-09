# ADR 0004: NDEF is a first-class domain

- Status: accepted
- Date: 2026-11-09

## Context

The product has two major business domains: MIFARE Classic operations and
NDEF. NDEF spans Type 2/NTAG and MIFARE Classic, with Type 4 later. Putting NDEF
inside Classic or Type 2 page code would make it untestable and hard to extend.

## Decision

- `nfc-ndef` is a hardware-free NDEF/TLV/message/record core.
- `nfc-type2` and `nfc-classic-ndef` are family adapters.
- `nfc-type4` is a later adapter over the same core.
- `nfc-app` composes read/write/format workflows.
- CLI and GUI expose both domains with the same safety rules.

## Consequences

- NDEF codecs can be property-tested without a reader.
- Type 2 and Classic share templates and validation but not page/block models.
- NDEF protection and lock/config actions remain family-specific and Advanced.
- The GUI needs separate Type 2 memory, Classic NDEF, and NDEF editor views.
