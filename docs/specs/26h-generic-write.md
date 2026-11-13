# Spec: phase 26.8 DESFire generic data write and transactions

Status: complete 2026-11-13; PN532 UART recorded, ACR122U pending. Plan:
[26-desfire-workbench.md](../plan/26-desfire-workbench.md). Note:
[26h-generic-write.md](../notes/26h-generic-write.md).

## Scope

Write Standard and Backup files with an offset and length in Plain, MACed, and
Enciphered communication modes, and commit or abort the transaction those writes
need. Value, record, and transaction-MAC writes stay in 26.10; application and
file management stays in 26.9.

## Protocol contract

The file's communication mode selects the protection. A key right grants the
write and the file's mode applies; free access grants it in Plain mode whatever
the file settings say; `Denied` refuses it.

EV1 (3K3DES and AES):

- Plain and MACed commands carry `INS || header || data` and, for MACed only, a
  CMAC over that input appended after the data. The CMAC chains from the
  previous command/response MAC.
- Enciphered commands carry the plaintext header followed by CBC ciphertext of
  `data || CRC32(INS || header || data)` padded with zeros to the block size.
  The CBC IV is the current chaining value; the next chaining value is the last
  ciphertext block.
- Every write response is an 8-byte CMAC over the success status; it is
  verified before the write is reported as accepted.

EV2 (AES EV2):

- Plain commands carry no MAC.
- MACed commands carry the truncated AES-CMAC over
  `INS || counter(2, LE) || TI || header || data`.
- Enciphered commands carry the plaintext header followed by CBC ciphertext of
  `data || 0x80` padded with zeros to 16 bytes. The IV is
  `AES-ENC(ENC key, 0xA5 || 0x5A || TI || counter || 0^8)`, and the MAC covers
  the ciphertext.
- The response MAC covers `0x00 || counter+1(2, LE) || TI` and is verified
  before the write is reported as accepted.

Both families:

- The command counter advances only after the card accepts the command.
- A missing, short, or wrong MAC, a failed status, or a transport error clears
  the session; the next operation authenticates again.
- Each APDU carries at most the data the reader's 255-byte APDU limit allows.
  `nfc-app` splits a longer value into chunks and verifies the complete written
  range once.

## Preflight and safety

`nfc-app` runs these checks before the first byte is written:

- The selected card is still the same ISO-DEP card.
- The file settings decode and the file is Standard or Backup.
- The requested range is non-empty and inside the file size.
- A write right exists: the specific write right or the read/write right grants
  it, free or keyed. A key right must match the authenticated key number.
- The file is readable with the same session so the current content can be
  exported and the write read back. A write-only file is refused: the product
  does not report an unverified write as complete.

The current file content and settings are exported before the write. An
unreadable file is refused rather than written without a backup, because the
same access rights would make the read-back verification impossible.

After the write:

- Standard files: read the written range and compare every byte.
- Backup files: read back, then `CommitTransaction`, then read back again. A
  mismatch before commit aborts the transaction. A failed commit is an error;
  uncommitted data is never reported as committed.
- Any mismatch stops the operation and reports the byte range that differed.

## API

- `Desfire::write_data(file_id, offset, data, communication)` returns the
  response `IntegrityCheck`.
- `Desfire::write_record(file_id, offset, data, communication)` keeps the
  existing record-slot semantics and gains the communication mode.
- `Service::desfire_write(cancel, DesfireWriteRequest) -> DesfireWriteResult`.

## Surfaces

- CLI: `nfctl desfire write --aid --file --data|--data-file [--offset]
  [--key --key-type --key-number]`.
- GUI: DESFire → Write with the same fields and a hex/text payload switch.
  Both call the same service.
- Output includes the settings summary, communication mode, response check,
  written range, backup state, commit state, and read-back result. The key is
  never printed.

## Verification

- Unit tests: EV1 plain/MACed/enciphered write wire bytes, EV2 plain/MACed/
  enciphered write wire bytes, response MAC verification, tampered MAC
  rejection, and the session reset.
- Service tests: free and keyed access, denied write, out-of-range offset and
  length, write-only refusal, backup export, Backup commit and abort, and
  read-back mismatch.
- CLI test against the scripted card with a write and a refused range.
- GTK test for the Write tab input parsing.
- Hardware: reserved 3K3DES and AES applications with Plain, MACed,
  Enciphered, and Backup files on PN532 UART, written through the service and
  read back, then removed. Passed 2026-11-13. The ACR122U row is pending.
