# Spec: phase 19c Type 2 / NTAG and Type 4 read

Status: complete 2026-11-11. Plan:
[19c-type2-type4.md](../plan/19c-type2-type4.md). Note:
[19c-type2-type4.md](../notes/19c-type2-type4.md).

Type 4 remains out of the UI: no reader/card hardware record exists, and the
phase adds no Type 4 command or surface.

## Surface

- `Type 2` workbench tab with `Memory` and `Protection`.
- Memory: identity/summary, partial-read warnings, page table (page, role,
  lock/access, data), page inspector (facts, hex/ASCII, write gate), user and
  Advanced page writes, project dump save/load, and restore include options.
- Protection: parsed configuration, one-way lock form with byte preview, and
  AUTH0/ACCESS/PWD/PACK form.
- `NDEF`: family-aware. A Type 2 session shows CC size/access, capacity, NDEF
  state, locked pages, password protection, the shared 19b composer, write, and
  format.
- Type 4: no surface before a hardware record; no GUI-only command.

## Safety

- Default to user-memory access only. CC, lock, configuration, and password/PACK
  pages are hidden until Advanced mode is on.
- Advanced mode is session-scoped and resets on card read, reader change, and
  loaded dump.
- Every Type 2 mutation request carries an optional expected `CardInfo`. The
  service compares it with the selected card after cancellation/confirmation
  and before the read, backup, or card I/O; a mismatch is `CardChanged`.
- Advanced page writes, lock, password, restore, and NDEF format require the
  typed `I_OWN_THIS_CARD` phrase. The response handler rechecks the phrase.
- The service owns backups, read-back verification, and card identity. The GUI
  reports the backup path.
- A locked data page blocks NDEF write and format before the backup. A
  read-only capability container blocks both. Password-protected data pages and
  lock pages are denied because the product has no PWD_AUTH workflow for
  mutations.
- Partial reads remain partial; unknown pages are never zero-filled.
- A capability-container write must keep the product and data-area size; the
  parsed CC is refreshed after a verified write.
- Format is destructive and confirmed; it removes proprietary TLVs, matching
  the service.

## NDEF

- Reuse the 19b draft/encoding path; there is no second NDEF codec.
- Show CC capacity, data-area size, NDEF state, locked pages, and PWD protection
  before writing.
- Unknown/proprietary TLVs are preserved by NDEF write and removed only by an
  explicit format.
- A protected, read-only, locked, partial, or too-large message cannot be
  written.

## Tests

- Unit tests in `crates/gui/src/type2.rs`: row visibility/roles, lock labels,
  page and hex parsing, lock preview, NDEF area/lock/password gates, and loaded
  images.
- Service tests: family read dispatch, result identity, identity mismatch
  before backup, and CC-write validation.
- Gated GTK scenario: Type 2 read, Advanced filtering, user/Advanced page
  writes, locked refusal, CC write refresh, lock and password flows, NDEF
  write/format and all gates, partial image, dump/load, restore includes and
  identity refusal.
- Hardware records only with explicit authorization; none were added.
