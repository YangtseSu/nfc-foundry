# Phase 05e implementation note: NDEF record catalogue

Date: 2026-11-07

## Implemented

- Extended `type2.Record` with `Kind`, `Summary`, and decoded `Fields`.
- Added decoders for:
  - Text and URI, with URI categories for URL, email, phone, SMS, and
    location, plus decoded mailto/tel/sms/geo fields;
  - Smart Poster nested URI/title;
  - MIME `text/plain`, `text/vcard`, `text/x-vcard`, `application/json`;
  - Wi-Fi WSC (`application/vnd.wfa.wsc`);
  - Bluetooth BR/EDR easy pairing (`application/vnd.bluetooth.ep.oob`);
  - External `android.com:pkg` (Android Application Record);
  - unknown MIME/external records with a printable fallback.
- Added encoders for Smart Poster, vCard 3.0, Wi-Fi WSC, Bluetooth OOB, external
  records, and MIME records.
- Added the common template catalogue:
  - Text, URL/URI, custom URI, search, payment link, social profile,
    application, email, contact, phone, SMS, location, address, Bluetooth,
    Wi-Fi, Smart Poster.
- Search uses a user-editable provider URL prefix (default DuckDuckGo), not a
  hard-coded search engine.
- Wi-Fi security supports Open, WPA-Personal, WPA2-Personal, and WPA/WPA2 mixed.
- Wi-Fi network keys are masked in read views; the editor still accepts the
  password for writing.
- The GUI NDEF editor now uses a template selector with dynamic fields.
- The GUI decoded view shows record kind, summary, and labeled fields,
  including contact, Wi-Fi, Bluetooth, email, phone, SMS, and location data. Raw
  payload/message hex is hidden behind a Show raw toggle.
- The CLI `ndef read` prints decoded summaries and fields; `--raw` still prints
  only the raw message hex.
- Added more than 60 English and Simplified Chinese labels for templates,
  fields, kinds, and security choices.

## Verification

- `go test ./...` passes.
- `make check` passes: tests, vet, i18n catalogue, headless GUI tests, and
  `reuse lint` (378/378 files).
- `CGO_ENABLED=1 go test -tags libnfc ./...` passes.
- Unit tests cover Smart Poster, vCard, Wi-Fi, Bluetooth, AAR, every template,
  required-field failures, URI query escaping, and folded vCard lines.
- GUI tests cover template field rendering, Wi-Fi template submission, decoded
  vCard/Wi-Fi cards, and raw-data opt-in.
- CLI tests cover vCard record decoding after a write.
- Hardware: `nfctl ndef read` on the ACR122U/NTAG215 now prints the existing
  vCard as:
  `summary: 苏长江 / record 1: vcard · 苏长江 / name: 苏长江 / phone: ...`
  instead of a `42 45 49 ...` hex dump.

## Limitations

- No chunked NDEF, signature, arbitrary MIME editor, or Connection Handover
  negotiation.
- Wi-Fi WSC is the validated 2.0 credential subset; no WPA3/WSC 3.0 claim.
- Bluetooth support is the BR/EDR easy-pairing OOB record, not negotiated
  handover or LE OOB.
- Raw payload remains available only through Show raw or CLI `--raw`.

## Follow-up

- Hardware validation of a written Wi-Fi and Bluetooth record on a disposable
  tag.
- Optional per-record raw reveal and Wi-Fi network-key reveal.
- Phase 05d protected writes remain pending.
