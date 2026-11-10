# Phase 19c plan: Type 2 / NTAG and Type 4 read

Status: complete 2026-11-11. Depends on 19b's shared NDEF draft/composer and
19a's mock/Xvfb test harness. Type 4 read is left out because no reader/card
hardware record exists.

Goal: add the second card family to the workbench. The default path is
user-memory access; configuration, locks, password, and one-way changes are
Advanced and confirmation-gated. Type 4 stays read-only and hardware-gated.

## Scope

### Type 2 memory

- Read the complete Type 2 image through `Service::type2_read`; keep partial
  reads explicit.
- Page table: page, role (UID, static lock, CC, user, dynamic lock,
  configuration, password/PACK, reserved), lock/access state, data.
- Inspector: hex/ASCII, page role, lock and configuration facts.
- Page write: user pages use the existing verified service path; Advanced
  pages (CC, lock, configuration, password/PACK) are hidden by default.
- Dump/restore Type 2 project images; expose restore include options
  (user memory, configuration, CC, locks) with the service defaults.

### Advanced mode and protection

- Advanced mode is off by default, session-scoped, and resets on card removal,
  reader change, and application restart. It is a confirmation/
  discoverability layer, not a protocol bypass.
- Lock pages: exact page list, current/new lock state, one-way warning,
  backup path, read-back plan; typed confirmation before the service call.
- Password/AUTH0/ACCESS: exact PWD/PACK/AUTH0/ACCESS values, old/new state,
  backup path, read-back plan; typed confirmation.
- Configuration and CC writes use the same Advanced gate.
- Every Advanced action names the card identity and refuses on a card change.
- Add an optional expected `CardInfo` to the Type 2 page/NDEF/format/restore/
  lock/password requests, mirroring 19a's Classic guard. The service compares
  it with the operation card before any backup or write.

### NDEF

- Reuse the 19b draft model for Text/URI/raw messages.
- Type 2 NDEF read/write/format through the existing service methods.
- Show CC/NDEF state, capacity, read-only/locked state, and partial warnings.
- Preserve unknown/proprietary TLVs unless the user explicitly formats.
- A protected or locked tag is never silently overwritten.

### Type 4 read (gated)

- Add a read-only Type 4 view only after a hardware record exists for the
  selected reader/card path.
- Do not advertise Type 4 before that record. Do not add Type 4 writes in this
  phase.
- If a read-only surface is added, add the matching `nfctl` read command at the
  same time; no GUI-only card operation.
- Without hardware, keep the Type 4 service and tests off the UI.

## Order

1. Type 2 image/page table and inspector.
2. User-page write and dump/restore.
3. Advanced mode, lock, configuration/CC, and password gates.
4. NDEF tab using the 19b draft.
5. Mock/Xvfb tests and hardware gate.
6. Type 4 read only if the hardware record is available; otherwise leave it
   out and record the blocker.

## Acceptance

- NTAG213/215/216 mock images render correct page roles, CC, locks,
  configuration, and NDEF state.
- A user-page write is backed up and read back; a locked/read-only page is
  refused.
- Advanced pages require Advanced mode plus the typed confirmation and show
  the exact pages/values/backup.
- Type 2 NDEF write/format preserves unknown TLVs and refuses locked/read-only
  tags.
- Card change aborts before mutation.
- Hardware tests are mock-only unless a write is explicitly authorized; no
  driver/card support is advertised from mock results.
- `cargo fmt`, workspace clippy with `-D warnings`, workspace tests, the gated
  GTK scenario, and `reuse lint` pass.

## Non-goals

- Type 4 writes, Amiibo-specific workflows, UID clone, factory format.
- Changing the Type 2 service safety rules.
- `zh-CN`, packaging, Windows/macOS.

## First files

- `crates/gui/src/app.rs` — Type 2 pages, Advanced state, family-aware NDEF tab.
- `crates/gui/src/type2.rs` — page/role/lock/NDEF presentation helpers.
- `crates/nfc-app/src/type2/mod.rs` — service entry points and result types.
- `crates/nfc-type2` — page/CC/lock/config model.
- `docs/design/workbench.md` — Type 2 screen contract.

## Result

The Type 2 service read carries card identity, all Type 2 mutation requests take
an optional expected `CardInfo`, and `Service::read_card` dispatches Classic or
Type 2 from one selection. Capability-container writes validate the resulting
product and refresh the parsed CC after verification.

The GTK workbench adds the Type 2 Memory/Protection tab and makes the NDEF tab
family-aware. Advanced mode, typed confirmations, lock/password forms,
dump/restore, and the NDEF write/format gates are implemented. The gated GTK
scenario covers the new surfaces against a scripted NTAG213.

Type 4 read remains out of the UI and has no `nfctl` command because no
reader/card hardware record exists. The Type 4 service and tests stay
hardware-free. See `docs/notes/19c-type2-type4.md`.
