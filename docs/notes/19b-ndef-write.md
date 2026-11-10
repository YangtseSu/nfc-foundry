# Note: phase 19b NDEF write and format

Plan: [19b-ndef-write.md](../plan/19b-ndef-write.md). Spec:
[19b-ndef-write.md](../specs/19b-ndef-write.md).

## Implemented

### Draft model

- `crates/gui/src/classic.rs` adds `NdefSource`, `RawNdef`, and `NdefDraft`.
  Text and URI sources encode through `nfc_ndef::{TextRecord, UriRecord}` on
  demand; a raw file is decoded once through `NdefMessage::decode`.
- Exactly one source is active. `NdefDraft::message` returns the codec error
  for an empty URI, an invalid language tag, or a missing raw file.
- `load_raw`/`load_raw_bytes` replace the raw message only after a successful
  decode, so a malformed file keeps the previous composed source.
- `ndef_capacity_error` and `hex_preview` keep the capacity refusal and the
  48-byte preview deterministic and testable.

### Composer

- The NDEF page keeps the mapping, capacity, record, and warning view and adds
  a composer card: source selector, Text language and body, URI entry, raw-file
  chooser, encoded preview, and the Write/Format actions.
- The preview shows the source summary, `message / capacity` bytes, and the
  first 48 encoded bytes.
- `Write NDEF` is enabled only when all of these hold: a live operation, no
  busy operation, a complete image, `Empty` or `Message` NDEF state, a
  non-empty writable NFC sector map, an encodable message that fits
  `ClassicNdefInfo::ndef.capacity`, and a message different from the stored
  TLV.
- `Format NDEF area…` is enabled for a live, complete image, including a card
  with no MAD. The read-only checkbox maps to `ClassicNdefFormat::read_only`.
- Unformatted, read-only, partial, loaded-dump, capacity, and encoding states
  produce one specific feedback line. The service keeps its own gates.

### Write and format

- `confirm_ndef_write` names the source, new size, new bytes, current NDEF
  state, backup, and read-back.
- `confirm_ndef_format` uses the destructive dialog and names the NFC sectors
  or the whole MAD, the read-only choice, the backup, and the read-back.
- Both operations run on a worker thread with `async-channel`, then finish on
  the GTK main context.
- Both pass the loaded image's `CardInfo` as `expected_card`; a swap is refused
  before the backup or card I/O.
- On success, the block table, card summary, NDEF facts, composer state, and
  log refresh from `ClassicNdefWriteResult`/`ClassicNdefFormatResult` without a
  second card read. On failure the previous image stays visible and the error
  is reported once in the toast and log.
- The service owns the backup and read-back; the GUI reports the backup path.

## Verification

- `cargo fmt --all --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test --workspace`: passes; the GTK scenario takes its skip branch
  without the gate.
- `env -u WAYLAND_DISPLAY NFC_FOUNDRY_GUI_TESTS=1 xvfb-run -a env
  GDK_BACKEND=x11 cargo test -p nfc-foundry -- --nocapture
  gtk_workbench_scenarios`: passes.
- `reuse lint`: clean.

The gated scenario adds a formatted Classic 1K mock image (MAD1, NFC sectors 1
and 2, empty mandatory TLV) and uses the base image as the unformatted card.
It covers Text/URI/raw encoding, malformed raw retention, capacity refusal,
invalid language, unformatted state, read-only refusal, partial image, loaded
dump, write confirm/cancel/success, format confirm/cancel/read-only success,
identity mismatch, and post-write refresh from the service result.

## Limits

- The raw NDEF file dialog is native; the test drives `load_ndef_raw` with a
  temporary file instead of opening the portal.
- No hardware was touched. Classic NDEF writes and formats were exercised only
  against the scripted transport.
- A multi-record composer and the remaining RTD templates stay out of 19b.
