# Note: 14b EMV payment-card read-only

Plan: [14](../plan/README.md#14-read-only-protocol-adapters-optional). Spec:
[14b](../specs/14b-emv-readonly.md). ADR:
[0009](../adr/0009-emv-readonly.md).

## Implemented

- `nfc-pboc`: new `emv` module with PPSE/application selection, PDOL
  construction, Get Processing Options, AFL record reads, BER-TLV decoding,
  BCD/ASCII Track 2 decoding, PAN masking, and a synthetic mock card/transport.
  The shared FCI parser now exposes `9F38` and accepts `9F12` as a label
  fallback.
- `nfc-app`: `emv::read` requires `CardType::Iso14443`, runs under the normal
  exclusive `Operation` lease, reselects the card after the read, and returns
  the card identity with the data. `Service::emv_read` is the shared entry
  point.
- `nfctl`: `emv read`, command name `emv.read`, human and JSON output, masked
  PAN, and the `mock:emv` fixture.
- GTK4: `Bank card` workspace in the workspace selector, a read-only report
  with the application list, masked number, expiry, AIP/ATC, and record count,
  and the header `Read card` action.
- Docs: ADR, spec, CLI contract, testing gate, parity matrix, hardware matrix,
  and plan index.

## Protocol notes

The profile selects the highest-priority PPSE application and sends GPO with
amount zero, qVSDC TTQ `26000000`, country/currency `0156`, current UTC date,
and TVR zero. Unknown PDOL tags are zero-filled with a warning. UnionPay uses
the same PDOL shape as the T-Union reader plus `DF60`/`DF69`; Visa uses the
standard qVSDC PDOL.

The tested UnionPay card returns:

- PPSE AID `A000000333010101`, label `UnionPay Debit`, priority 1;
- FCI version `0030`, 9F38 PDOL with `DF60`/`DF69`;
- GPO format 2 with AIP `7C00`, an AFL, `57` Track 2, and an AC/SDAD pair;
- seven AFL records carrying `5A`, `5F24`, `5F25`, `5F28`, `9F07`, and `9F08`.

The tested Visa card returns:

- PPSE AID `A0000000031010`, label `VISA CREDIT`, priority 1;
- FCI version absent, standard qVSDC PDOL without `DF60`/`DF69`;
- GPO format 2 with AIP `2000`, `57` Track 2, and an AC;
- no AFL, so the read has zero records and uses the inline Track 2 data.

Track 2 and `5A` are BCD on both cards. The first implementation accepted only
ASCII Track 2 and therefore missed the PAN; the hardware run caught this and
the parser now accepts both encodings.

## Hardware record

2026-11-15, PN532 UART `pn532_uart:/dev/ttyUSB0`, PN532 firmware 1.6:

- UnionPay debit card UID `2D D1 5B 63`, ATQA `00 08`, SAK `28`,
  ISO/IEC 14443-4.
- Selected `A000000333010101`; label `UnionPay Debit`; version `0030`.
- Masked PAN `**************3678`; expiry `2031-07-03`; effective
  `2023-07-03`; PAN sequence `01`; service code `220`; issuer country `0156`;
  application usage `FF00`; AIP `7C00`; ATC 20 at the recorded run; seven
  records; result complete with no warnings.
- Hardware test completed in 1.31 s.

2026-11-15, ACR122U direct USB/CCID `acr122_usb:001:002`, `pcscd` stopped:

- Visa credit card UID `39 34 C1 60`, ATQA `00 04`, SAK `28`,
  ISO/IEC 14443-4.
- Selected `A0000000031010`; label `VISA CREDIT`; no FCI version.
- Masked PAN `************3514`; expiry `2031-06`; PAN sequence `01`; service
  code `201`; AIP `2000`; ATC 5 at the recorded run; zero records; result
  complete with no warnings.
- Hardware test completed in 0.71 s.

No full PAN, Track 2 bytes, application cryptogram, IAD, or SDAD was printed,
logged, or persisted. No GENERATE AC, PIN, balance, statement, purchase,
top-up, or write command was sent. ACR122U PC/SC is not yet recorded for EMV.

## Tested

- `cargo fmt --all --check`.
- `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo test --workspace`.
- `cargo test -p nfc-pboc`, `cargo test -p nfc-app --lib emv`,
  `cargo test -p nfctl --test cli emv`, and the gated GTK scenario.
- `reuse lint`.
- Hardware: `NFC_FOUNDRY_HARDWARE=1
  NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 cargo test -p nfctl --features
  hardware --test hardware_emv -- --ignored --nocapture`.
- Hardware: the same command with `NFC_FOUNDRY_DEVICE=acr122_usb:001:002`.

## Limitations

- Type 4B, Type 3 / FeliCa, PSE (`1PAY.SYS.DDF01`), FMCOS, Calypso, CIPURSE,
  and ITSO remain phase 14 follow-ups. Type B work still depends on phase 13.
- EMV transactions, PIN, online authorization, balance, credit limit,
  statements, and card writes are not implemented and are not planned in this
  profile.
- GPO may advance the card ATC and may include a cryptogram. The cryptogram is
  discarded; no authorization exists.
- The full PAN is never shown. The masked number, expiry, and cardholder name
  are cardholder data; do not add them to logs or project files.
- ACR122U PC/SC is pending for this profile; PN532 UART and ACR122U direct USB
  are validated.
