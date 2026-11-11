# Phase 19 plan: GTK4 workbench

Status: complete 2026-11-11. Phase 19 is the GTK4 workbench umbrella and was
executed as slices 19a–19e. There is no
separate implementation step called "19" outside those slices.

The GPUI Kit shell was removed on `feat/gtk4-ui`. The product uses one desktop
frontend: GTK4-rs (`gtk4` + `libadwaita`) in `crates/gui`, binary
`nfc-foundry`. Do not maintain a second frontend. Windows and macOS are
evaluated only after the Linux feature set is complete; a GTK Windows build
would keep GTK's look, not WinUI.

## Goal

- Persistent workbench: reader/card rail, card-family tasks, page content,
  inspector, status bar, activity log.
- Every card operation goes through `nfc-app`; the view never touches a reader
  or a driver.
- Reader I/O runs off the GTK main thread; the UI stays responsive and reports
  progress/errors in one place.
- Safety gates stay visible and mandatory: block 0 read-only, trailer
  confirmation, pre-write backup, read-back verification, no write on an
  unverified or swapped card.
- English base copy first; the `zh-CN` catalog and language switch are
  restored before the GUI is called complete.
- `nfctl` parity remains the completion rule for every operation.

## Implemented in the first slice

- `crates/gui` GTK4 application with the AppStream-style shell:
  - left rail: reader menu, card summary, sector-key status, selected block,
    task and security log;
  - `Card` and `NDEF` task switcher;
  - block table with real `Dump` rows, selection, inspector, hex editor;
  - status bar and toast feedback.
- Real `DriverRegistry` enumeration and exact reader selection through
  `nfc-app`.
- `Read card`: `Service::classic_read` on a worker thread; partial reads and
  warnings are shown, not hidden.
- Block read and block write through `Service::classic_read_block` and
  `Service::classic_write_block`:
  - old/new confirmation;
  - destructive confirmation for sector trailers;
  - backup path reported after the write;
  - read-back performed by the service;
  - block 0 and unchanged blocks cannot be written.
- Save/load project dumps through the file dialog.
- Read-only Classic NDEF tab from the loaded dump: MAD sectors, mapping
  version, access state, capacity, Text/URI records, warnings.
- Reader changes invalidate the live-write permission; a fresh read is
  required after changing the reader.

## Remaining

None in the GTK4 workbench scope. The Windows/macOS assessment mentioned in
earlier drafts is out of the Linux-only product scope; a future port would be a
separate decision under [ADR 0006](../adr/0006-gtk4-rs.md).

## Acceptance for the first slice

- Real PN532 UART Classic 1K read: 60/64 blocks, sector 5 authentication
  warning visible.
- Table shows all 64 rows; selecting a row updates the inspector.
- Block 0 cannot be written; a changed trailer opens a destructive
  confirmation; cancel leaves the card unchanged.
- Reader change disables writes until the card is read again.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
  warnings`, `cargo test --workspace`, and `reuse lint` pass.

Detailed spec: [19-gtk-ui.md](../specs/19-gtk-ui.md). Implementation record:
[19-gtk-ui.md](../notes/19-gtk-ui.md).
