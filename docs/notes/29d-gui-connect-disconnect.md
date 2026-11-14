# Note: 29.4 GUI Connect / Disconnect

Plan: [29-cli-reader-ownership.md](../plan/29-cli-reader-ownership.md). Spec:
[29d-gui-connect-disconnect.md](../specs/29d-gui-connect-disconnect.md).

## Implemented

- `crates/nfc-app/src/reader.rs`: `ReaderStatus::Disconnected` and
  `MonitorState::disconnected`, which clears card and capability facts,
  queues the removal event, and emits the status transition. `poll_due`
  rejects both `Error` and `Disconnected`; `forget_card` preserves a
  deliberate disconnect.
- `crates/nfc-app/src/device.rs`: `DeviceManager` tracks the host intent.
  `disconnect` closes the reader, drops the cross-process lock, and parks the
  monitor. `connect` clears the state and resets polling; the next poll or
  operation opens the reader and claims the lock again. `begin_operation`,
  `retry`, `select`, and `refresh` keep the state consistent.
- `crates/nfc-app/src/service.rs`: `disconnect_reader` and `connect_reader`
  wrappers for both surfaces.
- `crates/gui/src/app.rs`: `Connect` and `Disconnect` buttons in the reader
  rail. Disconnect invalidates the live image, resets Advanced mode, disables
  the write commit, and keeps the inspected bytes editable. Connect polls once
  and reports the resulting status. Button sensitivity follows the snapshot
  and the busy flag.
- `crates/gui/src/i18n.rs`: `Disconnected`, `Reader disconnected`,
  `Reader connected`, `Reader recovering`, and `Reader error` entries.
- Documentation: `docs/design/workbench.md` reader contract, spec 24f
  lifecycle list, plan index, and this note.

## Behavior

- A disconnected monitor consumes no reader I/O. The GUI background thread
  keeps stepping the service, but `poll` returns the snapshot without opening
  the device.
- The reader dropdown and refresh stay usable while disconnected. Changing the
  selection or refreshing changes the selected connstring but does not resume
  polling.
- `Read card` reconnects through `begin_operation`, so the header action is
  still the single "get me a card" path.
- Disconnect cannot run during an operation: the buttons are disabled while
  busy, and the device manager's operation lease owns the reader.
- The status bar reports `Reader connected` when the connect poll reaches
  `Polling`; a lock contention or open failure leaves the panel error and the
  status bar reports `Reader error`.
- No card data, key material, or new log fields are added.

## Tested

2026-11-14:

- `cargo fmt --all --check`.
- `cargo test -p nfc-app disconnect`: 4 tests pass.
- `NFC_FOUNDRY_GUI_TESTS=1 xvfb-run -a cargo test -p nfc-foundry --bin
  nfc-foundry app::tests::gtk_workbench_scenarios -- --exact`: pass, including
  the disconnect/reconnect scenario.
- `cargo clippy --workspace --all-targets -- -D warnings`.
- `cargo test --workspace`: 787 passed, 0 failed.
- `reuse lint`: 693/693 files compliant.

## Limitations

- Disconnect releases only the lock held by this process. Another process may
  claim the reader immediately after release; Connect then reports the busy
  error through the existing reader lock path.
- The GTK scenario uses the mock transport, which has no physical id, so the
  lock release itself is covered by the `nfc-app` device test with an injected
  lock directory rather than by the GUI test.
