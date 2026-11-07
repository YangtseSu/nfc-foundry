# Phase 05i spec: NTAG copy and lifecycle operations

Date: 2026-11-07

## Goal

Add Advanced-mode Type 2 copy, repeated copy, erase, lock, and password set/
remove operations to the GUI and CLI with the existing backup, confirmation,
read-back, and card-change rules.

## Lifecycle model

`internal/type2` owns the plans; `internal/workflow` executes them.

```go
type CopyOptions struct {
    IncludeProtected bool
    SameUIDOnly      bool
}
func PlanCopy(source, target Document, options CopyOptions) ([]PageWrite, []Change, error)

type LockOptions struct {
    StaticLock  *uint16
    DynamicLock *[3]byte
    CCReadOnly  bool
    ConfigLock  bool
}
func PlanLock(target Document, options LockOptions) ([]PageWrite, []Change, error)

type PasswordOptions struct {
    Password [4]byte
    Pack     [2]byte
    Auth0    int
    Protect  bool
}
func PlanPasswordSet(target Document, options PasswordOptions) ([]PageWrite, []Change, error)
func PlanPasswordRemove(target Document) ([]PageWrite, []Change, error)
func PlanErase(target Document) ([]PageWrite, error)
```

`Change` carries `Target`, `Old`, `New`, and `OneWay` for the confirmation UI.

### Page write ordering

Writes are planned so that recovery-critical pages go last:

1. user memory and NDEF bytes;
2. PWD and PACK;
3. configuration pages (AUTH0, ACCESS);
4. dynamic lock;
5. CC access byte;
6. static lock.

The workflow verifies every page immediately, then reselects the card, reads
the complete image with the current (possibly new) password, and verifies the
final image.

## Copy

- Source and target must use the same product and geometry.
- Default copy writes user-memory pages only. UID pages are never written and
  are not part of the plan.
- A different target UID is the normal copy case. `SameUIDOnly` rejects a
  different UID. The copy never claims an identical-UID clone.
- Target CC must be valid and writable for a default copy; a read-only or
  missing CC is reported before the first write.
- Advanced copy additionally plans CC, configuration, PWD, and PACK when the
  target state allows it:
  - CC: a blank `00 00 00 00` target may receive the source CC. Otherwise only
    access bits may be ORed; bytes 0-2 are preserved.
  - Static lock bits are not copied.
  - Dynamic lock bits are copied only as an OR of target bits, never cleared.
  - CFGLCK is one-way. A target with CFGLCK already set rejects configuration
    and password writes. Copying a source with CFGLCK set is allowed only with
    protected confirmation and is reported as irreversible.
  - A protected target requires the current password for the whole operation.
- Every page is read back. A mismatch stops the sequence and keeps the
  pre-write backup.

### Repeated copy

The CLI applies one source image to `--repeat N` targets. It waits for a card,
copies, verifies, then waits for that card to leave before accepting the next
one. The default stops on the first target error. `--continue-on-error` keeps
waiting for a new target after a failed target and reports both failures and
successes. The GUI exposes the same loop as an Advanced task with a target
count and a stop/cancel control.

## Erase

- Zeroes every user-memory page that is not locked.
- Does not touch UID, static lock, dynamic lock, configuration, PWD, or PACK.
- Reports explicitly that lock and configuration bits remain.
- A formatted CC remains; the tag reads as having no NDEF message.

## Lock

All targets are optional and explicit; at least one is required.

| Target | Page | Rule |
|---|---|---|
| Static lock | 2, bytes 2-3 | OR-only, preserve bytes 0-1 |
| Dynamic lock | dynamic-lock page, bytes 0-2 | OR-only |
| CC read-only | 3, byte 3 | OR `0x0F`, preserve bytes 0-2 |
| CFGLCK | config end page, byte 0 bit 6 | one-way; reject when already set |
| Password protection | PWD/PACK/AUTH0/ACCESS | use the password-set plan |

Static lock bits for pages 3-15 are checked before planning. Writing a lock
bit that is already set is skipped. A target product without dynamic lock,
configuration, or password pages refuses the corresponding target.

## Password set/remove

Set:

- PWD (four bytes), PACK (two bytes plus two RFUI bytes kept as `00 00`);
- AUTH0 (byte 3 of the configuration start page);
- ACCESS: set PROT, preserve CFGLCK and AUTHLIM;
- `Auth0` defaults to the first user page; the caller may supply another page
  inside user memory;
- an unprotected tag needs no current password; a protected tag requires the
  current password through PWD_AUTH.

Remove:

- PWD becomes `FF FF FF FF`, PACK `00 00`, AUTH0 `FF`, and PROT is cleared;
- requires the current password when the target reports password protection;
- refuses when CFGLCK is set, because configuration and password pages are no
  longer writable;
- verifies the final image without a password.

## Confirmation

Every lifecycle plan is previewed before execution. The preview returns the
card UID, operation, old/new values, irreversibility flags, page count, and a
token over the planned writes. Execution re-reads the card, re-plans, and
refuses when the token or the selected card changed.

The GUI requires a typed confirmation equal to the card UID for copy, erase,
lock, and password operations. A repeated-copy confirmation authorizes the
sequence; each later target is re-read and re-planned before it is written, and
a changed plan for the first target aborts the sequence. The CLI uses `--yes`
plus the operation-specific confirmation flags:

```text
--confirm-copy --confirm-erase --confirm-lock --confirm-password
```

`--protected` copy additionally requires `--confirm-protected`.

## CLI

```text
nfctl type2 copy SOURCE [--include-protected] [--same-uid-only] [--repeat N]
    [--continue-on-error] [--pwd HEX] [--yes] [--confirm-copy]
    [--confirm-protected] [--dry-run]
nfctl type2 erase [--pwd HEX] [--yes] [--confirm-erase] [--dry-run]
nfctl type2 lock [--static-lock HEX4] [--dynamic-lock HEX6] [--cc-readonly]
    [--cfglock] [--pwd HEX] [--yes] [--confirm-lock] [--dry-run]
nfctl type2 password set --new-pwd HEX --pack HEX [--auth0 N] [--protect]
    [--current-pwd HEX] [--yes] [--confirm-password] [--dry-run]
nfctl type2 password remove [--current-pwd HEX] [--yes]
    [--confirm-password] [--dry-run]
```

`SOURCE` accepts JSON or binary dumps. Copy/erase/lock/password print the plan
in `--dry-run` and the written/verified page lists otherwise.

## GUI

The Protection tab gains an Advanced section, visible only while Advanced mode
is enabled:

- password field for PWD_AUTH (shared with reads and writes);
- import source dump, source summary, and Copy to this tag;
- repeated copy target count;
- erase user memory;
- static lock, dynamic lock, CC read-only, CFGLCK checkboxes/fields;
- password set fields and password remove action.

Every action first requests a preview, then opens a modal with old/new values,
the backup rule, irreversibility, and a UID text field. The confirm button
stays disabled until the UID matches.

## Tests

- Copy between same-product tags with different UIDs, user memory only.
- Advanced copy preserves target UID/BCC and applies CC/config/PWD/PACK rules.
- Same-UID policy, product mismatch, read-only CC, and CFGLCK refusals.
- Repeated-copy stop and continue policies.
- Erase leaves lock/config/PWD pages unchanged.
- Static/dynamic lock OR rules, CC read-only, and CFGLCK one-way checks.
- Password set/remove with current-password enforcement and post-write auth.
- Card change during the operation aborts after the backup.
- GUI preview, typed confirmation, and Advanced reset on card removal.
- `make check` passes.

## Non-goals

- Genuine NTAG UID writes.
- Magic-clone UID/backdoor commands (Phase 05h).
- Universal kill/delete command.
- Bypassing an unknown password or CFGLCK.
- Chunked or interrupted-write continuation within one target.
