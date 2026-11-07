# Phase 05h: Magic Type 2 UID and backdoor operations

## Goal

Support UID/page 0-1 changes on explicitly identified magic Type 2 clones,
without pretending that genuine NTAG pages are writable.

## Prerequisites

- Detect the clone family from memory behavior and, where possible, a validated
  backdoor command sequence.
- Document the exact clone families and commands; never guess a backdoor.
- Test only on disposable tags owned by the operator.
- A failed attempt may leave the tag unselectable; require a backup and an
  explicit "I understand this may destroy the tag" confirmation.

## Work

- Add a magic-tag probe that is separate from the normal NTAG path.
- Implement only validated backdoor sequences for identified clone families.
- Back up the complete image before any UID/block-0 attempt.
- Require Advanced mode, a typed confirmation, and an exact target UID.
- Reselect and verify UID/BCC0/BCC1/full block 0 after the attempt.
- Keep genuine NTAG UID pages read-only and report the hardware refusal
  honestly.
- Add fixtures and hardware tests gated by a disposable-tag environment.

## Acceptance

- Genuine NTAG never claims a UID write succeeded.
- A supported magic clone can be identified, written, reselected, and verified.
- Unsupported clones fail safely before a destructive command.
- Every attempt has a full backup and a typed confirmation.
- `make check` passes; hardware tests are explicit.

## Non-goals

- No universal "UID writer" that blindly sends backdoor frames.
- No support claims for untested clone families.
- No normal Type 2 UID write path.
