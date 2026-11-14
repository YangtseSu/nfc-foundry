# Note: phase 25.5 DESFire EV3 8K NDEF enablement

Status: inventory, factory-key authentication, NDEF provisioning, undo, CLI,
GTK surfaces, and the PN532 UART and ACR122U format/write/delete hardware
records complete 2026-11-12.
Plan: [25-card-support.md](../plan/25-card-support.md). Spec:
[25e-desfire.md](../specs/25e-desfire.md).

## Card

- MIFARE DESFire EV3 8K.
- Random UID enabled: observed selection UID `04 77 1F A2 BB 17 90`; the last
  four bytes `A2 BB 17 90` are stable. An earlier session observed
  `04 4D 09 A2 BB 17 90`.
- Reader: PN532 UART (CH340), `pn532_uart:/dev/ttyUSB0`, firmware 1.6.

## Inventory

`nfctl desfire info`:

```text
MIFARE DESFire EV3 · vendor 04 type 01 subtype 01 version 51.0 storage 1A protocol 05
applications: none
```

`GetKeySettings` (`0x45`) returned `0F 01`, so the PICC has one key.
`GetApplicationIDs` returned an empty list. No authentication or write was
performed.

## Key investigation

- `AuthenticateEV2First` (`0x71`) with an AES zero key returned `91AE`.
- `AuthenticateISO` (`0x1A`) returned an 8-byte challenge with `91AF`, which is
  the DES/2K3DES response shape.
- Follow-up on 2026-11-12: the factory PICC key is legacy DES/2K3DES all zero.
  `Authenticate` (`0x0A`) with an 8-byte zero DES key authenticates both
  directions when the second frame uses D40 send mode (XOR with the previous
  ciphertext, then 3DES decrypt). RndA from the card matches. A 16-byte
  2K3DES zero key also authenticates; `0x1A` 3K3DES and `0x71` AES do not.
- The earlier 54-key dictionary run failed because it used standard CBC for the
  second frame and the wrong command/key type. The card is factory-default, not
  custom-keyed.

No NDEF application was created. The CLI deliberately exposes only
`desfire info`; native authentication and application creation are not exposed
through the CLI or GUI yet. The PICC key is no longer the blocker.

## Implementation

- New `nfc-desfire` crate with `GetVersion`, `GetApplicationIDs`,
  `SelectApplication`, `GetFileIDs`, and `GetFileSettings`.
- Legacy DES/2K3DES authentication (`0x0A`) with D40 send mode, session-key
  derivation, and random RndA.
- Primitive `CreateApplication`, `DeleteApplication`, `CreateStdDataFile`,
  `WriteData`, `ReadData`, `CommitTransaction`, and `AbortTransaction`.
- Application and file creation support ISO file IDs and an ISO 7816-4 DF
  name; these are required for the NDEF Tag Application.
- `Service::desfire_inventory` and `nfctl desfire info`.
- A read-only GTK `DESFire` `Inventory`/`Read` workspace.

## Temporary application round trip

The hardware test on 2026-11-12 used the factory DES zero key on PN532 UART:

```text
create application F0 0D 01 (1 key, 2K3DES)
authenticate app key 0
create standard file 01 (plain, 32 bytes, key-0 access)
write 16 bytes at offset 0
read 16 bytes; payload matched
delete application F0 0D 01
GetApplicationIDs: empty
```

The test is idempotent: it removes a leftover reserved AID before starting and
uses the same reader for cleanup. It deletes only `F0 0D 01`.

```sh
NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfc-desfire --features hardware --test hardware_legacy -- --ignored
```

`hardware_legacy` also passed on ACR122U `acr122_usb:001:002` on 2026-11-14,
with the same reserved AID and cleanup.

## CLI read diagnostic

`nfctl desfire read` exposes the same plain-file read without a GUI. On
2026-11-12 a reserved temporary application `F0 0D 01` was seeded with a
32-byte standard file containing `NFC Foundry CLI read`, then read through the
CLI and removed:

```text
aid F0 0D 01 file 01 settings 00 00 00 00 20 00 00
data: 4E 46 43 20 46 6F 75 6E 64 72 79 20 43 4C 49 20 72 65 61 64 00 ...
ascii: NFC Foundry CLI read............
```

Omitting `--length` reads the rest of the file. A non-parity wrong key returns
`91AE`; a free-access file can be read without `--key`. The key is accepted
from `--key` or `NFC_FOUNDRY_DESFIRE_KEY`, never printed. This started as a
developer diagnostic; the read-only GUI workspace now exposes the same service.

## GUI workspace

The GTK workspace selector has a `DESFire` entry with `Inventory`, `Read`, and
`NDEF` tabs. `Inventory` lists the version and applications; `Read` takes an
AID, file ID, optional legacy key, key number, offset, and length, then shows
the settings, hex data, and ASCII. `NDEF` has a session-scoped Advanced switch,
a PICC master key and key number, and `Format NDEF application`/`Delete NDEF
application` actions. Both actions show a typed-confirmation dialog; the
advanced switch resets on card removal, reader change, and restart. The read
and refresh buttons use the same `nfc-app` service as the CLI.

The GUI test `desfire_workspace_switches_and_parses_inputs` covers workspace
selection, default inventory text, Advanced gating, key/number parsing, and
the AID/key parsers.

## NDEF provisioning

`nfctl desfire format-ndef` and `nfctl desfire delete-ndef` call
`Service::desfire_ndef_format`/`Service::desfire_ndef_delete`. The service
creates `D27600` with DF name `D2760000850101`, app ISO ID `E110`, CC file
`01`/`E103` (15 bytes), and NDEF file `02`/`E104` (255 bytes), writes the CC
and empty NLEN, commits, then reads the CC and NDEF file through the Type 4
adapter. A failure after application creation rolls back by aborting the
transaction and deleting the application.

Undo validates the exact file numbers, plain communication mode, sizes, and
CC bytes before deleting, so it does not remove an NDEF application created by
another tool. Neither operation changes the PICC key or claims a full-card
backup.

The EV3 random UID changes its leading bytes on reselection. The Type 4 and
DESFire workflows now compare ATQA, SAK, and the stable last four UID bytes;
Classic and Type 2 keep the strict UID comparison.

Read-only CLI checks on 2026-11-12:

```sh
./target/debug/nfctl --device pn532_uart:/dev/ttyUSB0 desfire info
# unsupported: the selected card is not an ISO/IEC 14443-4 card
```

The PN532 UART held an NTAG216 (`04 14 B5 7F DD 2A 81`) at that time, and the
ACR122U held a MIFARE Classic 1K (ATR `3B 8F 80 01 80 4F 0C A0 00 00 03 06 03
00 01 00 00 00 00 6A`). Neither reader had the DESFire EV3, so the hardware
run could not start.

## Hardware test

```sh
NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfctl --features hardware-write \
  --test hardware_desfire_ndef -- --ignored
```

The test refuses to run when the NDEF application already exists. It checks
the Advanced and confirmation gates, verifies that a wrong key creates
nothing, formats the card, writes and clears one Text record through the Type
4 path, deletes only the NDEF application, and compares the final application
list with the pre-format list. On failure it attempts the same guarded delete.

## PN532 UART hardware record

Run on 2026-11-12 with the DESFire EV3 on `pn532_uart:/dev/ttyUSB0` (PN532
firmware 1.6). The card had a random UID; the observed selection was
`04 4D 09 A2 BB 17 90` with the stable suffix `A2 BB 17 90`.

The gated service test passed:

```text
formatted aid=[00, 76, D2] cc=15 state=Empty
test formats_writes_restores_and_deletes_the_ndef_application ... ok
```

It checked the Advanced and confirmation gates, verified that a wrong key
created nothing, created `D27600` and the `E103`/`E104` files, read the CC and
empty NDEF file back through the Type 4 adapter, wrote and read one Text
record, cleared it to the empty state, deleted the NDEF application, and
compared the final application list with the pre-format list (`applications:
none`).

The first run stopped at `CommitTransaction` with `0x910C` (`NoChanges`).
Treating that status as success, as PM3 does, let the next run pass; the
Type 4 read-back then verified the CC and empty NDEF. The service still fails
on any other commit status.

The CLI surfaces were exercised on the same card:

```text
$ nfctl --device pn532_uart:/dev/ttyUSB0 --yes desfire format-ndef \
    --key 0000000000000000 --advanced
...
NDEF application formatted
CC: 00 0F 20 00 3B 00 34 04 06 E1 04 00 FF 00 00
NDEF file: empty

$ nfctl --device pn532_uart:/dev/ttyUSB0 desfire info
MIFARE DESFire EV3 · vendor 04 type 01 subtype 01 version 51.0 storage 1A protocol 05
application 00 76 D2
  file 01: 00 00 00 E0 0F 00 00
  file 02: 00 00 E0 EE FF 00 00

$ nfctl --device pn532_uart:/dev/ttyUSB0 --yes desfire delete-ndef \
    --key 0000000000000000 --advanced
NDEF application D27600 deleted
applications: none
```

`desfire info` prints the native AID bytes (`00 76 D2`); the logical
application ID is `D27600`. The test and the CLI cycle both left the card with
no applications, matching the pre-format state. ACR122U
`acr122_usb:001:002` (`pcscd` stopped) passed the same provisioning,
Type 4 write/read/clear, guarded delete, and CLI read cycle on 2026-11-12; the
ACR122U row is validated.

## To continue

The follow-on slices are tracked in
[phase 26](../plan/26-desfire-workbench.md): file-settings decoding, Value/Record
reads and writes, 3K3DES/AES authentication, EV1/EV2 secure messaging,
application/file/key management, Type 4 NDEF, and cryptographic verification.

Do not add business templates (access control, wallet, transit).

A changed PICC key is not recoverable from the card; record and back it up
before any key change.
