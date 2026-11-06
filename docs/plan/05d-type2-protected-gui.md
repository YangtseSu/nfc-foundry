# Phase 05d: Advanced Type 2 protected writes

## Goal

Add GUI writes for Type 2 configuration, dynamic lock, PWD/PACK, and static lock
behind the shared **Advanced operations** mode. Every operation keeps the
existing backup, read-back, card-change, and confirmation safety rules.

This phase does not write UID pages or the capability container.

## Repository constraints

- Type 2 pages 0-1 contain the UID and are read-only on genuine NTAG.
- Page 3 is the capability container/OTP area and is not a normal write target.
- Static and dynamic lock bits are one-way. A failed or incorrect lock write can
  permanently reduce or destroy usable memory.
- Configuration and PWD/PACK writes can lock the tag or make it unreadable.
- A failed configuration write can brick a card.
- The existing rule remains: no generic "write everything writable" operation
  and no UID-page writes through a normal Type 2 command.

If UID/0-3 writes are a product requirement, they need a separate validated
magic-tag protocol phase and an explicit update to `AGENTS.md`. This phase does
not add that protocol.

## Advanced operations mode

Phase 05c introduces the session-scoped toggle. Phase 05d reuses it:

- off by default;
- reveals the Protection controls only after an explicit warning;
- resets on card removal, device disconnect, and application restart;
- every protected action still has its own confirmation dialog;
- the confirmation dialog requires typing the exact word `PROTECTED` for
  configuration, lock, and PWD/PACK changes;
- no "do not ask again" option.

## Work

- Expose protected Type 2 writes through `app.Bindings` and the desktop
  `Backend`.
- Add a semantic Protection editor:
  - AUTH0 page and ACCESS flags (PROT, CFGLCK, AUTHLIM);
  - PWD and PACK values;
  - dynamic lock bits;
  - static lock bits.
- Show a read-only page 0-3 summary with an explicit "UID/CC is read-only"
  state.
- Validate one-way lock bits: pending bits may only be set, never cleared.
- Perform static-lock writes as a partial page update: read page 2, replace
  bytes 2-3, preserve BCC1 and the internal byte, write, and verify.
- Perform dynamic-lock writes as a partial update of lock bytes while preserving
  RFUI bytes.
- Perform PWD/PACK writes as byte-level updates that preserve RFUI bytes.
- Write configuration pages after ordinary user pages and PWD/PACK, and dynamic
  lock last, following the existing protected restore order.
- Save a full-image backup before the first protected write and abort if the
  backup fails.
- Verify every changed page immediately and verify the final image after
  reselection.
- Add English and Simplified Chinese messages and warnings.
- Add a hardware test gated by
  `NFCX_HARDWARE_TYPE2_PROTECTED_CONFIRM=I_OWN_THIS_DISPOSABLE_TAG`. It must not
  run in ordinary CI and must be documented as destructive.

## Acceptance

- Advanced mode is off by default and resets on card/device change.
- Configuration, dynamic lock, PWD/PACK, and static lock controls are hidden
  without advanced mode.
- Every protected write requires advanced mode, an explicit confirmation, the
  `PROTECTED` phrase, and a full-image backup.
- Lock writes refuse to clear one-way bits.
- Static lock writes preserve BCC1 and the internal byte.
- Pages 0-1 and page 3 are shown read-only and cannot be written.
- Every changed page is read back; the final image is verified.
- Tests cover confirmation refusal, lock-bit validation, static-lock partial
  write, backup failure, read-back mismatch, and card change.
- `make check` passes.

## Non-goals

- No UID/page 0-1 write.
- No capability-container/page 3 write.
- No arbitrary page 0-3 editor.
- No magic-tag UID backdoor.
- No unprotected write path that skips backup or confirmation.
