# Note: 14a China T-Union read-only

Plan: [14](../plan/README.md#14-read-only-protocol-adapters-optional). Spec:
[14a](../specs/14a-tunion-readonly.md). ADR:
[0008](../adr/0008-tunion-readonly.md).

## Implemented

- `nfc-pboc`: new crate with BER-TLV parsing, the read-only APDU builders
  (SELECT by DF name, READ RECORD, GET BALANCE), FCI/PPSE parsing, the T-Union
  profile, and a synthetic mock card/transport. The crate has no write or
  financial command.
- `nfc-app`: `tunion::read` requires `CardType::Iso14443`, runs under the
  normal exclusive `Operation` lease, reselects the card after the read, and
  returns the card identity with the data. `Service::tunion_read` is the
  shared entry point.
- `nfctl`: `tunion read`, command name `tunion.read`, human and JSON output,
  masked card number, and envelope warnings. The `--mock` service exposes
  `mock:tunion`.
- GTK4: `China T-Union` workspace in the workspace selector, a read-only
  report with balance, application metadata, transactions, and travel records,
  and the header `Read card` action. No NDEF page, Advanced mode, or mutation
  action exists.
- Docs: ADR, spec, CLI contract, testing gates, parity matrix, hardware
  matrix, and plan index.

## Protocol notes

The card's PPSE labels are `MOT_T_CASH` and `MOT_T_EP`; the application FCI
carries `84 = A000000632010105`, `9F08 = 02`, and 30 bytes of `9F0C`
metadata. It has no separate `50` label in the FCI, so the PPSE label is the
display label.

The command sequence matches Proxmark3 `read_t-union.py`:

- SELECT `2PAY.SYS.DDF01`;
- SELECT `A000000632010105`;
- GET BALANCE `80 5C 00 02 04`;
- READ RECORD `00 B2 <record> C4 00` for transactions (SFI `0x18`);
- READ RECORD `00 B2 <record> F4 00` for travel records (SFI `0x1E`).

The tested card returns 23-byte transaction records and 48-byte travel
records. Travel records after the last used entry are zero-filled; the reader
stops at the first all-zero record.

## Hardware record

2026-11-15, PN532 UART `pn532_uart:/dev/ttyUSB0`, PN532 firmware 1.6:

- T-Union CPU card UID `9D 7F B9 5F`, ATQA `00 08`, SAK `28`,
  ISO/IEC 14443-4.
- PPSE listed `A000000632010106` (`MOT_T_CASH`, priority 1) and
  `A000000632010105` (`MOT_T_EP`, priority 2).
- Selected `A000000632010105`; the balance command returned a four-byte
  amount; ten transaction records and fourteen travel records decoded; the
  result was complete. No transaction log entry required authentication.
- No write, credit, purchase, PIN, or other mutation command was sent.

ACR122U is not recorded for this card. During the run the ACR122U held a
different ISO-DEP card, so no cross-reader claim is made. The `nfctl`
`--mock` path is the hardware-free test surface.

## Tested

- `cargo fmt --all --check`.
- `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo test --workspace`: 879 passed, 0 failed, 9 ignored.
- `cargo test -p nfc-pboc`, `cargo test -p nfc-app tunion`,
  `cargo test -p nfctl --test cli tunion`, and the gated GTK scenario.
- `reuse lint`: 734/734 files compliant.
- Hardware: `NFC_FOUNDRY_HARDWARE=1
  NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 cargo test -p nfctl --features
  hardware --test hardware_tunion -- --ignored`.

New tests cover TLV parsing, FCI/PPSE parsing, metadata and record decoding,
mock reads, partial reads, cancellation, CLI human/JSON output, card-number
masking, the non-ISO-DEP refusal, and the GTK workspace/report.

## Limitations

- Type 4B, Type 3 / FeliCa, EMV public data, FMCOS, Calypso, CIPURSE, and ITSO
  remain phase 14 follow-ups. Type B adapters still need phase 13.
- Chinese resident ID is phase 16, blocked on an authorized SAM and written
  authorization. It is not part of this adapter.
- The ACR122U row is pending.
- Cards that protect the log files with application authentication will show
  the public balance and a partial record set with warnings; no key recovery or
  authentication path is added.
- The card number is masked to the last four digits. The full number is not
  available from the CLI or GUI.
