# Note: phase 25.4 Type 2 UID change

Status: complete 2026-11-12 on PN532 UART with a 7-byte NTAG213 clone;
ACR122U not recorded. Plan:
[25-card-support.md](../plan/25-card-support.md). Spec:
[25d-type2-uid.md](../specs/25d-type2-uid.md).

## Card

- UID-changeable NTAG213 clone.
- UID `04 00 E3 C2 00 00 02`, ATQA `00 44`, SAK `00`.
- Reader: PN532 UART (CH340), `pn532_uart:/dev/ttyUSB0`, firmware 1.6.
- Product reports NTAG213, 45/45 pages, CC `E1 10 12 00`, empty NDEF.
- UID pages before the run: page 0 `04 00 E3 6F`, page 1 `C2 00 00 02`,
  page 2 `C0 08 00 00` (BCC0/BCC1 valid, static lock `00 00`).

## Same-value probe

The first hardware step wrote the current page 0/1/2 values back:

```text
page 0 accepted same-value write: 04 00 E3 6F
page 1 accepted same-value write: C2 00 00 02
page 2 accepted same-value write: C0 08 00 00
```

The UID was unchanged after reselection. A genuine NTAG213 would refuse page 0.

## Change and restore

Dry run:

```text
NFC Forum Type 2 · current UID 04 00 E3 C2 00 00 02, target UID 04 00 E3 C2 00 00 03
page 0: 04 00 E3 6F -> 04 00 E3 6F
page 1: C2 00 00 02 -> C2 00 00 03
page 2: C0 08 00 00 -> C1 08 00 00
```

With `--advanced --yes`, the service wrote page 1, page 2, and page 0:

```text
UID changed: 04 00 E3 C2 00 00 02 -> 04 00 E3 C2 00 00 03
page 0 verified: 04 00 E3 6F
page 1 verified: C2 00 00 03
page 2 verified: C1 08 00 00
backup: /tmp/nfc-uid213/backups/0400E3C2000002-NTAG213-uid-*.json
```

`type2 info` reselected the card as `04 00 E3 C2 00 00 03` and read 45/45
pages. A second guarded run restored the original UID:

```text
UID changed: 04 00 E3 C2 00 00 03 -> 04 00 E3 C2 00 00 02
page 0 verified: 04 00 E3 6F
page 1 verified: C2 00 00 02
page 2 verified: C0 08 00 00
backup: /tmp/nfc-uid213/backups/0400E3C2000003-NTAG213-uid-*.json
```

A fresh project dump reported 45/45 pages. `jq -S '.pages'` over the original
and restored dumps was identical. Static lock bytes stayed `00 00`; no
configuration page was written.

## Implementation

- The transport layer has no changes. `nfc-app::type2` adds the UID plan and
  execution: standard-layout validation, BCC0/BCC1, page 1→2→0 ordering,
  per-page read-back, reselection, and automatic original-UID restore.
- The normal Type 2 page policy still refuses manufacturer pages; the UID path
  is a separate Advanced service.
- `nfctl type2 uid` supports `--dry-run`, `--advanced`, and the normal `--yes`
  confirmation.
- The GTK Protection tab adds the target UID entry and plan dialog; the worker
  uses the same service and re-reads the card after the change.
- The mock Type 2 page writer can update its selected UID when explicitly
  enabled, so the application and GTK tests exercise the real service flow.

## Remaining

- ACR122U UID-page probe and change are not recorded.
- Four- and ten-byte UIDs are not implemented.
- Password-protected UID pages are not supported by this workflow.
- No NDEF or lock-bit behavior was changed.
