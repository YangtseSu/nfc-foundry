# Phase 05i: NTAG copy and lifecycle operations

Status: done (2026-11-07). See `docs/notes/05i-ntag-lifecycle.md`.

## Goal

Add useful NTAG lifecycle workflows to the GUI and CLI: content copy, repeated
copy, erase, lock, set password, and remove password.

The operations are Advanced-mode features. They keep the existing Type 2
backup, read-back, card-change, and confirmation rules.

## Capability matrix

| Operation | Genuine NTAG21x | Notes |
|---|---|---|
| Copy user memory / NDEF | Yes | Target has its own UID |
| Copy CC/config/PWD/PACK | Conditional | Only writable, unlocked areas; CC is OR-only |
| Identical UID clone | No | UID pages are manufacturer read-only |
| Identical UID on magic clone | Phase 05h | Requires a validated backdoor |
| Repeated content copy | Yes | Sequential target writes with per-target verification |
| Erase user memory / NDEF | Yes | Does not erase lock/config bits |
| Delete/kill tag | No standard command | Use erase or irreversible lock instead |
| Lock pages / CC / config | Yes | One-way; cannot be undone |
| Set password | Yes | Writes PWD/PACK and AUTH0/ACCESS; current password required if protected |
| Remove password | Conditional | Requires writable config; CFGLCK-locked tags cannot be reset |

## Work

- Add a copy workflow:
  - default copy: user memory and NDEF only;
  - advanced copy: CC, config, PWD/PACK when the target and source state allow
    it;
  - never copy UID pages to genuine NTAG;
  - verify product, UID mismatch policy, target writability, and every page.
- Add a repeated-copy workflow that applies one dump to a sequence of tags.
  Stop on the first mismatch unless an explicit continue policy is chosen.
- Add an erase workflow for user memory/NDEF. Do not claim it wipes lock or
  configuration bits.
- Add a lock workflow with explicit targets:
  - static lock bits (partial page 2 write);
  - dynamic lock bits;
  - CC read-only bits (OR-only);
  - configuration lock (CFGLCK);
  - password protection (AUTH0/PROT).
- Add password set/remove:
  - set PWD/PACK, AUTH0, and PROT;
  - remove by restoring PWD `FF FF FF FF`, PACK `00 00`, AUTH0 `FF`, and
    disabling PROT where writable;
  - require the current password or an explicit failure.
- Every destructive action shows old/new values, backup path, irreversibility,
  and requires a typed confirmation.
- Add English and Simplified Chinese messages.

## Acceptance

- Content copy works between same-product genuine NTAG tags with different UIDs.
- The UI never claims an identical-UID copy on genuine NTAG.
- Repeated copy writes and verifies each target independently.
- Erase clears NDEF/user memory and reports that lock/config bits remain.
- Lock and password operations are Advanced-only, backed up, read back, and
  refused when the required current password is missing.
- Genuine NTAG UID writes fail honestly.
- `make check` passes; destructive hardware tests are explicit.

## Non-goals

- No genuine NTAG UID clone.
- No universal kill/delete command.
- No bypass of an unknown password or CFGLCK.
- No magic-clone backdoor; that is Phase 05h.
