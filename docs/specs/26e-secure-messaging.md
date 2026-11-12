# Spec: phase 26.5 DESFire EV1/EV2 secure-messaging read

Status: complete 2026-11-12. Plan:
[26-desfire-workbench.md](../plan/26-desfire-workbench.md). Note:
[26e-secure-messaging.md](../notes/26e-secure-messaging.md).

## Scope

Read Standard, Backup, Value, Linear Record, and Cyclic Record files after a
3K3DES, AES EV1, or AES EV2 authentication. DES/2K3DES sessions keep the legacy
D40 channel and do not use secure messaging.

Secure-messaging file writes are out of scope; the write methods answer with a
typed error until phase 26.8.

## Protocol contract

EV1 (3K3DES and AES EV1):

- The command CMAC covers `INS || command data` and chains from the previous
  CMAC value; the response CMAC covers `response data || 0x00` and becomes the
  next chaining value.
- `Plain` and `MACed` file reads return `data || MAC(8)`; the MAC is verified
  before any data is returned.
- `Enciphered` reads return CBC ciphertext; the IV is the command CMAC. The
  plaintext is `data || CRC32(data || 0x00) || zero padding`, and the next IV is
  the last ciphertext block. The CRC32 is the DESFire variant (reflected
  polynomial, initial value `0xFFFFFFFF`, no final complement, little endian).
- A read command never carries a command MAC, including a read of a MACed
  file.

EV2 (AES EV2):

- The command MAC is the AES-CMAC over
  `INS || counter(2, LE) || TI || command data`, truncated to bytes
  `1, 3, 5, 7, 9, 11, 13, 15`.
- The response MAC covers `0x00 || counter+1(2, LE) || TI || response data`
  with the same truncation. `Plain` responses carry no MAC but still advance
  the counter.
- `Enciphered` responses are verified by MAC before decryption. The response
  IV is `AES-ENC(ENC key, 0x5A || 0xA5 || TI || counter || 0^8)`; padding is
  ISO/IEC 9797-1 method 2, always adding a block when the data is aligned.

Both families:

- Chained response frames are reassembled into one logical response before
  secure-messaging decoding; the MAC is at the end of the logical response.
- A missing, short, or wrong MAC, a CRC mismatch, invalid padding, or a length
  mismatch is an error. No bytes from that response are returned.
- Any error status or decode failure clears the session; the next operation
  must authenticate again.

## Effective protection

The file's communication mode applies when the read is granted by the
authenticated key. The PICC runs the operation in plain when it is granted by a
free access right (`0x0E`), and `GetValue` is plain when the file has Free
GetValue enabled. `nfc-app` selects the effective mode from the decoded file
settings, the request key number, and whether a key was supplied.

## API

- `Desfire::read_data(file_id, offset, length, CommunicationMode)`
- `Desfire::get_value(file_id, CommunicationMode)`
- `Desfire::read_records(file_id, start_record, count, record_size, CommunicationMode)`

Record size is required so an enciphered response can be unpadded and checked
against the exact expected length.

## Verification

- Unit tests: published CMAC vectors, AN12343 EV2 MAC and IV vectors, CRC32,
  chained frames, MAC/CRC/padding tampering, counter advancement.
- Hardware test: reserved 3K3DES and AES applications with plain, MACed, and
  enciphered Standard files and MACed/enciphered Value files, read through the
  direct layer and through `nfc-app`, then removed.
- GUI and CLI use the same `nfc-app::desfire_read`; no CLI-only read path.
