# Note: 29.2 cross-process reader lock

Plan: [29-cli-reader-ownership.md](../plan/29-cli-reader-ownership.md). Spec:
[29b-reader-lock.md](../specs/29b-reader-lock.md). Contract:
[cli.md](../cli.md).

## Implemented

- `crates/nfc-core/src/error.rs`: `NfcError::ReaderBusy(String)`. The CLI maps
  it to `reader_busy` and exit 4; other surfaces print its message.
- `crates/nfc-transport/src/device.rs`: `DeviceInfo::physical_id:
  Option<String>` and `with_physical_id`. `new` keeps `None`, which marks
  scripted transports.
- `crates/nfc-app/src/reader_lock.rs`:
  - `ReaderLock` owns an open lock file and a non-blocking exclusive `flock`;
  - lock root is `$XDG_RUNTIME_DIR/nfc-foundry`, or
    `$TMPDIR/nfc-foundry-<uid>` without a runtime directory;
  - root mode `0700`, file mode `0600`;
  - stable file names use a sanitized prefix plus FNV-1a 64 suffix;
  - holder `pid`/`program`/`reader` is written after acquisition and read for
    contention messages;
  - `flock` comes from `rustix` so the workspace keeps `unsafe_code = "deny"`.
- `crates/nfc-app/src/device.rs`:
  - `DeviceManager` holds `Option<ReaderLock>` for the open connection;
  - `ensure_open` acquires before the transport or reader opens;
  - release, selection change, refresh removal, and transient close drop the
    lock; monitor open failure records `ReaderBusy` as a fatal `Error`.
- Drivers:
  - PN532 UART: by-id symlink preferred, otherwise canonical path;
  - ACR122U USB: USB serial number preferred, otherwise bus/address;
  - ACR122U PC/SC: reader name.
- `nfctl`: typed `reader_busy` classification; mock devices carry a physical
  identity so the binary exercises the lock in every scripted run. CLI test
  sandboxes now set `XDG_RUNTIME_DIR`.

## Behavior

- Contention occurs before `Reader::open`, so the losing process never claims
  the serial port, the CCID interface, or the PC/SC card.
- The lock file is permanent; kernel lock state is the only authority. A
  process exit or crash releases the lock without cleanup.
- Locking is advisory for cooperating NFC Foundry processes. Another program
  that opens the device directly is caught by the existing OS or `pcscd`
  error, not by this lock.
- The monitor does not spin on a busy reader: `ReaderBusy` is fatal until the
  reader is refreshed or re-selected.

## Limitations

- ACR122U direct USB and PC/SC may be the same physical reader, but PC/SC
  exposes no USB serial or bus address to correlate them. The two paths keep
  separate lock keys; the OS claim and actionable busy errors remain the
  backstop. Two processes using the *same* transport path are fully serialized.
- Only enumerated devices lock. A connstring outside the device list is
  already refused by selection.
- The CLI lock error for an unwritable runtime directory is `io`/1; only
  contention is `reader_busy`/4.

## Tested

2026-11-14:

- `cargo fmt --all --check`.
- `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo test --workspace`: 771 passed, 0 failed.
- `cargo test -p nfc-app --lib`: 80 tests pass, including lock contention,
  stale-file reuse, permissions, stable names, and `DeviceManager` claiming.
- `cargo test -p nfc-drivers --lib`: 72 tests pass, including PN532 by-id
  selection and ACR122U serial identity.
- `cargo test -p nfctl --test cli_contract a_second_process`: two processes
  contend; the loser exits 4 with `reader_busy`; SIGTERM release leaves the
  reader usable.
- `reuse lint`: 688/688 files compliant.
