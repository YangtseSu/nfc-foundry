# Note: phase 26.8 DESFire generic data write and transactions

Date: 2026-11-13. Spec:
[26h-generic-write.md](../specs/26h-generic-write.md). Plan:
[26-desfire-workbench.md](../plan/26-desfire-workbench.md).

## Implemented

- `nfc-desfire` writes Standard and Backup files under the file's
  communication mode:
  - EV1 Plain/MACed compute the command CMAC over `INS || header || data` and
    append it only for MACed. Enciphered commands encrypt
    `data || CRC32(INS || header || data)` in CBC with the current chaining
    value as IV.
  - EV2 Plain sends no MAC, MACed appends the truncated AES-CMAC over
    `INS || counter || TI || header || data`, and Enciphered encrypts
    `data || 0x80` with `AES-ENC(ENC key, A5 5A || TI || counter)` before the
    MAC.
  - Status-only responses are verified: EV1 always checks its 8-byte response
    CMAC; EV2 checks the truncated MAC for MACed/Enciphered and advances the
    counter for Plain. A refused status or failed check clears the session.
- `write_data` and `write_record` take a `CommunicationMode` and return the
  response `IntegrityCheck`. The NDEF provisioning and hardware record tests
  pass `Plain`.
- `Service::desfire_write` selects the application, authenticates optionally,
  decodes the file settings, checks the file type, the write right (specific or
  read/write), the range, and the read-back right. It exports the full file
  before writing, chunks at 200 bytes, verifies the written range, commits
  Backup files, and reads the committed range again. A mismatch aborts an open
  Backup transaction.
- `nfctl desfire write --aid --file --data|--data-file [--offset] [--key
  --key-type --key-number]` prints the settings, effective mode, response
  check, range, backup size, commit state, and read-back size.
- The GTK DESFire workspace adds a `Write` tab with the same fields, a
  hex/UTF-8 payload switch, and the same service call. The `zh-CN` catalog
  covers the new labels, hint, and statuses.
- The scripted DESFire card answers `GetFileSettings`, `ReadData`,
  `WriteData`, `CommitTransaction`, and `AbortTransaction` for legacy plain
  sessions, with optional commit failure and read-back corruption for the
  failure paths.

## Verification

Passed:

```text
cargo test -p nfc-desfire --lib
cargo test -p nfc-app --test desfire_write
cargo test -p nfctl --bin nfctl
cargo test -p nfc-foundry --bin nfc-foundry
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

New unit tests cover EV1 Plain/MACed/Enciphered wire bytes, EV2
Plain/MACed/Enciphered wire bytes, response MAC verification, tampered MAC
rejection, and the refused-status session reset. Service tests cover free and
keyed writes, wrong key, denied write, out-of-range write, write-only refusal,
Backup commit, failed commit, and read-back mismatch with abort. The CLI test
runs `desfire write` against the scripted card, including the read-back
report and an out-of-range refusal. The GTK test checks the new tab,
the write button, and the payload/key parsers.

The gated hardware test `hardware_desfire_secure_messaging` now writes a known
range to the Plain, MACed, and Enciphered standard files through
`Service::desfire_write` under 3K3DES, AES EV1, and AES EV2, then reads the
range back through `Service::desfire_read`.

## Problems and resolutions

- A Backup file does not expose pending writes through `ReadData`; the first
  hardware run read the old bytes and reported a mismatch. The service now
  commits a Backup file first and reads the committed range back. Standard
  files still read back immediately. A commit failure still aborts an open
  transaction; a mismatch after a successful commit cannot be aborted and is
  reported as a verification failure.
- The EV1 response MAC is the first 8 bytes of the AES-CMAC. A test initially
  supplied the full 16-byte CMAC as the response, which `split_mac` treated as
  data plus MAC; the test now truncates it to 8 bytes.
- The access-rights word is little-endian in the settings response. The
  low byte's high nibble is read/write and the high byte's high nibble is read;
  the first denied-write fixture was wrong and is now `FE EF`.
- The plan allowed an unreadable backup with an extra confirmation. That
  conflicts with mandatory read-back verification, so a write-only file is
  refused before the first command instead. The spec records the stricter
  behavior.

## Hardware

PN532 UART `pn532_uart:/dev/ttyUSB0`, 2026-11-13, owned MIFARE DESFire EV3 8K
(random UID `04 4D 09 A2 BB 17 90`, no applications before the run). The gated
test created reserved 3K3DES `F0 0D 06` and AES `F0 0D 07` applications with
Plain, MACed, and Enciphered Standard files plus a Plain Backup file, read all
files under 3K3DES EV1, AES EV1, and AES EV2, then wrote a known 16-byte range
through `Service::desfire_write` and read it back:

```text
3K3DES secure-messaging reads passed
AES secure-messaging reads passed
AES EV2 secure-messaging reads passed
3K3DES file 01 plain write: EV1 CMAC verified
3K3DES file 02 MACed write: EV1 CMAC verified
3K3DES file 03 enciphered write: EV1 CMAC verified
3K3DES file 06 plain write: EV1 CMAC verified
3K3DES secure-messaging writes passed
AES file 01 plain write: EV1 CMAC verified
AES file 02 MACed write: EV1 CMAC verified
AES file 03 enciphered write: EV1 CMAC verified
AES file 06 plain write: EV1 CMAC verified
AES secure-messaging writes passed
AES EV2 file 01 plain write: no response MAC
AES EV2 file 02 MACed write: EV2 CMAC verified
AES EV2 file 03 enciphered write: EV2 CMAC verified
AES EV2 file 06 plain write: no response MAC
AES EV2 secure-messaging writes passed
applications: none
```

The CLI was checked against a kept fixture:

```text
$ nfctl --device pn532_uart:/dev/ttyUSB0 desfire write --aid F00D06 \
    --file 06 --offset 0 --data CAFEBABE12345678 \
    --key <24-byte zero> --key-type 3k3des
aid F0 0D 06 file 06
settings: backup data · plain · read key 0, write key 0, rw key 0, change key 0 · 48 bytes
protection: 3K3DES · EV1 CMAC verified
written: 0..8 (8 bytes)
backup: 48 bytes
commit: committed
read-back: 8 bytes verified

$ nfctl --device pn532_uart:/dev/ttyUSB0 desfire read --aid F00D06 \
    --file 06 --offset 0 --length 8 --key <24-byte zero> --key-type 3k3des
data: CA FE BA BE 12 34 56 78
protection: plain · 3K3DES · EV1 CMAC verified
```

The fixtures were removed afterwards and `nfctl desfire info` reported
`applications: none`. The ACR122U row remains unrecorded: `pcscd` held
`acr122_usb:001:002` during the run and could not be stopped without an
interactive credential. Re-run the command above with
`NFC_FOUNDRY_DEVICE=acr122_usb:<bus>:<address>` and `pcscd` stopped.

## Next

26.9 adds application and file management. 26.10 adds value/record/transaction
writes, 26.11 key management, 26.12 the NDEF replace and FormatPICC paths, and
26.13 the shared write gates and journal.
