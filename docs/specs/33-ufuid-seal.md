# Spec: UFUID support

Status: implemented 2026-11-15; PN532 UART UID change/restore passed and the
seal command was applied to the owned UFUID, which is now permanently sealed.
The immediate post-seal verification returned a false negative; it now polls
for a stable negative probe. A clean automated rerun is impossible because
the card cannot be unsealed. Plan: [33-ufuid-seal.md](../plan/33-ufuid-seal.md).

## Problem

A UFUID is an unsealed Gen1A UID card that can be made permanent by writing a
vendor configuration block. The product already changed Gen1A block 0, but it
had no UFUID-specific capability, no probe, no seal, and no way to prove that
the backdoor stopped answering. Treating the seal as an ordinary config write
would expose an irreversible action without a backup or verification.

## Scope

Supported:

- Four-byte UID MIFARE Classic 1K UFUID cards.
- Repeated UID changes through the existing guarded Gen1A writer.
- A read-only Gen1A probe used by the seal preflight.
- The canonical UFUID seal configuration:
  `85 00 00 00 00 00 00 00 00 00 00 00 00 00 00 08`.
- `nfctl classic seal` and the GTK Seal UFUID action.

Not supported:

- FUID one-time-write semantics as a separate card type.
- Gen3/Gen4 configuration commands, hidden blocks, or CUID-mode conversion.
- Five- or seven-byte UIDs, Classic 4K, or a bulk format.
- Reversing a seal.

## Transport

`ReaderCapabilities::manufacturer_seal` fails closed. A reader exposes
`Reader::manufacturer_sealer()` only when it implements the full sequence.

`ManufacturerSealer`:

- `probe_gen1a()` sends HALT, `40(7)`, `43`, returns `true` only for two ACKs,
  and reselects the card. It does not write.
- `seal_manufacturer_card(config)` accepts only
  `nfc_transport::UFUID_SEAL_CONFIG`, unlocks the backdoor, sends
  `E1 00` + CRC_A, waits for ACK, sends the 16-byte configuration + CRC_A, and
  waits for ACK. It leaves the card selected; the caller verifies.

The PN532 UART and both ACR122U links forward these operations and enter raw
mode for the sequence. The shared `nfc-drivers::gen1a` module owns the frames.

## Preflight

`ClassicSealPreflight` requires Advanced mode and a reader with the seal
capability. It:

1. Checks the expected card identity.
2. Requires a four-byte UID MIFARE Classic 1K.
3. Probes the Gen1A backdoor. A card that does not answer is refused before a
   backup or write.
4. Authenticates sector 0 with the key store and common keys.
5. Reads blocks 0..3 and requires block 0's UID to match the selected card and
   its BCC to be valid.
6. Returns a plan with the token, card, UID, block 0, trailer, sector-0 key,
   canonical configuration, warnings, 120-second expiry, and a fingerprint of
   the four sector-0 blocks plus card identity.

## Execution

`ClassicSealExecution` consumes the one-shot token and requires Advanced mode
and confirmation. Before the backup it rechecks the card identity, probes the
backdoor again, authenticates sector 0 with the planned key, and requires the
fingerprint to match.

It writes a project backup of sector 0 and a `UfuidSealBackupRecord` containing
block 0, the trailer, the UID, and the configuration. It then sends the seal
command. Every attempt is verified even when the command reports an error:

- reselect and require the same UID, ATQA, and SAK;
- authenticate and read blocks 0..3, requiring the pre-seal fingerprint;
- poll the Gen1A probe for up to five seconds and require two consecutive
  negative results. The backdoor can still answer immediately after the write acknowledgement;
  a single probe is not authoritative.

Outcomes:

- Sealed and sector 0 unchanged → success. A lost acknowledgement is a warning,
  not a failure.
- Backdoor still answers through the settle window and sector 0 is unchanged →
  error "unchanged"; the command did not seal the card.
- Card identity or sector 0 changed → uncertain error with the backup path.
- Verification read or probe failed → uncertain error with the backup path.

Cancellation before the command stops the workflow. Once the command may have
run, cancellation is ignored for verification.

## CLI

`nfctl classic seal --advanced [--dry-run]`.

- `--advanced` is required in every mode.
- `--dry-run` prints the UID, sector-0 key, block 0, trailer, configuration,
  and warnings; it writes no backup and no card data.
- Execution requires confirmation; `--yes` is the non-interactive form.
- Success prints the verified UID, block 0, configuration, and backup path.

## GUI

The Classic Blocks inspector has an Advanced-mode switch, Write UID…, and Seal
UFUID…. Seal UFUID is sensitive only for a live four-byte UID Classic 1K
image with Advanced mode on.

The first step is a read-only preflight. The plan dialog shows the UID, sector-0
key, block 0, trailer, configuration, the permanent loss of the backdoor, and
the backup/verification plan. The destructive confirmation starts the same
service method as `nfctl`. A verified seal disables Advanced mode and leaves
the current image valid because sector data did not change. A failure re-reads
the card.

## Safety

- Only the canonical seal configuration is accepted, in both the app and the
  driver.
- The seal cannot run without a successful Gen1A probe.
- The backup and fingerprint are bound before the irreversible command.
- The service never reports success from the command acknowledgement alone;
  the removable backdoor and unchanged sector 0 are the success criteria.
- A verified seal clears pending UID and seal plans for the card.
- The card is never written when the preflight fails.

## Tests

- `nfc-drivers::gen1a`: CRC_A, ACK nibbles, probe true/false, seal frame order,
  canonical configuration rejection.
- `nfc-transport`: capability gating, probe, seal, ignored seal, already sealed.
- `nfc-app`: success, no Gen1A, refused command, ignored command, lost
  acknowledgement, missing capability, token reuse and expiry.
- `nfctl`: Advanced gate, confirmation gate, dry run, verified seal, backup.
- GTK: plan cancel, destructive confirm, seal state, disabled Advanced mode,
  backup count.
- Hardware-gated: UID change and restore on the owned UFUID; dry-run seal plan;
  an environment-gated irreversible seal test.

## Required checks

`cargo fmt --all --check`, workspace Clippy with `-D warnings`, workspace
tests, gated GTK tests, and `reuse lint`.
