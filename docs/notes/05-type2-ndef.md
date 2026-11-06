# Phase 05 implementation note: Type 2 / NTAG21x and NDEF

Date: 2026-11-07

## Implemented

- Added `internal/type2` with NTAG213/215/216 and original MIFARE Ultralight
  geometry, product detection from `GET_VERSION`, BCC0/BCC1 validation,
  capability-container parsing, TLV/NDEF parsing, dump validation, protection
  decoding, NDEF write planning, and restore planning.
- Added `internal/iso14443a.CRCA` and moved the Gen1A CRC-A implementation onto
  it. Type 2 frames use the same CRC.
- Type 2 commands (`READ`, `FAST_READ`, `WRITE`, `PWD_AUTH`, `GET_VERSION`) run
  through a `type2.Session` over `nfc.RawTransceiver`. The session enables raw
  mode, appends CRC-A to commands, validates and strips CRC-A from data
  responses, and leaves raw mode on every exit path.
- Added `workflow.Type2Service`: identify, full read, user-page write,
  NDEF write, restore, dry-run planning, mandatory full-image backup, immediate
  page read-back, final full-image verification, and UID/ATQA/SAK change
  detection.
- Added `FileType2BackupStore` under
  `$XDG_CONFIG_HOME/NFC Foundry/type2-backups/` with mode 0600 files.
- Added application DTOs and service methods, a `Type2EventName` snapshot
  event, and bindings for the GUI.
- Added `nfctl type2 info|read|write|dump|restore` and
  `nfctl ndef read|write`, including `--pwd`, `--all`, `--protected`,
  `--confirm-protected`, and `--dry-run`. `nfctl card info` now probes the
  product for Type 2 tags.
- Added live GUI Memory, NDEF, and Protection tabs. Type 2 reads use the same
  `Read card` action as Classic. Password and PACK values are masked; the GUI
  is read-only for Type 2 in this phase.
- Added deterministic `mock.Type2Tag` raw transport, synthetic NTAG213/215/216
  fixtures under `testdata/type2/`, unit tests, workflow tests, CLI tests, and
  desktop view tests.
- Added the `libnfc-type2-hardware-test` Make target and
  `TestHardwareType2ReadWrite`.

## Transport finding

The first implementation added a shim call using ordinary libnfc framing
(`InDataExchange`). On the validated ACR122U it failed consistently:

```text
nfc transceive type2: Success
error libnfc.driver.acr122_usb Command Code verification failed
```

A raw-mode probe against the same ACR122U and NTAG215 returned correct frames:

```text
raw GET_VERSION response: 00 04 04 02 01 00 11 03 01 9E
raw READ page 0:          04 34 4C F4 C2 83 5C 80 9D 48 00 00 E1 10 3E 00 10 9E
```

The product therefore uses raw mode plus manual CRC-A (`InCommunicateThru`).
This is also the mechanism already proven by the in-process Gen1A path. The
unused framed shim call and `nfc.Type2Transceiver` interface were removed rather
than left as a second transport.

Raw mode must stay enabled across a command sequence and must be disabled
before `CardInfo` reselects a card. `type2.Session` exposes
`Activate`/`Deactivate` so the write workflow can power-cycle the field, check
the card identity, and reactivate for final verification.

## Safety behavior

- User pages only for `type2 write` and `ndef write`; pages 0-3 are never
  written.
- Protected restores require `--yes` and `--confirm-protected`.
- A full-image backup is saved before the first page write. The write is
  aborted if the backup fails.
- Every written page is read back immediately. A mismatch reports
  `verification_failed` with the pages already written and the backup path.
- After writes the session is deactivated, the card is reselected, and
  UID/ATQA/SAK are compared. A different card aborts the operation.
- The final full image is read again and every written page is compared.
- Restore order is user memory, PWD/PACK, configuration, then dynamic lock.
- NDEF write refuses malformed TLVs or any preceding lock/memory-control TLV.
- NDEF parse failures leave the tag untouched and are reported with the raw
  memory still available through `type2 read`.
- PWD and PACK pages are masked in CLI and GUI output unless `--show-keys` is
  set. Dump files contain them and are written mode 0600.

## Verification

Hardware-free:

- `make check` passes: `go test ./...`, `go vet ./...`,
  `go tool mygo i18n check .`, the MyGo view tests, and `reuse lint`
  (358/358 files annotated).
- `CGO_ENABLED=1 go test -tags libnfc ./...` passes against the installed
  `libnfc-ng` 1.8.0.
- Tests cover CRC-A command frames, ACK/NAK, bad CRC, response lengths,
  NTAG geometry, BCC validation, CC parsing, TLV/NDEF parsing, long records,
  malformed messages, NDEF write planning, protected restore order, JSON
  round-trip, synthetic fixtures, mock-tag reads/writes, write verification
  failure, card change, missing backup storage, dry-run behaviour, CLI exit
  codes, and desktop views.

Hardware (ACR122U `acr122_usb:001:005`, NTAG215, 7-byte UID
`04 34 4C C2 83 5C 80`):

- `nfctl card info` reports `NFC Forum Type 2`, ATQA `00 44`, SAK `00`, and
  product `NTAG215`.
- `GET_VERSION` returns `00 04 04 02 01 00 11 03`; CC is `E1 10 3E 00`,
  capacity 496 bytes, 135 pages, 504 user bytes.
- `TestHardwareType2ReadWrite` read the full image and an existing `text/vcard`
  NDEF record.
- With `NFCX_HARDWARE_TYPE2_CONFIRM=I_OWN_THIS_TAG`, the test wrote a text NDEF
  message, verified pages 4-23, saved a backup under
  `/tmp/nfc-type2-backup/`, restored the original user memory, and verified the
  complete original user memory after restore.
- CLI smoke: `type2 dump /tmp/ntag215.json`, `type2 restore --dry-run`,
  `ndef read --raw`, `ndef write --hex D101085402656E68656C6C6F --yes`, then
  `type2 restore /tmp/ntag215.json --yes`. Both writes verified 20 pages; the
  final `ndef read` showed the original vCard record again.
- A transient ACR122U `PN532 init failed, trying again...` message appeared on
  stderr during one CLI restore; the command retried and completed
  successfully. This is the existing ACR122U USB lifecycle behaviour.

Not hardware-tested:

- NTAG213 and NTAG216 use the same commands and geometry model.
- Original MIFARE Ultralight `GET_VERSION` NAK fallback (no tag available).
- Protected restore of configuration, PWD/PACK, or dynamic lock bytes. Those
  writes are implemented and mock-tested but were not exercised on the
  hardware tag to avoid an irreversible configuration change.

## Deviations from the spec

- The spec originally proposed a new ordinary-framing shim primitive. Hardware
  validation showed that path failing on the ACR122U, so the implementation uses
  the existing raw-frame shim with a session-level CRC-A layer. The spec was
  updated to match the validated design.
- The GUI is read-only for Type 2 and NDEF. Writes remain CLI-only until a
  confirmation-dialog design exists.

## Follow-up

- Validate NTAG213/216 and an original Ultralight on hardware.
- Exercise protected restore on a disposable tag.
- Add GUI write confirmation for NDEF and page writes when the dialog surface is
  designed.
