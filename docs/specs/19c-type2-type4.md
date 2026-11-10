# Spec: phase 19c Type 2 / NTAG and Type 4 read

Status: pending. Plan: [19c-type2-type4.md](../plan/19c-type2-type4.md).

## Surface

- Type 2 memory page: page table with role, lock state, and data; inspector
  with hex/ASCII and page facts.
- Type 2 NDEF page: shared composer from 19b, NDEF state/capacity, write and
  format.
- Type 2 protection: static/dynamic locks, configuration/CC, password/AUTH0/
  ACCESS; Advanced-only.
- Type 4: read-only, hardware-gated; no surface before a hardware record.

## Safety

- Default to user-memory access only. Advanced mode is off by default,
  session-scoped, and resets on card removal, reader change, and restart.
- Every Advanced action shows the exact pages, old/new values, backup path,
  and read-back plan; destructive actions require typed confirmation.
- One-way locks, configuration, CC, and password/PACK are never silently
  written.
- A locked or read-only tag is refused, not overwritten.
- Partial reads remain partial; unknown pages are not zero-filled.
- The service owns backup, read-back verification, and card identity checks.
  Type 2 mutation requests carry an optional expected `CardInfo`; a mismatch is
  refused before backup or card I/O.
- Type 4 read-only surfaces require a recorded hardware test; no Type 4 write
  is exposed in this phase.

## NDEF

- Reuse the 19b draft/encoding path; do not add a second NDEF codec.
- Show CC capacity and NDEF state before writing.
- Preserve unknown/proprietary TLVs unless the user explicitly formats.
- Format is destructive and confirmed.

## Tests

- Mock NTAG213/215/216 images for page roles, CC, locks, configuration, NDEF
  state, and partial reads.
- UI tests: page selection, user-page write, locked/read-only refusal,
  Advanced gate/typed confirmation, NDEF write/format, identity change.
- Hardware records only when explicitly authorized; no support claims from
  mock tests.
