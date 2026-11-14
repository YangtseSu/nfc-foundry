# Spec: 29.2 cross-process reader lock

Status: implemented 2026-11-14.

## Goal

One physical reader has one owner across the GTK workbench and `nfctl`. A
second process must fail before it claims the device instead of racing the
first process for the serial port, USB interface, or PC/SC card handle.

## Ownership

`nfc-app` owns the lock; the GUI and CLI inherit it through `DeviceManager`.
A lock is per physical reader, not per user-facing connstring, so a reader
selected as `/dev/ttyUSB0` and as its `/dev/serial/by-id` alias still shares
one lock.

## Lock file

- Root: `$XDG_RUNTIME_DIR/nfc-foundry`.
- Fallback when `XDG_RUNTIME_DIR` is unset or empty:
  `$TMPDIR/nfc-foundry-<uid>`.
- Root directory mode `0700`; lock file mode `0600`.
- File name: `reader-<sanitized key>-<fnv1a64>.lock`. The suffix keeps
  distinct identities separate after path sanitization and is independent of
  the Rust toolchain.
- POSIX advisory `flock(LOCK_EX | LOCK_NB)` on an open descriptor, held while
  the reader connection is open.
- The file is never deleted on release, so a stale file cannot block anyone.
  Kernel lock state disappears with the process, including after a crash.
- The holder writes `pid`, `program`, and `reader` into the file after
  acquiring. A contention message reports the holder when that record is
  present.

## Physical identity

`DeviceInfo::physical_id` carries the lock identity. Drivers populate it
during enumeration; scripted transports leave it `None` and never take a host
lock.

| Transport | Identity |
|---|---|
| PN532 UART | `/dev/serial/by-id/*` entry when one resolves to the port, else the canonical device path, else the connstring path |
| ACR122U USB | `acr122_usb:serial:<serial>`; bus/address pair only when the reader reports no serial |
| ACR122U PC/SC | reader name |

Direct USB and PC/SC are separate transports for the same hardware. PC/SC
does not expose a USB serial or bus identity, so the two aliases keep separate
lock keys unless a future driver resolves a shared identity. The OS device
claim and the `pcscd`/busy errors remain the backstop; this limitation is
intentional and recorded in the phase note.

## Lifecycle

- Acquire immediately before `Transport::open` and `Reader::open`. A failure
  to acquire leaves the reader unopened and the device unclaimed.
- Hold while the connection is open.
- Release on explicit release, selection change, transient transport close,
  device removal, and application exit.
- A monitor that cannot acquire the lock reports `ReaderBusy` and enters the
  non-retrying `Error` state instead of retrying in a tight loop.

## Surfaces

- GUI: the reader panel shows `Error: reader busy: …`. Refresh or re-selecting
  the reader retries the claim. No write path is available without a live
  connection.
- CLI: `nfctl` exits 4 with `code: "reader_busy"`; the message names the
  reader, holder program, and holder PID when available.
- `NfcError::ReaderBusy` is a typed variant; no message text is classified.

## Tests

- `nfc-app` unit tests: exclusive contention, release, stale file reuse,
  directory/file permissions, stable file names, holder detail in the error.
- `nfc-app` device-manager test: another holder blocks the monitor before the
  reader opens; release frees the device; release also occurs on disconnect.
- `nfctl` integration test: two real processes contend for one mock reader;
  the loser exits 4 with `reader_busy`; after the holder exits, the reader is
  usable again.

## Non-goals

- A global device daemon.
- Locking readers that were never enumerated.
- Sharing one key between direct USB and PC/SC without a proven common
  identity.
