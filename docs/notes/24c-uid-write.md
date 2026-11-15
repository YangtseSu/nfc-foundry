# Note: phase 24c standalone UID / block-0 write

Status: complete 2026-11-11; PN532 UART and ACR122U hardware validated.
CUID/Gen2 ordinary backend validated on PN532 UART 2026-11-15. Plan:
[24-parity-recovery.md](../plan/24-parity-recovery.md). Spec:
[24c-uid-write.md](../specs/24c-uid-write.md).

## Problem

The application refused every manufacturer-block write. The ACR122U driver had
a hardware-validated Gen1A sequence, but the PN532 driver had none, and there
was no plan, backup, reselect, or verification around either, plus no ordinary
CUID/Gen2 path. A UID change can make the old card identity and workbench
stale, so it cannot share the ordinary block-write path.

## Implemented

### Transport capability

- `Reader::manufacturer_writer` returns the protected writer when a driver has
  one. The default is `None`.
- `Acr122Usb` and `Pn532Uart` return themselves; `MockReader` returns itself
  when its scripted Gen1A capability is enabled. The mock defaults to a magic card: the Gen1A
  writer is available, an ordinary block-0 write is accepted, and a block-0
  write updates the scripted card UID so a reselect observes it.
- `MockCall::ManufacturerWrite`, `MockFault::WriteAppliedError`,
  `set_manufacturer_write`, and `set_block_zero_writable` script the protected
  path, lost acknowledgements, and immutable cards.
- `CrcA` and the `0x40`/`0x43` unlock sequence live in one reader-neutral
  `nfc-drivers::gen1a` module. The ACR122U and PN532 implementations only
  provide raw byte/bit exchanges and reselect/raw-mode transitions.

### UID workflow

`nfc-app::classic::uid` owns the request, plan, execution, and result types.

- Preflight requires Advanced mode and a four-byte UID Classic 1K. It
  authenticates sector 0, reads blocks 0..3, requires the block-0 UID to match
  the selected card, requires a valid current BCC, decodes the sector-0 trailer
  access bits, and builds the new block 0 from the target UID, its BCC, and the
  preserved tail.
- Backends are ordered Gen1A first when the reader exposes a protected writer,
  then the ordinary write when the access bits permit one. At least one backend
  must exist. The plan carries the authenticated sector-0 key, access key
  types, backends, warnings, a 120-second expiry, and an FNV fingerprint of the
  four sector-0 blocks plus the card identity.
- Execution consumes the one-shot token, rechecks Advanced mode and
  confirmation, the card identity, and a fresh sector-0 read against the plan
  fingerprint, then writes a project backup of sector 0 and a
  `UidBackupRecord` with the old and new block 0 and both UIDs.
- The plan stores the read key and the ordinary-write key separately. The
  ordinary backend is offered only when a candidate key of an access-allowed
  type authenticates, and execution re-authenticates with that key before the
  write. Without this, a card that authenticated with Key A but permitted only
  Key B writes would fail after the backup.
- Every attempt is followed by a reselect, authentication, and a read of blocks
  0 and 3. This post-write path deliberately bypasses cancellation checks: a
  cancel must not hide whether the card changed.
  - full planned block 0, target UID, valid BCC, and unchanged trailer →
    success;
  - old UID and old block 0 and old trailer → unchanged, try the next backend;
  - anything else → uncertain failure with the backup path.
- A write error that the read-back shows as applied is success with an
  "acknowledgement was lost" warning. A verification read failure is an
  uncertain outcome, never a claimed refusal.
- `Service::classic_uid_preflight` stores the plan under the shared one-shot
  token counter. `Service::classic_uid_execute` removes the token before I/O,
  refuses expiry, and clears a workbench that belonged to the old card
  identity after a changed UID.

### CLI

`nfctl classic uid --uid <UID> --advanced [--dry-run]`.

- `--uid` accepts compact hex, spaces, colons, or dashes.
- The plan prints current/target UID, sector-0 key, access key types, backend
  order with notes, and old/new block 0.
- Execution requires confirmation; `--yes` is the non-interactive form.
  `--advanced` is always required.
- Success prints the old and new UID, the backend, the full new block 0, the
  BCC/reselect statement, and the backup path. An unchanged target exits
  without a backup.

### GUI

- The Blocks inspector has a session Advanced-mode switch and a separate target
  UID entry plus "Write UID…" destructive action. The controls are sensitive
  only for a live four-byte UID Classic 1K image.
- The target dialog rejects an invalid or unchanged UID. The preflight plan is
  shown in a destructive confirmation dialog.
- Execution uses the same service methods as `nfctl`. A changed UID invalidates
  the live image and workbench and triggers a full re-read, so the header,
  block table, keys, and NDEF state reflect the new identity. Any execution
  error also invalidates the image, because the outcome may be unknown.
- Advanced mode resets on card read, reader change, reader refresh, UID change,
  and application restart. New strings are in the English and `zh-CN`
  catalogs.
- The phrase-typing gate was removed from every GUI mutation flow.
  Type 2 Advanced pages, locks, password configuration, restore, NDEF format,
  Classic restores, and the UID plan now use one destructive confirmation
  dialog. The CLI uses the existing `--yes` confirmation.

## Tests

- `nfc-drivers` unit tests: CRC_A vector, ACK nibble positions, and
  card/BCC validation. The existing scripted ACR122U Gen1A sequence still runs
  through the shared module.
- `nfc-app` unit tests: UID text parsing and backend labels.
- `nfc-app` workflow tests: ordinary CUID success with backup and reselect
  evidence, Gen1A when access bits deny a standard write, Gen1A-to-standard
  fallback, unchanged UID, wrong key, broken access bits, broken BCC, ignored
  write, lost acknowledgement confirmed by read-back, card disappearance,
  card change, cancellation, immutable card, and token reuse.
- `nfc-app` service test: expired and unknown UID tokens are refused before
  I/O.
- `nfctl` mock tests: Advanced gate, confirmation gate, invalid UID, dry run,
  successful write, and an unchanged target.
- Gated GTK scenario: Advanced switch, unchanged-target refusal, plan cancel,
  plan confirmation, write, and automatic re-read.
- Environment-gated `nfctl` hardware test changes the UID and restores the
  original block 0 on an owned magic card. It passed on PN532 UART on
  2026-11-11 with a Gen1A card and on 2026-11-15 with a CUID/Gen2 card.

## Commands run

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
NFC_FOUNDRY_GUI_TESTS=1 xvfb-run -a cargo test -p nfc-foundry gtk_workbench_scenarios
reuse lint
```

Hardware, 2026-11-11, PN532 UART `pn532_uart:/dev/ttyUSB0`, Gen1A magic
Classic 1K UID `FA B9 CF F4`, sector-0 Key A `FFFFFFFFFFFF`:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
NFC_FOUNDRY_HARDWARE_KEY=FFFFFFFFFFFF \
NFC_FOUNDRY_HARDWARE_KEY_TYPE=A \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_UID_CONFIRM=yes \
NFC_FOUNDRY_HARDWARE_BACKUP_DIR=/tmp/nfc-uid-backups \
cargo test -p nfctl --features hardware-write --test hardware_classic \
  -- --ignored changes_and_restores_the_card_uid --nocapture
```

The run selected `[Gen1a, Standard]`, wrote UID `FA B9 CF F5` through Gen1A,
reselect verified the complete block 0, BCC, and trailer, then restored UID
`FA B9 CF F4` and the original block 0. The read-only PN532 suite also passed
(`classic_1k`, 15/16 sectors readable, 15 verified keys) after the driver
change.

Hardware, 2026-11-15, PN532 UART `pn532_uart:/dev/ttyUSB0`, CUID/Gen2
Classic 1K UID `6A 31 5E 28`, sector-0 Key B `FFFFFFFFFFFF`:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
NFC_FOUNDRY_HARDWARE_KEY=FFFFFFFFFFFF \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_UID_CONFIRM=yes \
NFC_FOUNDRY_HARDWARE_BACKUP_DIR=/tmp/nfc-uid-cuid \
cargo test -p nfctl --features hardware-write --test hardware_classic \
  -- --ignored changes_and_restores_the_card_uid --nocapture
```

The plan listed `[Gen1a, Standard]`. The card refused the Gen1A unlock, so the
ordinary backend changed `6A 31 5E 28` to `6A 31 5E 29`, reselected, verified
the complete block 0 and BCC, and restored the original UID and block 0. The
gated test passed. A `classic dump` also read 64/64 blocks with the sector-0
Key B.

The `hardware-write` feature build was also checked:

```sh
cargo test -p nfctl --features hardware-write --test hardware_classic --no-run
```

That build exposed a pre-existing type error in the temporary data-block write
test: it treated `ClassicBlockReadResult` as `[u8; 16]`. The test now uses the
result's `.data` field, so the hardware suite compiles again.

## Decisions and problems

- Gen1A is tried first on both readers because it does not depend on access
  bits. The ordinary write remains the fallback for CUID/Gen2 cards.
- The PN532 Gen1A path reuses the ACR122U sequence rather than duplicating it.
  The driver only adds the reselect/raw-mode wrapper and the raw exchanges.
  The first PN532 attempt against the same card failed before this port because
  the standard block-0 write timed out and correctly left the card unchanged.
- The preflight cannot prove that a card is magic without attempting a write.
  An ordinary immutable card is therefore refused by the card itself: the
  Gen1A unlock fails, the ordinary write is not acknowledged, and the reselect
  read confirms the old block 0. The service reports "the card was not
  changed"; it never claims the UID changed.
- Backup is sector 0 only, not a full card. Only sector 0 is read and only
  block 0 changes, so a full-card read would add time without adding recovery
  data. The project backup preserves block 0, the trailer, and the two data
  blocks.
- Verification uses the preflight sector-0 key directly. A changed key between
  plan and execution makes the verification read fail, which is reported as an
  uncertain outcome with the backup path.
- The service clears the workbench only when its card identity matches the card
  that was changed. A file-backed workbench for another card is untouched.
- The PN532 UART service path is now validated. On 2026-11-11 the same test
  passed on `acr122_usb:001:002` with a Gen1A magic Classic 1K: it selected
  Gen1A, changed `FA B9 CF F4` to `FA B9 CF F5`, reselected and verified block
  0/BCC, then restored the original UID. The ACR122U service path is now
  validated. On 2026-11-15 the ordinary CUID/Gen2 backend passed the same test
  on PN532 UART with card `6A 31 5E 28`; the Gen1A probe was refused and
  `Standard` changed and restored block 0.

## Next

S3b (restore with UID) is complete on both readers; see
[24d-restore-with-uid.md](24d-restore-with-uid.md). It reuses the UID plan,
backup, and post-write verification but runs the UID write only after the dump
restore and its full read-back have succeeded.
