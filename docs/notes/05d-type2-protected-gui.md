# Phase 05d implementation note: Advanced Type 2 protected writes

Date: 2026-11-07

## Implemented

- Added `type2.PlanProtectionWrites` for per-field protection edits:
  AUTH0, ACCESS, PWD, PACK, dynamic lock, and static lock.
- Write order is PWD/PACK, configuration, dynamic lock, and static lock last.
- AUTH0 accepts any byte value; `FF` disables authentication.
- ACCESS changes only PROT, CFGLCK, the NFC counter flags, and AUTHLIM. Bit 3
  and every reserved bit stay unchanged. CFGLCK cannot be cleared.
- PACK writes preserve the two RFUI bytes. Dynamic-lock writes preserve byte 3.
  Static-lock writes preserve BCC1 and the internal byte.
- Lock writes refuse to clear set bits and continue to OR pending bits.
- Configuration and password writes are refused once CFGLCK is set.
- Added the `protect` workflow operation. It reuses the lifecycle preview,
  full-image backup, immediate page read-back, reselection, final-image
  verification, and projected password for the final read.
- `Type2PlanToken` now hashes the complete current image in addition to the
  card identity and planned writes, so any image change invalidates a preview.
- Added `Type2ProtectionWriteRequestDTO`, `Type2ProtectionPreflightDTO`,
  `PreflightType2Protection`, and `StartType2ProtectionWrite` to the service
  and bindings.
- Protection preview tokens expire after five minutes. The start method
  requires the exact confirmation word `PROTECTED`.
- Replaced the earlier lock/password GUI panels with one semantic protection
  editor, visible only in Advanced mode. The dialog lists every old/new value,
  warns about one-way locks and bricking, and requires `PROTECTED`.
- Removed the superseded GUI strings; the CLI still exposes the 05i lifecycle
  lock and password commands.
- Added English and Simplified Chinese strings for the editor, warnings, and
  errors.

## Scope boundaries

- Pages 0-1 and page 3 remain read-only here. The capability-container
  access-byte lock belongs to Phase 05i; magic-clone UID writes belong to
  Phase 05h.
- A changed field equal to the current value produces no page write.
- A preview with no effective change is rejected before a token is issued.

## Deviations from the spec

- The editor accepts ACCESS as one hexadecimal byte and masks it to the
  writable PROT, CFGLCK, counter, and AUTHLIM bits. It does not use separate
  flag checkboxes. The preview still shows the exact old/new byte.
- Password changes are shown as `current → new 4-byte value` rather than
  echoing either secret in the confirmation dialog.
- Page 3 is excluded from this phase, matching the spec and 05i ownership.

## Problems resolved

- The 05i lifecycle token bound only the card and planned writes, so a changed
  current image could still match a preview when the resulting bytes were the
  same. The token now hashes the full current image as well.
- The GUI already had separate lock and password panels from 05i. They wrote
  through the lifecycle path without the protection-specific preflight or
  `PROTECTED` phrase, so they were replaced by one protection editor. The CLI
  lifecycle commands remain unchanged.

## Verification

- `go test ./internal/type2 ./internal/workflow ./app ./internal/desktop`
  passes.
- Planner tests cover write order, reserved-bit and RFUI preservation, BCC1
  preservation on static-lock writes, one-way lock refusal, CFGLCK refusal, and
  AUTH0 range validation.
- Workflow tests cover the protection plan and image-change token
  invalidation.
- App tests cover preflight, exact `PROTECTED` enforcement, token expiry, and
  an end-to-end AUTH0/ACCESS/static-lock/PACK write against the mock tag.
- GUI tests cover Advanced-mode gating, the old/new preview, and refusal to
  start without `PROTECTED`.

## Hardware validation

Not run. Configuration and lock writes are one-way or potentially
card-bricking; they require a disposable NTAG213/215/216. The gated test
`TestHardwareType2ProtectedWrite` is available through
`make libnfc-type2-protected-hardware-test` with
`NFCX_HARDWARE_TYPE2_PROTECTED_CONFIRM=I_OWN_THIS_DISPOSABLE_TAG` and at least
one `NFCX_HARDWARE_TYPE2_PROTECT_*` field selector. It leaves the tag in the
tested state and keeps the full-image backup.
