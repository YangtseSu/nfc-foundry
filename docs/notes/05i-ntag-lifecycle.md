# Phase 05i implementation note: NTAG copy and lifecycle operations

Date: 2026-11-07

## Implemented

- Added `internal/type2` lifecycle plans:
  - `PlanCopy` with same-product checks, user-memory default, optional
    PWD/PACK/configuration/capability-container copy, same-UID policy, static
    lock detection, and NDEF capacity validation;
  - `PlanErase` for unlocked user memory;
  - `PlanLock` for static lock, dynamic lock, CC read-only, and CFGLCK;
  - `PlanPasswordSet` and `PlanPasswordRemove` with AUTH0/PROT handling.
- Added `workflow.Type2Service.PreviewType2Lifecycle` and
  `ExecuteType2Lifecycle`. Previews return old/new changes and a SHA-256 token
  over the card identity and planned writes. Execution re-reads the card,
  re-plans, rejects a stale token, writes a full-image backup, verifies every
  page, reselects the card, and verifies the final image.
- Protected final reads use the projected protection state, so password
  removal does not send a stale PWD_AUTH.
- Added CLI commands:
  `type2 copy`, `type2 erase`, `type2 lock`, `type2 password set|remove`.
  Copy supports `--repeat N`, `--same-uid-only`, `--include-protected`, and
  `--continue-on-error`.
- Added Advanced-mode GUI controls with preview dialogs, old/new values,
  irreversibility flags, a target count for repeated copy, and a typed
  card-UID confirmation.
- Added English and Simplified Chinese strings for every new control and error.

## Behaviour details

- Copy never writes UID pages. Different target UIDs are the normal case;
  `--same-uid-only` rejects them.
- Default copy requires a valid, writable target capability container. The
  protected path may format a blank `00 00 00 00` container from the source or
  OR the source access bits into an existing container; bytes 0-2 are
  preserved.
- Static lock accepts the full desired 16-bit value and rejects clearing set
  bits. It is written last because it can block later writes.
- Dynamic lock accepts three bytes and is OR-only. Erase skips statically
  locked pages and pages covered by set dynamic-lock bits.
- CC read-only ORs `0x0F` into the access byte and preserves the rest.
- CFGLCK is one-way; a second attempt and any config/password write on a
  CFGLCK-locked tag is refused.
- Password set defaults to PROT enabled in the GUI and CLI. `--no-protect`
  installs PWD/PACK/AUTH0 without enabling PROT. Password removal refuses when
  CFGLCK is set.
- MIFARE Ultralight rejects CC/config/password lifecycle writes because page 3
  is OTP.

## Verification

- `go test ./...` passes.
- `make check` passes: tests, vet, i18n catalogue, headless GUI tests, and
  `reuse lint` (397/397 files).
- Workflow tests cover user-memory copy, same-UID and product mismatch,
  advanced copy of PWD/PACK/config, blank CC formatting, erase and locked-page
  skip, static/dynamic/CC/CFGLCK locks, password set/remove, stale tokens, and
  CFGLCK refusal.
- CLI tests cover copy, erase, lock, password set/remove, and repeated copy
  across two simulated tags with per-target verification.
- GUI tests cover Advanced gating, preview, typed UID confirmation, and the
  lifecycle request token.

## Hardware validation (2026-11-07)

- Reader: ACR122U, `acr122_usb:001:004`.
- Card: one NTAG215, 7-byte UID, factory configuration, 496-byte NDEF capacity.
- Actual `type2 copy` onto the same UID restored the original user memory and
  NDEF after writing two different NDEF messages; every planned page was
  written and verified, and the final 135-page image matched the pre-test
  backup.
- Actual `type2 erase` zeroed the 20 changed user pages, reported that
  lock/config/PWD/PACK remain, and left the tag with no NDEF. Restoring the
  binary backup brought the original image back exactly.
- Actual binary restore (`type2 restore FILE.bin`) wrote and verified pages
  4-23 and reproduced the original page array.
- Not tested on hardware: copy between different UIDs, repeated copy across
  tags, static/dynamic lock writes, CFGLCK, and password set/remove. Only one
  tag is available and those operations are one-way or would change its
  protection state.

## Limitations

- Copy, erase, and binary restore were hardware-tested. Lock and password
  writes are irreversible and were not tested; they require a disposable tag.
- The dynamic-lock page-to-page mapping used by erase follows the implementation's
  NTAG21x grouping (two pages per bit on NTAG213, sixteen on NTAG215/216).
  Confirm the resulting skipped-page set on a disposable tag before relying on
  partial erase.
- The GUI repeated-copy task waits for card removal and appearance through the
  device manager. Headless tests cover its preview and typed-confirmation
  contract; the full sequence is covered by the CLI switching-tag test.

## Follow-up

- Hardware-validate copy, erase, lock, and password set/remove on disposable
  NTAG213/215/216 tags.
- Phase 05h remains the only magic-clone UID/backdoor path.
