# Phase 29 S4: GUI Connect and Disconnect

Status: complete 2026-11-14.

## Goal

Give the reader rail explicit Connect and Disconnect controls instead of
letting reader changes and background polling own the connection implicitly.
Disconnect must actually release the reader and its cross-process lock while
keeping the inspected bytes readable.

## Lifecycle

`nfc_app::reader::ReaderStatus` gains `Disconnected`. It is distinct from
`Connecting`, `Polling`, `Busy`, `Recovering`, and `Error`, and its stable
label is `disconnected`.

`DeviceManager` records the host intent:

- `disconnect` closes the reader, releases the cross-process lock, clears the
  card and capability facts, and moves the monitor to `Disconnected`.
- `connect` leaves `Disconnected`, closes any remaining reader, and resets the
  monitor to `Connecting`; the next poll or operation opens the selected
  reader and reacquires the lock.
- `begin_operation` leaves `Disconnected` before opening, so `Read card` also
  reconnects.
- `poll` returns the `Disconnected` snapshot without opening or touching the
  reader.
- Selecting another reader and refreshing the device list preserve
  `Disconnected`; only Connect, retry, or an operation resumes polling.

## GUI

The reader rail keeps the reader dropdown, refresh, selected connstring,
status, card line, and capability line, and adds `Connect` and `Disconnect`
buttons.

- `Disconnect` is enabled for a selected reader that is not disconnected and
  while no operation is busy.
- `Connect` is enabled only in `Disconnected` with a selected reader and while
  no operation is busy.
- Disconnect stops polling, closes the reader, releases the lock, clears the
  live flag, resets Advanced mode, and disables the write commit. Inspected
  bytes stay on screen with `Inspection only · read the card again` or, with no
  image, `Reader disconnected`. Editing the inspected image stays available;
  writing it requires a new live read.
- Connect closes any stale reader state, resets the monitor, polls once, and
  reports `Reader connected`, `Reader recovering`, or `Reader error`. The
  previous image stays inspection-only until a new card read.
- `Read card` from `Disconnected` reconnects through the service operation
  path. The read path, not the button, owns the wait for the reader.

## Gating

No new card path or write gate is introduced. Live-image invalidation and the
existing service write preconditions remain the gate; a disconnect cannot
leave a live image behind.

## Tests

- `nfc-app` device test: disconnect closes the reader, releases the lock,
  reports `Disconnected`, and a poll does not reopen; connect reopens and
  reacquires the lock.
- `nfc-app` device test: an operation after disconnect reopens the reader.
- `nfc-app` device test: refresh and selection change keep the monitor
  disconnected; Connect opens the newly selected reader.
- `nfc-app` service test: disconnect stops polling and the next scan
  reconnects.
- GTK scenario: Disconnect updates the status, disables the write commit,
  releases the reader, and repeated polls do not reopen it. Connect returns to
  `Polling · card present` and the stale image stays inspection-only.
