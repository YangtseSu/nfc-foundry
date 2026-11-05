# Phase 09: card emulation for mobile Linux (conditional)

## Status

Conditional. This is a valid future direction, not a global non-goal. It is not
part of the initial desktop milestones and is not exposed by the current GUI or
CLI.

## Goal

Define and validate card emulation for mobile Linux devices, then decide whether
to implement it.

## Questions to answer

- Which emulated target types are needed: Type 2, Type 4, ISO14443-4, others?
- Which PN53x/PN71xx firmware and Linux targets support target mode reliably?
- Which readers and systems are the validation targets?
- How should emulation state be isolated from initiator card workflows?
- What recovery/abort behaviour is required when emulation fails?

## Entry criteria

- Phase 05 NDEF and Phase 06 reader capability model are stable.
- A target-mode-capable reader and test hardware are available.
- The scope above is written as an accepted spec.

## Rules

- libnfc-ng retains upstream target-mode APIs; do not remove them.
- Keep target-mode and raw-frame abstractions in the transport design.
- Do not add emulation commands or UI until this phase has an accepted spec and
  hardware validation.
- Do not advertise emulation support before validation.
- Do not reuse initiator write paths for emulation state.

## Non-goals for this phase

- No emulation implementation in Phases 00-08.
- No claim that current builds expose emulation.
- No commitment to a release date.

This phase is conditional: it can remain unstarted, but it must not be documented
as "No emulation" for the product.
