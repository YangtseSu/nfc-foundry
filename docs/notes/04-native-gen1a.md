# Phase 04 implementation note: native Go Gen1A

Date: 2026-11-06

## Implemented

- Added `nfc.RawTransceiver` with `SetRawMode`, `TransceiveBytes`, and
  `TransceiveBits`. `libnfc.Reader` and the mock implement it; the
  `!libnfc` stub reports the capability as unsupported.
- Extended the C shim with raw-mode property control and bounded raw
  transceive calls. Enabling raw mode disables CRC and easy framing while
  leaving PN53x parity generation active. Disabling it restores the ordinary
  properties, forgets the selected target/authentication state, and
  power-cycles the RF field so the next anti-collision sees a rewritten UID.
- Added `internal/mifare/gen1a.go`: CRC-A, HALT, the 7-bit `0x40` and `0x43`
  unlock steps, and the MIFARE write command plus complete 16-byte block 0.
  Invalid BCC, non-4-byte UID, and non-Classic-1K input are rejected before
  raw mode is entered. The port retains the nfc-mfsetuid BSD-2-Clause notice.
- Reworked `workflow.UIDService.Write` so the ordinary CUID/Gen2 write and the
  Gen1A fallback run inside the same `WithReader` lease. The workflow backs up
  block 0 once, reselects the planned card before the raw sequence, leaves raw
  mode on every exit path, then reselects, authenticates sector 0 with the
  original key, and requires an exact block-0 readback plus valid BCC.
- An uncertain Gen1A data-frame response is not treated as a failure by
  itself. The reselect/readback result is authoritative, matching the existing
  treatment of an uncertain ordinary manufacturer write.
- Removed the `nfc-mfsetuid` engine adapter and tests, the runtime manifest
  component, the diagnostic check, the external fallback interface, and the
  terminal help lines that advertised the executable.
- Added `Makefile` target `libnfc-gen1a-hardware-test` and
  `TestHardwareGen1AUIDWrite`. The test is skipped unless
  `NFCX_WORKFLOW_HARDWARE=1` and
  `NFCX_HARDWARE_GEN1A_CONFIRM=I_OWN_THIS_CARD`; it writes
  `NFCX_TEST_UID`, verifies the new identity and block 0, then restores the
  original block 0 through the same in-process Gen1A path and verifies the
  restore.

## Verification

- `make check` passes: `go test ./...`, `go vet ./...`, MyGo i18n check, UI
  tests, and `reuse lint` (BSD-2-Clause, GPL-2.0-or-later,
  GPL-3.0-or-later, LGPL-3.0-or-later, and MIT are all declared).
- `CGO_ENABLED=1 go test -tags libnfc ./...` passes against the installed
  `libnfc-ng` 1.8.0.
- Built `nfc-foundry` and `nfctl` with `-tags libnfc`; both link
  `libnfcng.so.6`.
- Both hardware-tagged packages compile with
  `-tags "libnfc libnfc_hardware"`.
- `TestHardwareGen1AUIDWrite` passed on the attached ACR122U through
  `acr122_usb:001:005`: UID `FA B9 CF F4` was written to `12 34 56 78`,
  verified by reselection and a full block 0 readback, then restored to
  `FA B9 CF F4` and verified again on the same open reader. Backups were
  written under `$XDG_CONFIG_HOME/NFC Foundry/uid-backups/`.
- The previous hardware paths also passed on the same reader and card:
  `TestHardwareMIFAREClassicKnownKeyIO` (write acceptance skipped),
  `TestHardwareMIFAREClassicDumpRestore` with an actual full restore
  (complete 64-block dump, 32/32 verified key slots), CLI `classic keyscan`,
  CLI `classic dump` (complete image plus sidecar), CLI `classic restore`
  (all 63 non-manufacturer blocks written and verified), and CLI `uid set` in
  both directions with verified readback.
- `nfctl diag` on the host reports MFOC, MFCUK, and MFOC-Hardnested only; no
  `nfc-mfsetuid` check is present.

## libnfc-ng

No libnfc-ng change was required. The fork remains clean at the pinned commit
`c1eb612d8f6032179e6994439802db1fdca1221a` (`v1.8.0-ng.1`). The 1.8.0 API
already provides `nfc_initiator_transceive_bits`,
`nfc_initiator_transceive_bytes`, `NP_HANDLE_CRC`, `NP_EASY_FRAMING`, and
`NP_AUTO_ISO14443_4`; the new code uses those through the existing shim. The
installed `libnfc-ng` 1.8.0 linked and ran the hardware test successfully.

## Hardware command

```sh
CGO_ENABLED=1 \
LIBNFC_DEVICE=acr122_usb:001:005 \
LIBNFC_AUTO_SCAN=false \
LIBNFC_INTRUSIVE_SCAN=false \
NFCX_WORKFLOW_HARDWARE=1 \
NFCX_HARDWARE_GEN1A_CONFIRM=I_OWN_THIS_CARD \
NFCX_TEST_UID=12345678 \
NFCX_TEST_UID_BACKUP_DIR=/tmp/nfc-foundry-gen1a-uid-backup \
NFCX_TEST_KEY_A=FFFFFFFFFFFF \
go test -v -count=1 -timeout=10m \
  -tags "libnfc libnfc_hardware" \
  -run '^TestHardwareGen1AUIDWrite$' ./internal/workflow
```

The Makefile target accepts `pn532_uart:`, `pn53x_usb:`, and `acr122_usb:`
connstrings and requires the same confirmation and backup variables.

## Problems encountered

- libnfc's 4-bit response alignment is not guaranteed by the public API. The
  ACK check accepts both `0x0A` and `0xA0` nibble alignment, while
  `TransceiveBits` still requires exactly four response bits.
- A HALT command has no response by design. The Gen1A protocol treats a
  timeout or missing target during HALT as expected and uses the `0x40`
  response as the capability decision instead of the high-level presence
  classification used by ordinary MIFARE commands.
- The read-only 100-cycle ACR122U USB lifecycle test failed at iteration 3
  with the transient PN532 init status-word behavior already recorded for
  this reader. The same transient failure appeared on the first open of two
  CLI/test runs; retrying the process succeeded. This is existing ACR122U USB
  behavior, not caused by the Gen1A shim.

## Deviations from the spec

- None. `nfc-mfsetuid` is no longer a runtime, diagnostic, or manifest
  component; only its BSD-2-Clause notice remains.

## Blockers and follow-up

None. The Gen1A hardware write, restore, and readback verification passed on
the ACR122U. PN532 UART hardware validation for Gen1A remains desirable but is
not required to ship the in-process implementation, because the raw-frame path
uses the same PN53x chip functions.
