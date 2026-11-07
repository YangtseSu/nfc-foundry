# Phase 04b spec: DirectWrite magic UID variants

Date: 2026-11-08

## Goal

Treat CUID/Gen2, UFUID, and FUID cards as DirectWrite variants of the existing
4-byte UID write path. Do not add a chip-identity claim and do not send
irreversible magic-card configuration commands.

## Affected packages

- `internal/workflow/uid.go`: keep the ordinary write first and the Gen1A
  fallback only for an unchanged UID. Update comments and event text.
- `app/uid_service.go`: update the user-facing warning.
- `docs/cards.md`, `docs/cli.md`, `README.md`: describe the variant behavior.
- `docs/plan/04-native-gen1a.md`, `docs/specs/04-native-gen1a.md`: remove the
  blanket "FUID/UFUID unsupported" statement and point to this phase.

## Behaviour

Preflight remains unchanged:

- MIFARE Classic 1K, 4-byte UID, verified sector-0 key;
- block 0 BCC and full block-0 readback;
- full-card backup and block-0 backup before the first change.

Write sequence:

1. authenticate sector 0 with the verified key;
2. send one ordinary protected manufacturer-block write;
3. reselect and wait for the target UID;
4. read back the complete block 0 and verify BCC, UID, and manufacturer bytes;
5. only if the UID did not change, try the Gen1A backdoor on the same reader;
6. if the card is locked or one-write already used, report failure with the
   original UID and the backup path.

UFUID/FUID are not detected in advance. The ordinary write succeeds or fails
based on the card's current state. The UFUID permanent-lock sequence
(`0x40`, `0x43`, `0xE0`, `0xE1`, `0x85...`) is deliberately absent.

## Safety

- Never send the UFUID/FUID lock command or any unknown magic command.
- Never report success without reselection and complete block-0 readback.
- Preserve the original block 0 before any write.
- Keep the reader open across the ordinary and Gen1A attempts.
- A locked card must not be reported as a successful UID change.

## Tests

Hardware-free:

- DirectWrite success performs `backup,write` and does not use Gen1A.
- An unchanged UID after the ordinary write falls back to Gen1A only when raw
  frames are available.
- A locked/one-write failure reports verification failure and preserves the
  original UID.
- The user-facing warning names CUID/Gen2, UFUID, and FUID without promising
  lock support.

Hardware:

- Write one UFUID card, reselect, and verify the full block 0.
- Write one FUID card once; retry to confirm the second write fails safely.
- Use a full backup and explicit confirmation; never run this on a card whose
  UID must remain unchanged.

## Rollback

Only documentation and text change. The write algorithm already uses the
ordinary path first; reverting this phase restores the previous incomplete
wording, not a different hardware behavior.
