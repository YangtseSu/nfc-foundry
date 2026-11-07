# Phase 04b: DirectWrite magic UID variants

## Goal

Support 4-byte UID MIFARE Classic 1K DirectWrite magic cards through the
existing ordinary authenticated block-0 write, without claiming a chip identity
or sending irreversible configuration commands.

## Background

Magic Classic cards are not one family:

- **Gen1A**: requires the `0x40`/`0x43` backdoor sequence; implemented in
  Phase 04 as a fallback.
- **Gen2/CUID**: block 0 is writable with an ordinary authenticated MIFARE
  write.
- **UFUID**: DirectWrite variant; block 0 remains writable until an explicit
  irreversible lock command is sent.
- **FUID**: DirectWrite variant; block 0 is writable once.

The application cannot reliably distinguish these variants from a normal
Classic card before writing. Support is therefore defined by observed behavior:
attempt the ordinary write, reselect, and verify the complete block 0. Never
send a lock or configuration command as part of a UID write.

## Work

- Keep the ordinary authenticated block-0 write first.
- Treat UFUID and FUID as DirectWrite variants of that same path.
- Use the Gen1A fallback only when the ordinary write did not change the UID.
- Do not send the UFUID lock command, FUID lock, or any Gen1B/Gen3/APDU magic
  command.
- Report a locked or one-write card honestly after reselection; keep the
  original block-0 backup.
- Update CLI, GUI text, and the support matrix without claiming chip
  identification.
- Extend hardware-free fixtures for DirectWrite success and locked/one-write
  failure.

## Acceptance

- A UFUID card can be written, reselected, and verified through the ordinary
  path.
- The first write to an FUID card works; a second write fails safely and reports
  that the UID did not change.
- A successful DirectWrite never sends Gen1A frames.
- A locked card keeps its original UID and the operation reports failure with
  the backup path.
- `make check` passes; the hardware test is explicit.

## Non-goals

- No UFUID lock command; that is a separate Advanced destructive action.
- No Gen1B, Gen3/APDU, or custom-unlock magic commands.
- No 7-byte or 10-byte UID support.
- No chip-family detection claim beyond observed write behavior.
