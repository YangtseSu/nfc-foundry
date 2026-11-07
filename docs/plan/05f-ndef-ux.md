# Phase 05f: NDEF UX and format completion

Status: done (2026-11-07). See `docs/notes/05f-ndef-ux.md`.

## Goal

Finish the useful NDEF workflows that were previously deferred as "not in the
current UI".

## Work

- GUI password input for PWD-protected Type 2 reads and writes.
- Explicit reveal control for masked Wi-Fi network keys and other sensitive
  NDEF fields.
- Raw NDEF editor with validation and a multi-record builder.
- NDEF file import/export in the GUI.
- `nfctl ndef write --template` with the same template catalogue as the GUI.
- Text language selector instead of a fixed `en` tag.
- Search provider presets plus the editable URL prefix.
- Social and payment provider presets on top of the generic URL form.
- Bluetooth LE OOB record template in addition to BR/EDR easy pairing.
- Smart Poster action, resource type, and resource size metadata.
- Binary Type 2 full-image dump/restore in addition to JSON.
- Optional per-record raw reveal and decoded-field copy actions.

## Acceptance

- Protected Type 2 tags can be read and written from the GUI.
- Sensitive fields are masked by default and reveal only after an explicit
  action.
- Raw, multi-record, and imported NDEF messages pass the same validation and
  read-back rules as templates.
- CLI and GUI produce identical messages for the same template.
- `make check` passes.

## Non-goals

- No chunked-record writer or NDEF signature generation.
- No WSC Password Token, because it requires an enrollee key that a static tag
  cannot provide.
- No full DPP protocol; DPP URI writing/reading remains the Phase 05e behavior.
