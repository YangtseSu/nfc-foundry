# Phase 05d spec: Advanced Type 2 protected writes

Date: 2026-11-07

## Goal

Add GUI writes for Type 2 configuration, dynamic lock, PWD/PACK, and static lock
under the shared advanced mode. Pages 0-1 and page 3 remain read-only.

## Scope

Writable in this phase:

| Area | Pages | Rule |
|---|---|---|
| Configuration | 41-42 / 131-132 / 227-228 | AUTH0 and ACCESS; per-action confirmation |
| PWD/PACK | 43-44 / 133-134 / 229-230 | Password and PACK; lockout warning |
| Dynamic lock | 40 / 130 / 226 | One-way lock bits; preserve RFUI |
| Static lock | 2 bytes 2-3 | Partial page write; preserve BCC1/internal |

Read-only:

- pages 0-1: UID and BCC0;
- page 2 bytes 0-1: BCC1 and internal;
- page 3: capability container/OTP.

## Application surface

Add DTOs and bindings:

```go
type Type2ProtectionWriteRequestDTO struct {
    Auth0           *int
    Access          *byte
    Password        *string
    Pack            *string
    DynamicLock     *string
    StaticLock      *string
    Confirmation    string // exact "PROTECTED"
}

func (s *Service) PreflightType2Protection(request Type2ProtectionWriteRequestDTO) (Type2ProtectionPreflightDTO, error)
func (s *Service) StartType2ProtectionWrite(request Type2ProtectionWriteRequestDTO) (TaskDTO, error)
```

The preflight reads the current image, validates the requested changes, builds a
page plan, and returns a token. The start method revalidates the card identity
and plan before writing.

## Validation

- AUTH0 must be `0-255`; `0xFF` keeps authentication disabled.
- ACCESS may change PROT, CFGLCK, AUTHLIM, and the NFC counter bits. The
  remaining bits are preserved.
- PWD is exactly four bytes; PACK is exactly two bytes. RFUI bytes are preserved.
- Static lock is a 16-bit value. Pending bits may only be set; clearing a set bit
  is rejected before any write.
- Dynamic lock is a 24-bit value. Pending bits may only be set; clearing a set
  bit is rejected.
- Static-lock writes read page 2, replace bytes 2-3, preserve bytes 0-1, write
  the page, and verify the complete page.
- Dynamic-lock writes read the lock page, replace bytes 0-2, preserve byte 3,
  write, and verify.
- PWD/PACK writes read their pages, replace only the defined bytes, preserve
  RFUI, write, and verify.
- Configuration writes preserve reserved and mirror bytes unless a semantic
  field explicitly changes them.

## Write order

1. ordinary user pages, if part of the same confirmed plan;
2. PWD/PACK;
3. configuration pages;
4. dynamic lock;
5. static lock last.

Static lock is last because it is the most likely one-way operation to make
later writes fail.

## Backup and verification

- A full-image backup is written before the first protected page change.
- A backup failure aborts before any write.
- Each changed page is read back immediately.
- The session is deactivated, the card is reselected, and UID/ATQA/SAK are
  compared.
- The final image is read and every changed page is compared.
- Failure stops the operation and reports the backup path.
- A PWD change updates the in-session authentication password for final
  verification.

## Confirmation UX

- Protection controls are hidden unless advanced mode is enabled.
- The dialog lists each field's old and new value.
- The dialog states that lock bits are one-way and configuration changes can
  brick the tag.
- The user must type `PROTECTED` exactly.
- The confirmation token expires after five minutes.
- The operation is refused if the card, product, or current image changes.

## Tests

Hardware-free:

- AUTH0/ACCESS/PWD/PACK validation and RFUI preservation;
- lock-bit refusal when clearing bits;
- static-lock partial page write preserves BCC1/internal;
- dynamic-lock partial write preserves RFUI;
- backup failure aborts;
- read-back mismatch reports written pages and backup;
- card change aborts;
- advanced-mode gating and `PROTECTED` phrase refusal;
- GUI tests for hidden controls, old/new preview, and confirmation.

Hardware:

- gated by `NFCX_WORKFLOW_HARDWARE=1` and
  `NFCX_HARDWARE_TYPE2_PROTECTED_CONFIRM=I_OWN_THIS_DISPOSABLE_TAG`;
- only on a disposable tag;
- set and verify static/dynamic lock or configuration as selected, then confirm
  the tag is intentionally left in the test state. Do not run on a production
  or user tag.

## Non-goals

- UID/page 0-1 writes.
- Capability-container/page 3 writes.
- Arbitrary page 0-3 editing.
- Magic-tag UID protocols.
- A write path that bypasses advanced mode, backup, or read-back.
