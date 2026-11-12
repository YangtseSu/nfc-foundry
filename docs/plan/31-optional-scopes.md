# Phase 31 plan: optional scope register

Status: conditional 2026-11-12. No implementation is scheduled. This phase
exists so two previously discussed directions are not silently dropped.

## Card emulation

Status: conditional. Old Phase 09 explicitly treated emulation as a future
direction, not a global non-goal. The current release does not expose it and
must not advertise it.

Open questions before any implementation:

- target types: Type 2, Type 4, ISO14443-4, or a subset;
- which PN53x/PN71xx firmware and Linux targets support target mode reliably;
- validation readers and systems;
- isolation from initiator card workflows and the reader lease;
- abort/recovery behavior when emulation fails or the reader disconnects.

Entry criteria:

- a target-mode-capable reader and test hardware are available;
- the scope is written as an accepted spec;
- ADR records the target-mode transport boundary;
- no initiator write path is reused for emulation state;
- the GUI/CLI surface is defined before code.

Rules:

- do not expose emulation commands or UI before the spec and hardware record;
- do not claim emulation support in the capability matrix;
- keep target-mode APIs in the transport design without advertising them.

## Jewel / Topaz (Type 1)

Status: conditional. Old `docs/cards.md` listed Jewel/Topaz as optional; the
current Phase 14 explicitly excludes Type 1/Topaz because the supported readers
do not provide the RF path. This remains an optional scope, not a committed
release feature.

If pursued, the minimum scope is:

- card identification;
- read of the user bytes;
- guarded user-byte write with read-back verification;
- an explicit capability profile per reader;
- hardware validation on an owned tag.

Non-goals:

- formatting/cloning;
- a generic Type 1 command console;
- advertising support before a hardware record.

## Documentation

- Add a conditional entry to `docs/plan/README.md`.
- Keep both items out of the release support matrix until a phase is accepted.
- Do not remove the conditional wording in favor of a global “no emulation”
  statement.
