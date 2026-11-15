# Note: controlled Amiibo / NTAG215 workflow

Status: implemented 2026-11-15. File decrypt/encrypt was validated against a
user-supplied Amiibo Pack and its key files; no key material or dump data is
committed. A full write and live decrypt passed on PN532 UART with a
discardable Amiibo-writable NTAG215; the ACR122U record is pending.
Plan: [README](../plan/README.md#17-amiibo--ntag215-read-decrypt-encrypt-write-user-keysdata).
Spec: [17-amiibo.md](../specs/17-amiibo.md).

## Scope

- NTAG215 only: 540-byte raw image, 520-byte crypto region, canonical
  `F1 10 FF EE` Amiibo capability container.
- User-supplied data/tag master key records. The application accepts the
  160-byte combined file or the two 80-byte files and keeps the bytes in
  memory only.
- File decrypt and encrypt, live read/decrypt, and an Advanced write that
  re-signs the source for the selected tag's UID.
- No Nintendo key material, identity catalog, retail dump, or Amiibo data is
  shipped, logged, or stored by the application.

## Implemented

### `nfc-amiibo`

- `AmiiboKeys::from_combined` and `from_parts` validate the 80-byte records
  (`unfixed infos`/`locked secret`, magic-byte count) and redact `Debug`.
- HMAC-SHA256 DRBG over the 78-byte prepared seed, AES-128-CTR over the
  `0x188`-byte data region, tag HMAC, then data HMAC, in the published
  Amiibo order.
- `AmiiboTag` enforces the 540-byte size, NTAG215 UID/BCC0/BCC1 layout, and
  manufacturer byte `04`.
- `AmiiboPlain` keeps the 520-byte internal plaintext plus the raw 20-byte
  tail. A 520-byte input receives the canonical NTAG215 tail. `validate`
  requires the Amiibo container and format version 2.
- `rebind_uid` replaces the internal UID pages and page-2 BCC1 for another
  seven-byte UID; `pack` then derives new keys and regenerates both HMACs.
- The numeric identity block exposes game/character ID, variant, figure type,
  model, series, format, write count, and AppID. No name database is shipped.

### Application

- `nfc-app::amiibo` loads keys, converts files, reads a live image, and owns
  the write plan. A live read requires all 135 pages, NTAG215 geometry, and a
  valid UID; the Type 2 "no valid capability container" warning is suppressed
  because an Amiibo intentionally carries `F1 10 FF EE`.
- File conversion verifies both signatures. A wrong key is a typed
  verification failure, never garbage output.
- The write preflight requires Advanced mode, an unlocked complete NTAG215, a
  capability container that can still reach `F1 10 FF EE`, an unlocked target
  for every page to be written, and current PWD/PACK when the configuration
  protects writes.
- The plan stores the full target image, the re-signed expected image, the
  selected/target UIDs, the numeric identity, the 133-page order, the raw
  backup path, and a 120-second one-shot token. `Debug` never prints key bytes
  or the plaintext.
- Execution re-reads and compares the full target image, writes the backup,
  then writes CC page 3 first, user pages 4..129, PWD/PACK, static lock page 2,
  dynamic lock page 130, and configuration pages 131/132. The CC-first order
  means a genuine NTAG215 fails its exact page-3 read-back before any user
  page is touched. It authenticates with the derived PWD and requires PACK
  `80 80` before the configuration write.
- The final verification reselects the card, requires the same UID/ATQA/SAK,
  reads all pages, compares pages 0..132 with the plan, authenticates with the
  new credentials, decrypts the written image, and requires the same numeric
  identity and plaintext as the plan.
- A failure stops at the first page and names the backup. Restore is not
  attempted because lock and configuration bits are one-way.
- The selected tag keeps its UID; UID pages 0 and 1 are never written. The
  source is re-signed for that UID.

## Validation

The user-supplied Amiibo Pack was used only during local validation. A
temporary ignored test and a throwaway reference script walked the pack:

- 278 dumps decrypted and verified both signatures;
- all 278 re-encrypted byte-for-byte;
- 3 files were shorter than 540 bytes and were skipped;
- no key or dump value was copied into the repository.

The synthetic vector in `crates/nfc-amiibo/tests/vector.rs` is the committed
regression: deterministic synthetic master keys and a synthetic plaintext image
were passed through an independent reference implementation. The test checks
the tag, plaintext, identity, tampering, wrong keys, UID rebinding, BCC errors,
and the canonical 520-byte tail.

## Tests

- `nfc-amiibo`: synthetic vector, tamper/wrong-key rejection, UID rebinding,
  canonical tail, key-record parsing and redaction.
- `nfc-app`: live read/decrypt, wrong key, file round trip, planned write,
  page order, derived PWD/PACK, backup, final read, locked target refusal,
  Advanced/confirmation gates, single-use token, changed target image, partial
  read.
- `nfctl`: file decrypt/encrypt with both key forms, missing/wrong keys,
  Advanced and dry-run gates, mock write execution, JSON fields, no secrets.
- GTK: blank NTAG215 read, raw-dump save state, plan dialog, confirmation,
  page/lock/config values, backup, signature/identity labels, and a live read
  after the write.
- Local pack validation is not part of `cargo test --workspace`; it requires
  the user's external key file and dumps.

## Hardware record

- Date: 2026-11-15.
- Reader: PN532 UART `/dev/ttyUSB0`, firmware 1.6.
- Card: discardable Amiibo-writable NTAG215, UID `04 63 CD 3B DE 2A 81`.
  Before the write the tag reported `E1 10 3E 00`, static lock `00 00`,
  dynamic lock `00 00 00`, `AUTH0=FF`, `ACCESS=00`, `CFGLCK=false`, and an
  empty NDEF area. The owner authorized destructive use.
- Keys and source: the user-supplied combined key file and a normal 540-byte
  Splatoon Callie dump.
- Dry run: target UID `04 63 CD 3B DE 2A 81`, source UID
  `04 F0 8D F2 44 4B 80`, identity game `0x0801` character `0x01` model
  `0x025D` series `0x04`, 133 pages, backup path printed.
- Execution: the 133-page write completed and the service reported
  `Amiibo written and verified`. The CC page accepted `F1 10 FF EE` from the
  standard container, so this tag is CC-writable.
- After: `amiibo read` decrypted the written tag with a valid signature and
  the same identity; `type2 info` reported static lock `0F E0`, dynamic lock
  `01 00 0F`, `AUTH0=04`, `ACCESS=5F`, `CFGLCK=true`, and the expected
  unformatted Type 2 container. The raw backup was written before the first
  mutation.
- Not recorded: ACR122U read/write. The tag is now locked and cannot be
  reused for another clean write.

Commands (the user-supplied key and dump paths are environment-specific):

```sh
nfctl --device pn532_uart:/dev/ttyUSB0 type2 info
nfctl --device pn532_uart:/dev/ttyUSB0 --amiibo-keys "$KEY" \
  --backup-dir /tmp/nfc-amiibo-write --dry-run \
  amiibo write --input "$DUMP" --advanced
nfctl --device pn532_uart:/dev/ttyUSB0 --amiibo-keys "$KEY" \
  --backup-dir /tmp/nfc-amiibo-write \
  amiibo write --input "$DUMP" --advanced --yes
nfctl --device pn532_uart:/dev/ttyUSB0 --amiibo-keys "$KEY" amiibo read
nfctl --device pn532_uart:/dev/ttyUSB0 type2 info
```


## Problems and decisions

- The pack's `key_retail.bin` is the 160-byte concatenation of
  `unfixed-info.bin` and `locked-secret.bin`, not a separate format. Loading
  the 80-byte record with the wrong type string is rejected.
- The mock NTAG215 config mapping now recognizes the Amiibo
  `F1 10 FF EE` container byte `0xFF`, so PWD/PACK writes are scripted the
  same way as a retail tag.
- Writes are limited to blank/magic tags because a genuine Amiibo is locked.
  The preflight rejects any already-set static/dynamic lock bit on a target
  page and a permanently locked configuration before the backup.
- The write is intentionally irreversible: it sets the static/dynamic locks
  and CFGLCK. The backup preserves the previous raw image but cannot clear
  those bits; the confirmation says so.
- PN532 UART has a full write/read-back record. The ACR122U was attached but
  no write was run on it; the PN532 test card is now locked and cannot be
  reused for a clean ACR122U write. A second discardable NTAG215 is needed for
  that row.

## Next

- Repeat the live read/decrypt and full write on ACR122U with a second
  discardable Amiibo-writable NTAG215, then add that column to
  `docs/hardware-matrix.md`.
- A read/decrypt against a genuine retail Amiibo is still welcome; the written
  clone already proves the read-back, PWD_AUTH, signature, and identity path.
