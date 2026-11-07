# Phase 05f implementation note: NDEF UX and format completion

Date: 2026-11-07

## Implemented

- Added a masked PWD_AUTH password field to the Type 2 memory view. The same
  value is used for reads, page writes, and NDEF writes and is cleared on card,
  device, or Advanced-mode reset.
- Restricted sensitive decoded fields: `type2.Field.Sensitive` marks Wi-Fi
  network keys. The GUI masks them until “Reveal secrets” or the per-field
  reveal action; CLI human output masks them unless `--reveal` is passed.
  Masking is a display guard, not a confidentiality boundary: the raw payload
  remains available through the explicit raw toggle or `--raw`.
- Added NDEF editor modes:
  - Template, using the shared `type2.Templates()` catalogue;
  - Raw message hex, validated through `type2.ParseMessage`;
  - Multi-record builder with add, remove, and reorder for Text, URI, and Smart
    Poster records.
- Added NDEF file import and export in the GUI. Import validates the message
  before loading the raw editor.
- Added `nfctl ndef write --template ID [--field K=V]...`, using the same
  catalogue and encoder as the GUI.
- Added a text language selector and a Smart Poster language selector.
- Added search provider presets with an editable prefix, plus social and
  payment provider presets with a handle on top of the editable URI.
- Added Bluetooth LE OOB (`application/vnd.bluetooth.le.oob`) with address,
  address type, optional name, and optional LE role.
- Added Smart Poster action, resource type, and resource size metadata.
- Added the `NFCT2BIN` binary full-image dump format. `type2 dump`/`restore`
  and the GUI file actions select JSON or binary by `.bin` extension; the
  loader also recognizes the magic.
- Added per-record raw reveal and copy actions for decoded fields, text, URIs,
  and raw payloads.

## Verification

- `go test ./...` passes.
- `make check` passes: tests, vet, i18n catalogue, headless GUI tests, and
  `reuse lint` (397/397 files).
- New unit tests cover text language, provider presets, Bluetooth LE, Smart
  Poster metadata, multi-record flags, sensitive-field masking, and binary dump
  round-trip/corruption handling.
- CLI tests cover template writes, masked and revealed Wi-Fi keys, and binary
  dump/restore.
- GUI tests cover password propagation to reads and page writes, raw and
  multi-record writes, reveal, import/export, and typed lifecycle confirmation.

## Hardware validation (2026-11-07)

- Reader: ACR122U, `acr122_usb:001:004`.
- Card: NTAG215, 7-byte UID, factory configuration, 496-byte NDEF capacity.
- Built `libnfc-ng` from the local source checkout at commit `c1eb612` with
  `pn532_uart,pn53x_usb,acr122_usb,acr122_pcsc,pcsc`; `make libnfc-ng-verify`
  passed.
- Wrote a Text record with language `zh-Hans` and read it back.
- Wrote a Smart Poster with URI, title, `action=save`, `resource_type=text/html`,
  and `resource_size=2048`; all metadata fields were present after read-back.
- Restored the original full image from JSON and from binary; the final page
  array matched the pre-test backup exactly and the original vCard NDEF was
  intact.
- ACR122U occasionally returned `Unable to claim USB interface` or
  `PN532 init failed` on the first open after a rapid sequence; retrying the
  command succeeded. No card data was lost.

## Limitations

- Text language and Smart Poster metadata were hardware-tested. Bluetooth LE
  OOB and provider-preset writes were not hardware-tested; their encoders and
  read-back are covered by unit tests.
- Lifecycle operations are documented in the Phase 05i note.
- No chunked NDEF, signatures, WSC Password Token, or DPP protocol handling.

## Follow-up

- Hardware-validate a Bluetooth LE record and a Smart Poster with metadata on a
  disposable NTAG.
- Phase 05d protected-write GUI polish and Phase 05g Classic NDEF remain open.
