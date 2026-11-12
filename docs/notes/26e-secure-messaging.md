# Note: phase 26.5 DESFire EV1/EV2 secure-messaging read

Status: complete 2026-11-12. Plan:
[26-desfire-workbench.md](../plan/26-desfire-workbench.md). Spec:
[26e-secure-messaging.md](../specs/26e-secure-messaging.md).

## Implementation

- `Session` now carries the EV1 chaining IV and the EV2 transaction
  identifier and command counter. DES/2K3DES sessions keep the D40 channel and
  never enter the secure-messaging path.
- `Desfire::exchange` encodes the command, reassembles chained response
  frames, then decodes the logical response. A status other than `0x9100`, a
  transport error, or a decode failure clears the session.
- EV1: CMAC is chained from the previous value. Reads never carry a command
  MAC; `Plain` and `MACed` responses are `data || MAC(8)`, and `Enciphered`
  responses are CBC ciphertext with a CRC32 over `data || 0x00`. The next IV is
  the last ciphertext block.
- EV2: command and response MACs cover the counter and transaction identifier;
  the 16-byte AES-CMAC is truncated to the odd-indexed bytes. `Plain` responses
  carry no MAC but the counter still advances. `Enciphered` responses are
  MAC-verified before decryption and use an IV derived from the ENC key, TI,
  and current counter.
- The CMAC implementation takes an explicit chaining value, because the EV1
  chain is not the usual all-zero IV. The CRC32 uses the DESFire variant
  (no final complement).
- `read_data`, `get_value`, and `read_records` take the file communication
  mode; `read_records` also takes the record size so enciphered responses can
  be unpadded and length-checked. Secure-messaging file writes return a typed
  error until phase 26.8.
- `nfc-app` selects the effective read protection: a matching authenticated
  key keeps the file mode, while free access or Free GetValue reads in plain.

## Fail-closed behavior

A response is decoded only after the complete logical frame has been
reassembled. The MAC (EV1 and EV2), CRC32 and padding (EV1 enciphered), or
padding and MAC (EV2 enciphered) are verified before returning any data. Any
failure is `VerificationFailed`; the session is dropped and nothing from that
response is surfaced. Unit tests flip a MAC byte and a ciphertext byte and
assert both the error and the dropped session.

## Hardware record

Run on 2026-11-12 on `pn532_uart:/dev/ttyUSB0` with a DESFire EV3 8K. The
owner approved a FormatPICC first because earlier fixtures had left 384 free
EEPROM bytes; formatting restored 8192 bytes. The test created reserved 3K3DES
`F0 0D 06` and AES `F0 0D 07` applications with 24-byte and 16-byte zero keys.
Each application held:

- Standard files `01` (plain), `02` (MACed), `03` (enciphered), 48 bytes each;
- Value files `04` (MACed, initial value 21) and `05` (enciphered, initial
  value 43), with Free GetValue disabled.

Reads ran through the direct `Desfire` layer and through
`nfc-app::Service::desfire_read` for three sessions: 3K3DES EV1, AES EV1, and
AES EV2. Every plain, MACed, and enciphered Standard file read returned the
expected zero-filled data; both value files returned their initial values.
Management commands (`GetFileSettings`, file creation) were also authenticated
on the same channels. Both applications were removed afterwards;
`GetApplicationIDs` was empty.

```text
3K3DES secure-messaging reads passed
AES secure-messaging reads passed
AES EV2 secure-messaging reads passed
applications: none
test reads_maced_and_enciphered_files_over_ev1_and_ev2 ... ok
```

`nfctl` then read the same fixtures through the CLI:

```text
$ nfctl desfire read --aid F00D06 --file 03 --key <24-byte zero> --key-type 3k3des
aid F0 0D 06 file 03 standard data · enciphered · ... · 48 bytes
data: 00 00 ... (48 zero bytes)

$ nfctl desfire read --aid F00D07 --file 02 --key <16-byte zero> --key-type aes
aid F0 0D 07 file 02 standard data · MACed · ... · 48 bytes
data: 00 00 ... (48 zero bytes)

$ nfctl desfire read --aid F00D07 --file 05 --key <16-byte zero> --key-type ev2
aid F0 0D 07 file 05 value · enciphered · ... · get value restricted
value: 43
```

The fixtures were removed afterwards; `nfctl desfire info` reported
`applications: none`.

## ACR122U record

`pcscd` was stopped and the reader opened as `acr122_usb:001:002` (direct
USB/CCID, internal PN532 firmware 1.6). The same test passed for 3K3DES EV1,
AES EV1, and AES EV2, including the plain, MACed, and enciphered fixtures and
the `nfc-app` path. `nfctl desfire read` then returned the same three results
through the ACR122U:

```text
aid F0 0D 06 file 03 standard data · enciphered · 48 bytes
aid F0 0D 07 file 02 standard data · MACed · 48 bytes
aid F0 0D 07 file 05 value · enciphered · value: 43
```

The fixtures were removed and `nfctl desfire info` reported
`applications: none`. `GetFileSettings` decoded the standard and value fixture
settings on the same reader. The 3K3DES/AES authentication and value/record
hardware tests also passed on this reader. No PICC format was needed for the
ACR122U run.

## Tests

- `nfc-desfire` unit tests cover published 3K3DES CMAC and AES-CMAC vectors,
  AN12343 EV2 command/response MAC and IV vectors, the DESFire CRC32 vector,
  chained EV1/EV2 responses, EV2 counter advancement on plain commands, and
  tampered MAC/ciphertext rejection.
- `nfctl/tests/hardware_desfire_secure_messaging.rs` is environment-gated and
  supports `NFC_FOUNDRY_HARDWARE_FORMAT=yes` to recover EEPROM tombstones.

## Next

Phase 26.6 reads the Type 4 NDEF application. Phase 26.8 adds secure-messaging
file writes on top of the primitives added here.
