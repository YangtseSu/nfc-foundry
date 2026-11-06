# Phase 05b implementation note: GUI write workflows

Date: 2026-11-07

## Implemented

- Added asynchronous application tasks:
  - `StartType2Write` for one or more user pages;
  - `StartType2NDEFWrite` for Text and URI messages.
  Both use the existing global operation slot, emit progress with written/
  verified counts and the backup path, and publish the refreshed Type 2
  snapshot on success.
- Added `type2.EncodeTextMessage` and `type2.EncodeURIMessage`. The encoder
  uses the short or long NDEF record layout, applies the longest matching URI
  prefix, and validates the generated message with the existing parser.
- Exposed the write tasks through `app.Bindings` and the desktop `Backend`.
- Added a selectable Type 2 Memory table. Selecting a user page fills an
  8-hex-digit editor; system, lock, capability, configuration, password, and
  PACK pages cannot be edited. Invalid hex is rejected before any dialog.
- Added a Text/URI NDEF editor with mode buttons, placeholder text, Enter
  submission, and an empty-input guard.
- Added `ui.AlertDialog` confirmation for every GUI write. It names the page
  and new value, or the NDEF mode and byte length, and states that a full-image
  backup is written first and the result is read back.
- Added task failure handling: a `type2_write` or `type2_ndef` failure sets the
  error banner from the task message, including the backup path. A completed
  write clears the banner and shows a completion toast.
- Completed English and Simplified Chinese messages for the new controls.

## Verification

- `go test ./...` passes.
- `go test -count=1 ./internal/desktop` passes, including:
  - page confirmation submits the expected page/hex request;
  - NDEF Text and URI confirmations submit the expected request;
  - Cancel starts nothing;
  - invalid page hex and system-page editing are rejected;
  - table selection updates the editor;
  - a failed task exposes the backup path in the error banner.
- `go test ./app` passes, including asynchronous page and NDEF tasks against a
  mock NTAG213, snapshot refresh, and failure with a backup path.
- `make check` passes: tests, vet, i18n catalogue, headless GUI tests, and
  `reuse lint`.

## Hardware status

The GUI adds no transport, command, backup, or verification code. The write
path it calls was hardware-tested in Phase 05 on an ACR122U with an NTAG215:
NDEF write verified pages 4-23, and the original user memory was restored and
checked. This phase did not run a physical GUI session; the native view tests
exercise the interaction layer headlessly.

## Non-goals and follow-up

- GUI writes remain limited to user-memory pages and Text/URI NDEF messages.
- Configuration, dynamic lock, PWD/PACK, static lock, and pages 0-3 remain
  CLI-only behind the protected-restore confirmations.
- Classic GUI writes still need their own confirmation and editor design.
- NDEF file import/export remains a CLI feature.
