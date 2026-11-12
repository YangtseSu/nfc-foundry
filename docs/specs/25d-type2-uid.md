# Spec: phase 25.4 Type 2 UID change

Status: implemented 2026-11-12; PN532 UART hardware validated with a 7-byte
NTAG213 clone, ACR122U not recorded. Plan:
[25-card-support.md](../plan/25-card-support.md).

## Problem

Type 2 writes deliberately refuse `PageRole::Manufacturer`, so page 0/1/2 are
read-only through the normal page workflow. A UID-changeable NTAG213 clone
accepts direct writes to those pages, but the product had no guarded path and
no BCC handling.

## Scope

- Seven-byte UID only. The UID must start with the NXP manufacturer byte `04`.
- Standard NTAG213 layout: page 0 `UID0..2 + BCC0`, page 1 `UID3..6`, page 2
  `BCC1 + internal + static lock bytes`.
- The clone must accept writes to pages 0, 1, and 2. A same-value probe is the
  first hardware step; a genuine NTAG213 refuses page 0 and is reported as not
  UID-writable.
- The target UID may differ in any byte. BCC0 and BCC1 are recomputed; the
  internal byte and static lock bytes are preserved exactly.
- No lock bit, configuration, password, or NDEF change is part of this
  operation.

## Workflow

1. Read the complete image and validate the current UID/BCC layout.
2. Build a plan with the old and new page 0/1/2 values. `--dry-run` stops
   here.
3. Require Advanced mode and explicit confirmation.
4. Save a project backup before the first write.
5. Write page 1 (UID3..6), page 2 (BCC1 plus preserved bytes), then page 0
   (UID0..2 plus BCC0). Each page is read back immediately.
6. Reselect and require the target UID, unchanged ATQA, and unchanged SAK.
7. Read pages 0..2 and verify all three values.
8. On any failure, write the original pages back, reselect, and require the
   original UID. Return an error that states whether the restore succeeded and
   names the backup.

## Surfaces

- `nfctl type2 uid --uid <14 hex> --advanced [--dry-run]`; `--yes` supplies the
  confirmation. Without `--advanced` the command refuses before reading.
- GTK: Protection → UID (magic clone) → target entry → Change UID… → plan
  dialog → confirm. The confirmation names every page transition and the
  restore behavior.
- Both surfaces use the same `nfc-app` service and the same backup/read-back
  path.

## Acceptance

- A same-value probe accepts pages 0/1/2 and leaves the UID unchanged.
- Change `04 00 E3 C2 00 00 02` to `04 00 E3 C2 00 00 03`: page 1 becomes
  `C2 00 00 03`, page 2 becomes `C1 08 00 00`, page 0 stays `04 00 E3 6F`, and
  the reselected UID is the target.
- Restore the original UID: all three pages return to the original values and
  the page JSON matches the pre-change image.
- A genuine NTAG213 refuses the write without mutation.
- The lock bytes remain `00 00` throughout.
- ACR122U remains unrecorded.

## Limits

- Four- and ten-byte UIDs are not implemented.
- A password-protected UID page path is not implemented; the workflow runs on
  an unprotected clone.
- The clone must be confirmed writable by hardware; no vendor claim is made.
