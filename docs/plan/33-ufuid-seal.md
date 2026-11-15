# Phase 33 plan: UFUID support

Status: implemented 2026-11-15 on PN532 UART. UID change/restore passed on
the owned UFUID, and the irreversible seal command was applied; the card is
now permanently sealed. The first automated run returned a false negative
because the immediate post-seal probes still saw the backdoor. Verification now
polls for up to five seconds for two consecutive negative probes, covered by a
delayed-probe test. A clean end-to-end rerun is impossible because the card
cannot be unsealed.

## Goal

Make an unsealed UFUID workbench card usable through the existing Classic
services:

- change the UID repeatedly through the Gen1A protected writer;
- back up sector 0 before the change and verify the complete manufacturer
  block, BCC, reselection, and trailer;
- seal the card permanently so it becomes an ordinary MIFARE Classic 1K;
- expose the same operations and gates in `nfctl` and the GTK workbench.

An unsealed UFUID is a Gen1A-compatible UID card. There is no non-destructive
way to distinguish it from another Gen1A magic card, so the seal path proves
the backdoor is present before offering a plan and proves the backdoor is gone
after execution.

## Protocol evidence

The seal sequence is the Gen1A unlock followed by the UFUID configuration
write:

```text
HALT
40(7) -> 0A
43    -> 0A
E1 00 + CRC_A -> 0A
85 00 00 00 00 00 00 00 00 00 00 00 00 00 00 08 + CRC_A -> 0A
```

The configuration starts with `85 00` (magic wakeup disabled) and ends with
SAK `08`. It is the only configuration the product will write.

## Safety

| Gate | Behavior |
|---|---|
| Reader capability | `manufacturer_seal` and a `ManufacturerSealer` implementation are required |
| Card | four-byte UID MIFARE Classic 1K only |
| Advanced mode | required |
| Plan | one-shot token, 120-second expiry, current card identity, complete sector-0 fingerprint |
| Preflight | Gen1A probe must answer; sector 0 must authenticate, read, and have a valid BCC |
| Backup | complete sector 0 is written before the seal attempt |
| Execution | fresh Gen1A probe and sector-0 fingerprint check before the backup |
| Verification | reselect, require the same UID/ATQA/SAK, re-read sector 0 byte for byte, then require the Gen1A probe to stop answering |
| Failure | refused command or still-open backdoor reports "unchanged"; a changed card or unreadable verification reports an uncertain outcome with the backup path |

The seal is irreversible. The GUI uses a destructive confirmation dialog and
the CLI requires `--advanced` and confirmation (`--yes` for non-interactive
use).

## Slices

### 33.1 Transport and driver

- `ManufacturerSealer` on `nfc-transport` with `probe_gen1a` and
  `seal_manufacturer_card`.
- `ReaderCapabilities::manufacturer_seal`.
- PN532 UART, ACR122U USB, and ACR122U PC/SC advertise and forward the
  capability through the shared ACR reader.
- Shared `nfc-drivers::gen1a` owns the probe, seal sequence, CRC_A, and the
  canonical configuration.
- The mock reader scripts probe, seal, refused seals, ignored seals, and lost
  acknowledgements.

### 33.2 Application workflow

- `nfc-app::classic::ufuid` owns the preflight plan, one-shot execution token,
  backup record, and verification.
- `Service::classic_seal_preflight` and `classic_seal_execute`.
- A workbench image stays valid because sealing changes configuration, not
  sector data.

### 33.3 CLI and GUI

- `nfctl classic seal --advanced [--dry-run]`.
- GTK Classic Blocks inspector: Advanced mode → Seal UFUID… → plan dialog →
  destructive confirmation.
- The GUI disables Advanced mode after a verified seal so a second seal is not
  offered.

### 33.4 Hardware

- UID change/restore on the owned UFUID through the guarded service.
- Seal preflight and dry run on the owned UFUID.
- The irreversible seal test is environment-gated by
  `NFC_FOUNDRY_HARDWARE_UFUID_SEAL_CONFIRM=yes` and is run only with owner
  approval. It was run on 2026-11-15; the seal applied, the immediate
  verification false negative was fixed, and the card is sealed.

## Acceptance

- Unit tests cover the probe, both ACK positions, the seal frames, CRC vectors,
  canonical configuration rejection, and capability gating.
- Workflow tests cover success, no Gen1A, ignored seal, refused seal, lost
  acknowledgement, missing capability, and token reuse/expiry.
- CLI mock tests cover the Advanced and confirmation gates, dry run, and a
  verified seal.
- GTK scenario covers the plan cancel and confirm paths.
- Hardware: UID change and restore pass on the owned UFUID; a seal dry run
  prints the exact block 0, trailer, and configuration without writing; the
  seal command was applied and the resulting sealed card was read back.
- Required checks: `cargo fmt --check`, workspace Clippy with `-D warnings`,
  workspace tests, GTK tests, and `reuse lint`.
