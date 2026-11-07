# Phase 07 spec: ISO/IEC 14443-4 APDU and DESFire read

Date: 2026-11-07

## Goal

Add a correct Type A ISO-DEP/APDU foundation, read-only DESFire enumeration
and file access, and NFC Forum Type 4 NDEF read/write. Keep all card-family
logic in Go above `nfc.RawTransceiver`.

## Transport

`internal/isodep` owns Type A activation and block framing:

- `OpenSession` enables raw mode, sends RATS `E0 80` with CRC-A, validates the
  ATS CRC and length, and parses FSCI/FWI and optional TA/TB/TC bytes.
- APDUs are carried in I-blocks with CRC-A. Command chaining uses the I-block
  chaining bit and waits for R(ACK). Response chaining is acknowledged with
  R(ACK) and accumulated.
- WTX S-blocks are answered with the same WTX frame.
- Only the default 106 kbit/s rate is used. PPS, CID, NAD, Type B, and extended
  APDU lengths are out of scope.
- `Close` leaves raw mode, which power-cycles the field and reselects the card.

The session is explicit:

```go
session, err := isodep.OpenSession(ctx, transport, card)
defer session.Close(context.WithoutCancel(ctx))
response, err := session.Transmit(ctx, apdu)
```

`Response` contains data and the two status bytes; non-`9000` status is a card
answer, not a transport error.

## APDU

`internal/isodep` provides short APDU encoding and decoding:

- case 1: `CLA INS P1 P2`;
- case 2: header plus `Le` (`00` means 256);
- case 3: header plus `Lc` and data;
- case 4: header plus `Lc`, data, and `Le`.

Command data must be at most 255 bytes; response data at most 256 bytes. The
caller sees the complete response data including any card-specific status
handling. `GET RESPONSE` (`61 XX`) and wrong-length (`6C XX`) retries are
helpers, not implicit behavior.

## DESFire

`internal/desfire` wraps native DESFire commands in ISO 7816-4 APDUs:

```text
90 INS 00 00 [Lc data] 00
```

`91 00` is success. `91 AF` requests an additional frame, sent as
`90 AF 00 00 00`. Other `91 XX` values are DESFire status errors.

Read-only operations:

- `GetVersion (60)` for vendor, type, subtype, version, storage, and protocol;
- `GetApplicationIDs (6A)` for 3-byte application IDs;
- `SelectApplication (5A)`;
- `GetFileIDs (6F)`;
- `GetFileSettings (F5)` for file type, communication mode, access rights, and
  standard/backup/record/value file sizes;
- `ReadData (BD)` for standard and backup data files.

No authentication, key change, formatting, or write command is implemented.
Encrypted or MAC-protected files are reported as inaccessible, not guessed.

## Type 4 NDEF

`internal/type4` reads and writes the NFC Forum Type 4 NDEF application:

- Select the NDEF application AID `D2760000850101`.
- Select and read CC file `E103`, validate CCLEN, mapping version, MLe/MLc,
  and the NDEF file-control TLV `04`.
- Select the NDEF file, read NLEN, read the message, and decode it with the
  existing `type2.ParseMessage` and record catalogue.
- Write validates the message and capacity, requires write access, sets NLEN
  to zero, writes the message in chunks, writes the new NLEN, and reads the
  file back. A backup of the previous message is written before the first
  change.

Type 4 NDEF has no Classic/Type 2 dump format; the backup is the previous raw
NDEF message plus card/CC metadata.

## Application and CLI

```text
nfctl apdu --raw HEX [--json]
nfctl desfire info [--json]
nfctl desfire apps [--json]
nfctl desfire files AID [--json]
nfctl desfire read AID FILE [--json]
nfctl ndef read [--raw] [--reveal] [--json]
nfctl ndef write (...) [--yes] [--dry-run] [--json]
```

`apdu --raw` is diagnostic: it sends exactly one short APDU and prints the
response and status word. It is never advertised as a write path.

`ndef read/write` selects Type 4 from the card SAK (`SAK & 0x20`) when the card
is not Classic or Type 2. The existing template/raw/multi-record inputs are
shared. Type 4 writes require `--yes` and save the previous message under
`$XDG_CONFIG_HOME/NFC Foundry/type4-backups/`.

## Detection

`nfc.InferCardType` gains `iso14443_4` for cards with SAK bit 5 set after the
cascade bit and outside the Classic/Type 2 signatures. The GUI label is
`ISO/IEC 14443-4`; no DESFire/Type 4 capability is advertised from SAK alone.

## Safety

- Raw mode is enabled for the whole ISO-DEP session and cleared on every exit
  path with a field power cycle.
- The selected card UID/ATQA/SAK must match before and after the operation.
- APDU and DESFire reads do not write. DESFire write commands are not exposed.
- Type 4 NDEF write checks the CC write access, capacity, and NLEN; it backs
  up the old message, stops on the first failure, and verifies the final file.
- No DESFire authentication, key change, formatting, or arbitrary APDU write.

## Tests

Hardware-free:

- ATS parsing with and without optional interface bytes, bad CRC/length.
- Short APDU cases 1-4 and status-word preservation.
- I-block command/response chaining, R(ACK), WTX, wrong block number, bad CRC.
- DESFire version, application list, file list/settings, ReadData, 91AF
  chaining, and read-only refusal.
- Type 4 CC parse, NDEF read, malformed NLEN, write capacity, backup, and
  read-back.
- CLI `apdu`, `desfire`, and Type 4 `ndef` with a mock ISO-DEP reader.
- Card detection does not confuse Classic/Type 2.

Hardware, explicit only:

- `TestHardwareISODEP` reads ATS and sends a harmless `Select` APDU.
- `TestHardwareDESFireRead` enumerates and reads a disposable DESFire card.
- `TestHardwareType4NDEF` reads and optionally writes a Type 4 tag when the
  operator confirms ownership.

## Non-goals

- No PPS, bitrate change, CID/NAD, or Type B activation.
- No extended APDU length.
- No DESFire authentication, key management, formatting, or write.
- No encrypted/MAC-protected DESFire file reads.
- No FeliCa, Jewel, or card emulation.
