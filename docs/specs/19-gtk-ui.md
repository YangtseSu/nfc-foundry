# Spec: phase 19 GTK4 workbench

Status: complete; implemented through slices 19a–19e.

## Surface

One GTK4/`libadwaita` window. The shell is persistent:

- header: product name, `Actions` menu, primary `Read card`;
- left rail: reader menu, card summary, sector-key status, selected block, and
  a session log;
- content: `Card` and `NDEF` tasks;
- `Card`: block table and fixed inspector;
- `NDEF`: read-only mapping/message view in this slice;
- status bar: current operation, spinner, and card/layout facts.

The rail stays visible. Reader selection is the only reader control; there is
no duplicate Read action on individual pages. The primary action stays in the
header.

## Threading

`Service` is held in `Arc<Mutex<Service>>`. Reader operations run on a
`std::thread`; the result returns through `async-channel` and
`glib::spawn_future_local`. GTK widgets are only touched on the main context.

The view disables the reader menu and Read action while an operation is in
flight. A reader change invalidates the live-write permission and requires a
new read.

## Safety

- The table and inspector show unknown bytes as `??`; a partial read is never
  presented as complete.
- Block 0 is read-only in the UI and the service.
- A data write opens a confirmation that names the block, sector, old bytes,
  and new bytes, and explains the backup and read-back.
- A trailer write uses the destructive response and the service's
  `confirm_trailer(true)` gate.
- Unchanged input disables the write button.
- The backup path is shown in the log and success toast.
- A loaded dump is inspectable but not writable; a live card is required.
- NDEF is read-only in this slice. No format/write action is shown until the
  shared document/schema path exists.

## Persistence

- Classic key store: `KeyStore::default_path()` via `nfc-app`.
- Backups: `BackupStore::default()` via `nfc-app`.
- Dump save/load: project JSON through `nfc-classic::exchange`.
- No card data, UID, or key is written to logs beyond the existing session
  log; the log is in-memory only.

## Out of scope

- A second frontend or a GPUI fallback.
- NDEF writes, formatting, key/diff/recovery screens, Type 2/Type 4 screens.
- `zh-CN` catalog (restored before GUI completion).
- Packaging and Windows/macOS support.
