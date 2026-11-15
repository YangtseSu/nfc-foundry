# Note: UFUID support

Status: implemented 2026-11-15 on PN532 UART. UID change/restore passed and
the seal command was applied to the owned UFUID; the card is now permanently
sealed. The first automated run returned a false negative because the immediate
probes still saw the backdoor; the workflow now polls for two consecutive
negative probes. A clean automated rerun is impossible because the card cannot
be unsealed. Plan: [33-ufuid-seal.md](../plan/33-ufuid-seal.md). Spec:
[33-ufuid-seal.md](../specs/33-ufuid-seal.md).

## Card

- UFUID, four-byte UID `47 D5 87 A9`, ATQA `00 04`, SAK `08`.
- Reader: PN532 UART (CH340), `pn532_uart:/dev/ttyUSB0`, firmware 1.6.
- All 16 sectors authenticate with Key A `FFFFFFFFFFFF`.
- Block 0: `47 D5 87 A9 BC 08 04 00 A0 A1 A2 A3 A4 A5 A6 A7`.
- Before sealing, the card answered the Gen1A `0x40(7)` / `0x43` unlock and
  the block-0 write.
- Backdoor read (`30 00`) and configuration read (`E0 00`) answer `05` or time
  out on this variant. The product does not depend on a config read.
- The reference `nfc-mfsetuid` confirmed the same unlock and write sequence on
  the same port. The card was restored to its original block 0 after the
  reference probe.

## Implemented

### Transport and drivers

- `ReaderCapabilities::manufacturer_seal` and the `ManufacturerSealer` trait
  with `probe_gen1a` and `seal_manufacturer_card`.
- `nfc_transport::UFUID_SEAL_CONFIG` is the single canonical configuration.
- PN532 UART, ACR122U USB, and ACR122U PC/SC advertise the capability. The ACR
  wrappers forward through the shared `Acr122Reader`.
- `nfc-drivers::gen1a` shares HALT/unlock, probe, block-0 write, seal, and CRC_A
  between both readers. The seal command is `E1 00` + CRC_A followed by the
  16-byte configuration + CRC_A.
- The mock reader scripts the probe, seal, ignored seal, applied-with-lost-ACK,
  already-sealed state, and missing capability.

### Application

- `nfc-app::classic::ufuid` owns the preflight plan, execution, backup record,
  and verification.
- Preflight requires Advanced mode, the reader capability, a four-byte UID
  Classic 1K, a successful Gen1A probe, authenticated sector 0, a matching
  card UID, and a valid BCC.
- Execution consumes a one-shot token, rechecks the card and a fresh sector-0
  fingerprint, writes a sector-0 project backup, sends the seal, then requires
  the same card identity, byte-identical sector 0, and two consecutive negative
  Gen1A probes within a five-second window.
- A lost acknowledgement with a verified sealed state is success with a
  warning. A refused or ignored seal with unchanged data is a failure that
  never claims a change. Any other state is uncertain and names the backup.
- The workbench image stays valid because sealing changes configuration, not
  block data. A verified seal clears pending UID and seal plans so a stale
  one-shot token cannot be used on the sealed card.

### CLI and GUI

- `nfctl classic seal --advanced [--dry-run]` uses the shared preflight and
  execution. Dry run prints the exact block 0, trailer, configuration, and
  warnings.
- The GTK Classic Blocks inspector adds a destructive Seal UFUID… action. The
  plan dialog shows the permanent loss of the backdoor; the confirmation uses
  the same service. A verified seal disables Advanced mode and logs the backup.
- New strings are in the English and `zh-CN` catalogs.

## Tests

- `nfc-drivers::gen1a`: canonical CRC_A vector, both ACK nibble positions,
  probe true/false, seal frame order, configuration rejection.
- `nfc-transport`: capability gating, successful seal, ignored seal, sealed
  probe.
- `nfc-app` workflow: success with backup and unchanged sector 0, no Gen1A,
  refused seal, ignored seal, lost acknowledgement, missing capability, token
  reuse, expired and unknown service tokens.
- `nfctl` mock: Advanced and `--yes` gates, dry run, verified seal, backup
  count.
- Gated GTK scenario: plan cancel, confirm, sealed state, disabled Advanced
  mode, backup count.
- Hardware: `changes_and_restores_the_card_uid` passed on `47 D5 87 A9` with
  backend Gen1a, changing to `47 D5 87 A8` and restoring the original block 0.
  `classic seal --advanced --dry-run` produced the plan on the same card.
- Hardware seal: `seals_a_ufuid_permanently` ran on 2026-11-15 and applied the
  seal. Its immediate verification reported "did not disable" while the card
  was still answering the backdoor; after the run the card no longer answered
  `40(7)`, a 64/64-block dump and block 0 were unchanged, and a new seal
  preflight was refused. The false negative led to the poll fix above.
- The delayed-probe workflow test sets a two-probe delay after the seal and
  requires the poll to recover. The refusal and ignored-seal tests still
  report "unchanged".

## Problems and decisions

- This UFUID does not answer backdoor block/config reads. The seal design must
  not require reading the configuration; the seal sequence only needs the E1
  write and a post-seal probe.
- The preflight cannot prove the card is a UFUID without attempting the
  irreversible command. It proves Gen1A and the product copy says a non-UFUID
  Gen1A card refuses the seal unchanged.
- The seal was run with owner approval after the implementation. The card is
  permanently sealed and is being discarded, so the immediate-verification
  false negative cannot be rerun end to end on this unit. The poll fix is
  covered by a delayed-probe workflow test; a clean run needs another UFUID.

## Commands run

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
NFC_FOUNDRY_HARDWARE_KEY=FFFFFFFFFFFF \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_UID_CONFIRM=yes \
NFC_FOUNDRY_HARDWARE_BACKUP_DIR=/tmp/nfc-ufuid-uid \
cargo test -p nfctl --features hardware-write --test hardware_classic \
  -- --ignored changes_and_restores_the_card_uid --nocapture
```

```sh
nfctl --device pn532_uart:/dev/ttyUSB0 --key FFFFFFFFFFFF \
  classic seal --advanced --dry-run
```

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
NFC_FOUNDRY_HARDWARE_KEY=FFFFFFFFFFFF \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_UFUID_SEAL_CONFIRM=yes \
NFC_FOUNDRY_HARDWARE_BACKUP_DIR=/tmp/nfc-ufuid-seal \
cargo test -p nfctl --features hardware-write --test hardware_classic \
  -- --ignored seals_a_ufuid_permanently --nocapture
```

The automated run applied the seal but reported
`the UFUID seal did not disable the Gen1A backdoor`. Afterward the card
returned `64/64` blocks, block 0 was unchanged, `classic seal --dry-run` was
refused because the Gen1A unlock no longer answered, and a direct probe timed
out. The poll fix is the result of that run.

## Next

- Run the seal test on another owned UFUID if one becomes available, to record
  a clean automated pass with the poll fix. The current card is sealed and
  scheduled for disposal.
