# Phase 05b: GUI write workflows

## Goal

Add confirmed Type 2 page and NDEF writes to the MyGo GUI. The protocol, backup,
verification, and CLI paths already exist from Phase 05; this phase connects
them to the native UI without weakening the safety rules.

Status: done (2026-11-07). See `docs/notes/05b-gui-write.md`.

## Work

- Add asynchronous application tasks for Type 2 page writes and NDEF writes.
- Expose those tasks through `app.Bindings` and the desktop `Backend`.
- Add a confirmation dialog that states the target, payload, backup behaviour,
  and read-back verification before any write.
- Add a user-page editor to the Memory tab with table selection and hex input.
- Add a Text/URI NDEF editor to the NDEF tab.
- Show task failure details, including the backup path, in the GUI.
- Keep protected pages, configuration, PWD/PACK, and dynamic-lock writes
  CLI-only.
- Complete English and Simplified Chinese messages.
- Cover the interactions with headless GUI tests.

## Acceptance

- A user page can be written from the Memory tab after explicit confirmation.
- A text or URI NDEF message can be written from the NDEF tab after explicit
  confirmation.
- The GUI refuses invalid page hex, empty NDEF input, system pages, and writes
  while another task runs.
- The confirmation states that a full-image backup is created and the write is
  read back.
- A failed write shows the backup path; a successful write refreshes the Type 2
  snapshot.
- `make check` passes.

## Non-goals

- No GUI writes to configuration, dynamic lock, static lock, PWD/PACK, or pages
  0-3.
- No Classic GUI write flow; it needs its own confirmation model.
- No NDEF record-type editor beyond Text and URI.
- No card emulation.
