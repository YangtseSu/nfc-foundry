# Spec: phase 19b NDEF write and format

Status: complete 2026-11-11. Note:
[19b-ndef-write.md](../notes/19b-ndef-write.md). Plan:
[19b-ndef-write.md](../plan/19b-ndef-write.md).

## Surface

- The existing NDEF tab keeps the read-only mapping/record view.
- Composer sources: Text (text + language), URI, or a raw NDEF file.
- Exactly one source is active. The encoded `NdefMessage` and its byte size are
  shown before writing.
- `Write NDEF` and `Format NDEF area` are the only mutation actions.

## Safety

- A write or format requires a live card read on the selected reader; a loaded
  dump is inspection-only.
- The loaded dump's `CardInfo` is passed as `expected_card`; a mismatch is
  refused before backup or card I/O.
- The message must fit `ClassicNdefInfo::ndef.capacity`; the service remains
  authoritative.
- A read-only NDEF area cannot be written.
- Format opens a destructive confirmation that names the NDEF area, the
  read-only option, the backup, and the read-back plan.
- The service performs the pre-write backup and read-back; the GUI shows the
  backup path.
- Cancel leaves the card unchanged.

## Service calls

- `Service::classic_ndef_write(ClassicNdefWrite::new(...)
  .expected_card(...))`.
- `Service::classic_ndef_format(ClassicNdefFormat::new(...)
  .confirm(true)
  .read_only(selected)
  .expected_card(...))`.
- Both run on a worker thread.

## Refresh

- On write success, use `ClassicNdefWriteResult.dump` and `.info`.
- On format success, use `ClassicNdefFormatResult.dump` and `.info`.
- Refresh the block table, NDEF page, card summary, selection state, and log.
  Do not re-read the card just to refresh.

## Errors

- Unformatted: show Format as the prerequisite, not a write failure.
- Too large: show required and available bytes before the dialog.
- Malformed raw file: report the decode error and keep the composed source.
- Read-only, partial image, no card, identity mismatch, busy: disable the
  action or refuse with a specific message.
- Service failure: keep the previous image; show the error once in the toast
  and log.

## Tests

- Draft encoding and capacity unit tests.
- Mock Classic NDEF card UI scenario: Text write, URI write, raw write,
  format, cancel, read-only, unformatted, too large, identity mismatch.
- Gated by `NFC_FOUNDRY_GUI_TESTS=1`; run with Xvfb in CI.
- Normal workspace tests stay hardware-free and headless-safe.
