# ADR 0009: Read-only EMV public-data adapter

- Status: accepted
- Date: 2026-11-15
- Phase: 14b

## Context

The supported ISO-DEP work already covered NFC Forum Type 4 NDEF, DESFire, and
the China T-Union transit profile. The next owned-card case was a UnionPay
debit card and a Visa credit card. Both are ISO 7816-4 payment cards: SAK
`0x28`, a PPSE directory, an application AID, a PDOL, and Get Processing
Options. They are not MIFARE Classic, Type 2, DESFire, T-Union, or NFC Forum
NDEF tags.

The product must not gain a payment, PIN, balance, statement, top-up, or
card-write path. A bank card exposes a primary account number, expiry, and
cardholder name. The existing T-Union profile already reused the BER-TLV and
APDU layers, but it has a transit-specific AID and record layout.

## Decision

- Add `nfc-pboc::emv`, a read-only EMV public-data profile over ISO-DEP Type A.
- The command allowlist is SELECT by DF name, Get Processing Options, and
  READ RECORD. There is no GENERATE AC, issuer authentication, PIN, update,
  credit, or purchase command.
- Select the PPSE (`2PAY.SYS.DDF01`) and the lowest-priority-value application.
  Send GPO with a zero amount and qVSDC TTQ `26000000`, then read the AFL
  records.
- Accept Track 2 and `5A` PANs in BCD or ASCII. Mask the PAN to the last four
  digits before it leaves the read function. Do not return or persist Track 2
  bytes, AC, IAD, or SDAD.
- `nfc-app` owns the reader lease and requires `CardType::Iso14443`. The
  protocol crate never opens a reader or selects a device.
- `nfctl emv read` and the GTK `Bank card` workspace call the same service.
  The workspace is read-only; it has no NDEF page, Advanced mode, payment, or
  mutation action.
- The GPO may advance the card's ATC and may include a cryptogram. The cryptogram
  is discarded and never sent to an issuer; no authorization or payment
  exists.

## Consequences

- A new card-family workspace and CLI command exist; Classic, Type 2, DESFire,
  NDEF, and T-Union code paths are unchanged.
- The profile is validated on one UnionPay debit card over PN532 UART and one
  Visa credit card over ACR122U direct USB. ACR122U PC/SC is not yet recorded
  for EMV.
- The cardholder sees application facts, a masked PAN, expiry, AIP/AFL facts,
  ATC, and record count. Balance, credit limit, statements, and transaction
  cryptograms are not exposed.
- Type B EMV, PSE directory records, transactions, PIN, and card writes remain
  follow-ups. Type B still depends on phase 13.
- A future EMV transaction adapter would need a separate spec, ADR, write gate,
  issuer/acquirer agreement, and hardware record; this ADR does not authorize
  one.
