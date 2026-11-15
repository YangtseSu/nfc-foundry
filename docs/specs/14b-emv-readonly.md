# Spec 14b: Read-only EMV payment-card public data

Status: implemented 2026-11-15. Plan:
[14](../plan/README.md#14-read-only-protocol-adapters-optional). ADR:
[0009](../adr/0009-emv-readonly.md).

## Scope

Read the highest-priority EMV application on an ISO/IEC 14443-4 Type A card and
show its public application data:

- PPSE directory and every advertised AID, label, and priority;
- selected application AID, label, priority, and version;
- masked primary account number and PAN sequence;
- expiry and effective dates;
- cardholder name when the card returns one;
- service code, issuer country, application usage control, AIP, and ATC;
- AFL record count and any read warnings.

The command set is read-only. There is no GENERATE AC, PIN, balance,
transaction-history, top-up, purchase, issuer authentication, or card-write
path.

## Protocol

`nfc-pboc::emv` runs this sequence:

1. SELECT `2PAY.SYS.DDF01` (PPSE). A `6A81`/`6A82`/`6A83` response is
   `Unsupported`.
2. Parse the PPSE and select the application with the lowest priority value;
   ties keep directory order. The PPSE is still required when one AID is
   known.
3. Parse the `6F` FCI: `84` DF name, `50`/`9F12` label, `87` priority, `9F08`
   version, and `9F38` PDOL.
4. Build the PDOL data from the terminal parameters:
   - `9F66` TTQ `26000000` (qVSDC);
   - `9F02`/`9F03` amount and amount other, both zero;
   - `9F1A` terminal country `0156`;
   - `95` TVR, all zero;
   - `5F2A` transaction currency `0156`;
   - `9A` current UTC date in BCD;
   - `9C` transaction type `00`;
   - `9F37` unpredictable number `01020304`;
   - `9F35` terminal type `22`, `9F33` terminal capabilities, `9F40`
     additional terminal capabilities, `9F41` sequence counter, `9F1E`
     interface serial, `9F15` merchant category, `9F1C` terminal ID,
     `9F1D` risk data, and `9F4E` merchant name;
   - UnionPay proprietary `DF60` and `DF69`, zero;
   - any other tag, zero bytes with a warning.
5. Get Processing Options:
   `80 A8 00 00 Lc 83 <PDOL length> <PDOL data> 00`.
   Format 2 (`77`) is parsed as TLV; format 1 (`80`) is AIP plus AFL.
   A `6985` response is an I/O error with the status word.
6. Read the AFL records with `00 B2 <record> <sfi<<3|4> 00`.
   `6A83` ends a file normally. Any other failure stops that file, marks the
   result incomplete, and records a warning. At most 64 records are read.
7. Parse each `70` record template and merge the fields below.

The command APDU is checked against the reader's `ApduCapabilities` and
command-length limit before every exchange. There is no retry.

## Decoding

- PAN: `5A` compressed numeric or `57` Track 2 Equivalent Data. Track 2 is
  accepted as BCD (`D` separator nibble) or ASCII (`D`/`=` separator). The
  PAN is 8..=19 digits.
- Expiry: `5F24` BCD `YYMMDD` when present, otherwise the Track 2 `YYMM`
  value. A month-only expiry displays as `YYYY-MM`.
- Effective date: `5F25` BCD `YYMMDD`.
- Cardholder name: `5F20` printable ASCII. The `20 2F` placeholder is treated
  as absent.
- PAN sequence: `5F34`; service code: three digits from Track 2.
- Issuer country: `5F28`; application usage control: `9F07`.
- AIP: `82`, with the standard capability bits.
- ATC: `9F36`.
- Application version: FCI `9F08`, falling back to `9F08` from card data.

The full PAN is held only inside the read function. The returned `EmvCard`
contains the masked PAN; the full number, Track 2 bytes, AC, IAD, and SDAD are
not returned or persisted. Raw APDU responses are never stored.

## GPO side effects

The card may include an application cryptogram in its GPO response and may
advance its ATC. The profile discards the cryptogram and never sends it to an
issuer. No authorization, settlement, or payment exists. The zero amount is
not a purchase.

## Surfaces

`nfctl emv read` prints the card facts, application list, masked PAN, expiry,
AIP, ATC, and record count. `--json`/`--jsonl` use command `emv.read` and these
result fields:

- `aid`, `label`, `priority`, `appVersion`, `applications[]`;
- `pan` (masked), `cardholderName`, `expires`, `effective`, `panSequence`;
- `serviceCode`, `aip`, `aipCapabilities`, `atc`;
- `issuerCountry`, `applicationUsage`;
- `recordsRead`, `complete`.

The GTK `Bank card` workspace shows the same facts in a read-only report. The
header `Read card` action runs the read when that workspace is selected. There
is no NDEF page, Advanced mode, payment button, or mutation action.

Warnings are printed on stderr and carried in the JSON envelope. A non-ISO-DEP
card exits 4 with `unsupported`; a card without a PPSE or application also
exits 4.

## Tests

- Protocol tests: exact UnionPay GPO APDU, unknown PDOL tag zero-fill, GPO
  format 1 and format 2, AFL validation, BCD and ASCII Track 2, PAN masking,
  malformed records, refused GPO, and missing PPSE.
- Mock read: application label, masked PAN, expiry, name, AIP, ATC, AFL record
  count, and incomplete record failures.
- `nfc-app` tests: service read, non-ISO-DEP refusal, cancellation.
- `nfctl` tests: human output, JSON envelope, PAN masking, and the non-ISO-DEP
  exit code.
- GTK test: workspace switch, empty state, masked number, expiry, application
  list, and the finish path. Gated by `NFC_FOUNDRY_GUI_TESTS=1`.
- Hardware test: `crates/nfctl/tests/hardware_emv.rs`, ignored unless
  `NFC_FOUNDRY_HARDWARE=1` and `NFC_FOUNDRY_DEVICE` are set.

## Non-goals

Type 4B, Type 3 / FeliCa, PSE (`1PAY.SYS.DDF01`) directory records, EMV
transactions, PIN, online authorization, balance, credit limit, statements,
card writes, and cloning are not part of this slice. Type B still depends on
phase 13.
