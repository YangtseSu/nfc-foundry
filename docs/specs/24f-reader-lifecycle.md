# Phase 24 S4: reader lifecycle and capability model

Status: complete 2026-11-12.

## Goal

Card presence and reader recovery are an application responsibility instead of
a GUI refresh button. `nfc-app` owns the monitor, `nfctl` and GTK consume the
same state, and drivers declare what they implement so UID and recovery gates
fail closed.

## Monitor model

`nfc_app::reader` defines the public lifecycle types:

- `ReaderStatus`: `Connecting`, `Polling`, `Busy`, `Recovering`, `Error`.
- `ReaderEvent`: `CardAppeared(CardInfo)`, `CardRemoved(CardInfo)`,
  `StatusChanged(ReaderStatus)`.
- `ReaderSnapshot`: current status, selected connstring, card, capabilities,
  and the last error text.
- `ReaderUpdate`: one snapshot plus the transitions since the previous step.
- `ReaderMonitorConfig`: poll interval, removal misses, reconnect initial delay,
  and reconnect maximum.

Defaults: 400 ms poll interval, two removal misses, 200 ms initial reconnect
delay, and a 5 s backoff cap. Removal and reconnect values are normalized to at
least one miss and 1 ms.

Behavior:

- A poll calls `card_info` on the selected reader and reports
  `CardAppeared` when no card was known or the identity changed. A changed
  identity reports the old card removed before the new card appears.
- A no-card answer increments the miss counter. The card is reported removed
  only after `removal_misses` consecutive misses; one transient miss keeps the
  known card and its live image.
- `Timeout` and `Io` failures close the reader, set `Recovering`, and schedule
  the next attempt with exponential backoff. A successful reconnect returns to
  `Polling`; a retained card is not re-announced.
- Other errors stop polling in `Error`. Selecting the same reader again,
  `Service::retry_reader`, or refreshing the device list starts a fresh attempt
  with the backoff reset.
- The monitor polls only while a reader is selected and polling is not paused.
  An explicit `Service::release_reader` parks the monitor; the next operation or
  selection opens the reader again.

`Service::poll_reader` steps the monitor with the wall clock.
`Service::poll_reader_at` accepts the host clock for deterministic schedulers
and tests. `Service::reader_state` returns the snapshot without polling.

## Exclusive lease

`DeviceManager::begin_operation` opens the reader if needed, sets the busy
status, selects the card once, and returns the existing `Operation` lease. The
lease clears the busy flag on drop. Polling takes `&mut DeviceManager`, and the
GUI and CLI wrap the service in one mutex, so a poll cannot share a reader with
an operation: the second locker waits until the operation ends. The monitor
never issues `card_info` while the busy flag is set.

## Capability model

`nfc_transport::ReaderCapabilities` now carries every operation a driver
implements:

- `classic_read`, `classic_write`
- `type2_read`, `type2_write`
- `raw_bytes`, `raw_bits`
- `manufacturer_write`
- `recovery_parity`

`Reader::capabilities` defaults to an empty profile. A driver that does not
override it fails closed. PN532 UART, ACR122U, the mock reader, and the test
readers populate the profile. `RecoveryCapabilities` and
`Reader::recovery_capabilities` were removed; `recovery_parity` is the single
source of truth.

Gates:

- Every UID entry point (`classic uid` preflight and execution, and restore
  with UID planning) refuses a reader without `manufacturer_write` before any
  card read or backup. The ordinary block-0 CUID/Gen2 path stays inside that
  gate; a driver without the capability cannot reach the UID service.
- Explicit active recovery strategies (`darkside`, `nested`, `staticnested`,
  `hardnested`, `fm11rf08s`) refuse a reader without `recovery_parity` before
  the engine starts. Auto recovery still starts with the dictionary and lets
  the engine report a missing parity path when it escalates.

The mock exposes `MockHandle::set_capabilities`; clearing `manufacturer_write`
also removes its `ManufacturerWriter` and makes the protected write fail.

## Surfaces

- `nfctl scan --watch` steps the monitor and prints card appearance, removal,
  `reader recovering`, and `reader error` transitions plus a changed error text.
  Normal commands still use the same lease, which updates the monitor card.
- The GTK reader rail shows the selected reader, lifecycle status, and the
  capability profile. A background thread steps the monitor; the GTK main
  context receives updates through an `async-channel`.
- A `CardRemoved` event, a different card appearing, or a `Recovering`/`Error`
  status invalidates the live flag, resets Advanced mode, and disables every
  write action. The inspected bytes stay on screen with an inspection-only
  status; a new read is required before any write.
- `refresh_devices` polls once so the panel reflects the connection immediately;
  tests construct the workbench with `auto_monitor: false` and call
  `poll_reader_once` for deterministic steps.

No new card path, backup format, or write gate is introduced by this slice.
Classic and Type 2 writes keep their existing confirmation, backup, identity,
and read-back requirements.

## Acceptance

- Monitor tests cover card appearance, removal after the configured misses, a
  transient timeout, reconnect with backoff, and the busy status.
- A service test proves a second locker cannot enter while an operation lease
  is held.
- A service test proves a reader without `manufacturer_write` is refused by the
  UID service before any card read.
- GTK tests prove a removed card invalidates the live image and disables the
  write commit action.
- No hardware run is required: the slice changes host lifecycle logic and gate
  selection, not a reader command sequence. Driver capability claims match the
  operations already recorded in the hardware matrix.
