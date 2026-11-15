# Spec 14a: China T-Union read-only public data

Status: implemented 2026-11-15. Plan:
[14](../plan/README.md#14-read-only-protocol-adapters-optional). ADR:
[0008](../adr/0008-tunion-readonly.md).

## Scope

Read the China T-Union transit application on an ISO/IEC 14443-4 Type A card
and show its public electronic-purse data:

- PPSE directory and application AID/label/version;
- card category, city, card number (masked), issue and expiry dates;
- electronic purse balance;
- up to ten transaction records and thirty travel records.

The command set is read-only. There is no credit, purchase, top-up, PIN, key,
file-write, NDEF, or card-mutation path.

## Protocol

`nfc-pboc::tunion` runs this sequence:

1. SELECT `2PAY.SYS.DDF01` (PPSE). A `6A82` response is `Unsupported`.
2. SELECT `A000000632010105` (T-Union transit). A `6A82` response is
   `Unsupported`; the PPSE label is used when the FCI has no `50` label.
3. Parse the `6F` FCI: `84` DF name, `9F08` application version, `9F0C`
   metadata (at least 28 bytes).
4. GET BALANCE `80 5C 00 02 04`; the response is a four-byte big-endian amount
   in cents.
5. READ RECORD records 1..=10 on SFI `0x18` (`P2 = 0xC4`), then records
   1..=30 on SFI `0x1E` (`P2 = 0xF4`). `Le` is `00`.

Only SELECT by DF name, READ RECORD, and GET BALANCE are constructed. The APDU
helper checks the reader's `ApduCapabilities` and command-length limit before
each exchange. There is no retry.

A `6A82`/`6A83` record response ends the file normally. An all-zero record
ends the file normally. Any other record status stops that file, marks the
result `complete = false`, and records a warning. A failure before the balance
is an error, not a partial card.

## Decoding

Metadata offsets follow the current Proxmark3 `read_t-union.py` and
T-Union_Master references:

- byte 0: card type (1 normal, 2 student, 3 elder, 4 test, 5 soldier);
- bytes 2..4: city ID, uppercase hex;
- bytes 10..20: card number, uppercase hex with the leading character
  removed (19 digits on the tested card);
- bytes 20..24: issue date, BCD year/month/day;
- bytes 24..28: expiry date, BCD year/month/day.

Transaction record (23-byte minimum):

- `0..2` sequence, big-endian;
- `5..9` amount cents, big-endian;
- `9` type: 1/2 load, 5/6 purchase, 9 compound purchase;
- `10..16` terminal ID, uppercase hex;
- `16..23` BCD timestamp.

Travel record (42-byte minimum, 48 on the tested card):

- `0` type: 3 entry, 4 exit, 6 single trip;
- `1..9` terminal ID;
- `9` sub-type;
- `10..17` station ID;
- `17..21` amount cents;
- `21..25` balance after the trip;
- `25..32` BCD timestamp;
- `32..34` city ID;
- `34..42` institution ID.

BCD fields that fail validation become `None` and produce a warning; the
record remains visible. Raw records are not stored outside the read call.

## Surfaces

`nfctl tunion read` prints the card facts, application, masked card number,
balance, transactions, and travel records. `--json`/`--jsonl` use the normal
envelope with command `tunion.read` and these result fields:

- `aid`, `label`, `appVersion`, `cardType`, `cityId`;
- `cardNumber` (masked), `issued`, `expires`;
- `balanceCents`, `currency` (`CNY`), `complete`;
- `transactions[]` and `travels[]` with decoded fields and timestamp strings.

The GTK `China T-Union` workspace shows the same facts in a read-only report.
The header `Read card` button runs the read when that workspace is selected.
There is no NDEF page, Advanced mode, or mutation button.

Card numbers are masked to the last four digits on both surfaces. The full
number, balance, terminal IDs, and records are never written to logs, project
files, or the key store.

## Tests

- BER-TLV unit tests: nested objects, two-byte tags, long lengths, malformed
  tails, tag formatting.
- FCI/PPSE tests: application version, metadata, directory labels/priority,
  and the real card's truncated labels.
- T-Union tests: full mock read, missing application, missing record, protocol
  error partial read, malformed metadata, BCD/date/amount helpers, and
  reference record offsets.
- `nfc-app` tests: service read, non-ISO-DEP refusal, cancellation.
- `nfctl` tests: human output, JSON envelope, card-number masking, and the
  non-ISO-DEP exit code.
- GTK test: workspace switch, empty state, balance/meta/record rendering, and
  the finish path. Gated by `NFC_FOUNDRY_GUI_TESTS=1`.
- Hardware test: `crates/nfctl/tests/hardware_tunion.rs`, ignored unless
  `NFC_FOUNDRY_HARDWARE=1` and `NFC_FOUNDRY_DEVICE` are set.

## Non-goals

Type 4B, Type 3 / FeliCa, EMV, FMCOS, Calypso, CIPURSE, ITSO, NDEF, Chinese
resident ID, card writes, top-up, purchase, key recovery, and transaction
authorization are not part of this slice.
