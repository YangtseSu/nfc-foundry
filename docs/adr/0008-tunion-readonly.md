# ADR 0008: Read-only T-Union public-data adapter

- Status: accepted
- Date: 2026-11-15
- Phase: 14a

## Context

The supported card families were MIFARE Classic, NFC Forum Type 2 / NDEF,
DESFire / Type 4 NDEF, and the Amiibo NTAG215 application. A China T-Union
transit card is an ISO 7816-4 CPU card: SAK `0x28`, a PPSE directory, and the
MOT transit application `A000000632010105`. It is not MIFARE Classic, Type 2,
DESFire, or an NFC Forum NDEF tag. Selection and generic ISO-DEP APDUs already
worked; there was no application layer.

The product must not gain a credit, purchase, top-up, PIN, or card-write path.
The card number and balance are cardholder data, and transaction logs can
identify travel. A new card family also needs its own boundary rather than
being folded into Classic, Type 2, or Type 4.

## Decision

- Add `nfc-pboc` for read-only ISO 7816-4 public-data profiles. The first
  profile is China T-Union over ISO-DEP Type A.
- The T-Union command allowlist is SELECT by DF name, READ RECORD, and
  `80 5C 00 02 04` GET BALANCE. No update, credit, purchase, PIN, or key
  command exists in the crate.
- Select the PPSE (`2PAY.SYS.DDF01`), then the transit AID
  `A000000632010105`. Read the electronic purse balance and up to ten
  transaction records from SFI `0x18` and thirty travel records from SFI
  `0x1E`, following the current Proxmark3 `read_t-union.py` reference.
- `nfc-app` owns the reader lease and requires `CardType::Iso14443`. The
  protocol crate never opens a reader and never selects a device.
- `nfctl tunion read` and the GTK `China T-Union` workspace call the same
  service. The workspace is read-only; it has no NDEF page, Advanced mode, or
  mutation action.
- The card number is masked to its last four digits in human and JSON output.
  Raw APDU responses, the full card number, terminal IDs, and record bytes are
  not persisted. Warnings report malformed fields and an incomplete read.
- Type 4B, Type 3 / FeliCa, EMV public data, FMCOS, Calypso, CIPURSE, and ITSO
  remain phase 14 follow-ups. Type B work still depends on phase 13. A
  Chinese resident ID remains phase 16, blocked on an authorized SAM; it is
  not part of this adapter.

## Consequences

- A new crate and a new card-family workspace exist; Classic, Type 2, DESFire,
  and NDEF code paths are unchanged.
- The T-Union profile is validated on one PN532 UART card. The ACR122U row is
  not recorded until the same read runs on that transport.
- The profile is read-only even when the card exposes a financial applet. No
  balance-changing or transaction command is reachable from the GUI or CLI.
- A future EMV adapter can reuse the BER-TLV parser and APDU builders, but it
  gets its own profile, tests, redaction rules, and hardware record.
