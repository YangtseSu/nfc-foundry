# Phase 04 spec: native Go Gen1A

Date: 2026-11-06

## Goal

Remove the `nfc-mfsetuid` executable from the product and write Gen1A block 0
in-process with the reader that is already open. The channel must fail with a
specific error on an ordinary card and must never route through the external
engine handoff.

## Layering

The C shim gains only generic raw-frame primitives. The Gen1A frame sequence,
CRC-A calculation, and card checks live in Go.

### Shim primitives

```text
nfcx_reader_set_raw_mode(handle, enabled)
nfcx_reader_transceive_bytes(handle, tx, tx_length, rx, capacity, *rx_length)
nfcx_reader_transceive_bits(handle, tx, tx_length, tx_bits, rx, capacity, *rx_bits)
```

`set_raw_mode(true)` disables `NP_HANDLE_CRC`, `NP_EASY_FRAMING`, and
`NP_AUTO_ISO14443_4` while leaving PN53x parity generation enabled.
`set_raw_mode(false)` restores those properties, clears the shim's selected
target/authentication state, and power-cycles the RF field so a rewritten
manufacturer block is visible to the next anti-collision.

Raw transceive calls use a bounded timeout, copy only into caller buffers, and
map libnfc failures through the existing stable error codes. They do not run
the high-level presence/reselection classification used by ordinary MIFARE
commands; a HALT response is expected to be absent and must not trigger an
implicit reselect.

### Go protocol

`internal/mifare/gen1a.go` implements:

- ISO/IEC 14443-3 CRC-A with the upstream test vectors
  (`50 00` -> `57 CD`, `A0 00` -> `5F B1`);
- HALT (`50 00` + CRC-A);
- the 7-bit `0x40` unlock step and the `0x43` second step;
- the raw MIFARE write command (`A0 00` + CRC-A) followed by the complete
  16-byte block 0 plus CRC-A;
- checks for a 4-byte UID, Classic 1K identity, and the new block 0 BCC.

The writer exposes two sentinel failures:

- `ErrGen1ANotSupported`: the card did not answer the 7-bit `0x40` step (or
  the `0x43` step), so it is not an unlocked Gen1A card.
- `ErrGen1AWriteUncertain`: unlock succeeded but a write frame's response was
  missing or not the MIFARE ACK. The caller must reselect and verify; a changed
  UID and matching block 0 still count as success.

## Workflow integration

`UIDService.Write` continues to:

1. repeat the preflight under one exclusive reader lease;
2. save the original block 0 backup;
3. verify the expected card, authenticate sector 0, and try the ordinary
   CUID/Gen2 manufacturer write;
4. reselect and verify the complete block 0.

If the ordinary write leaves the UID unchanged and the reader implements
`nfc.RawTransceiver`, the same lease now runs the Gen1A path in-process:

1. reselect and re-check UID, ATQA, and SAK against the planned card;
2. enter raw mode, HALT, unlock, and write the planned 16-byte block 0;
3. leave raw mode, reselect until the requested UID appears, authenticate with
   the original key, and require an exact block 0 readback plus valid BCC.

No second `WithReader` call, reader close, process launch, runtime locator
lookup, or capability check is involved. The backup saved before the write is
the recovery artifact.

## Removals

- `internal/attack/mfsetuid.go` and its tests.
- `workflow.UIDFallbackWriter`, `workflow.Gen1AUIDWriter`, and
  `workflow.UIDFallbackRequest`.
- The `nfc-mfsetuid` diagnostic check.
- The `nfc-mfsetuid` runtime manifest component.
- Terminal help lines that advertise the upstream executable.

`third_party/libnfc/nfc-mfsetuid-BSD-2-Clause.txt`, the REUSE annotation, and
the `THIRD_PARTY_NOTICES.md` attribution remain because the Go sequence is a
port of that BSD-2-Clause reference code.

## Errors and confirmation

- Confirmation remains at the application and CLI layer: `--yes` plus the exact
  `--confirm-uid`. The workflow also requires the preflight token and a fresh
  card/block-0 fingerprint.
- A normal card produces a `CodeUnsupported` error whose message names the
  failed Gen1A unlock step. It is not reported as a successful write.
- Device disconnection, cancellation, authentication failure, and verification
  failure keep their existing stable categories.

## Tests

Hardware-free:

- CRC-A vectors and exact Gen1A frame sequence replay with a fake raw
  transport;
- normal-card failure at `0x40`;
- uncertain write response still verified by the caller;
- raw mode is restored on success and failure;
- workflow Gen1A path keeps the same reader open (`openCount` unchanged,
  `closeCount` unchanged) and verifies UID plus full block 0;
- shim/native binding tests for raw mode and transceive result validation.

Explicit hardware:

- `NFCX_WORKFLOW_HARDWARE=1`, `NFCX_HARDWARE_GEN1A_CONFIRM=I_OWN_THIS_CARD`,
  `LIBNFC_DEVICE`, `NFCX_TEST_UID`, and `NFCX_TEST_KEY_A`;
- writes a new 4-byte UID, verifies reselection and block 0, then restores the
  original block 0 with the same Gen1A path and verifies the restore.

## Non-goals

- No format-all command.
- No irreversible UFUID/FUID lock command. DirectWrite variants are covered by
  Phase 04b.
- No non-4-byte UID handling.
- No card-family logic in libnfc-ng.
