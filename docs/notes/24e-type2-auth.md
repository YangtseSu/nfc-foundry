# Note: phase 24e Type 2 authenticated access

Status: implementation complete 2026-11-12 and hardware-validated on PN532
UART with an owned NTAG216; the ACR122U record is pending. Plan:
[24-parity-recovery.md](../plan/24-parity-recovery.md). Spec:
[24e-type2-auth.md](../specs/24e-type2-auth.md).

## Problem

Phase 09 could program PWD/PACK/AUTH0/ACCESS and verify the new PACK once, but
the product had no path to authenticate afterwards. Setting AUTH0 made the
tag's own NDEF message and configuration unreadable and unwritable, and the
hardware password test left the tag protected. The missing piece was a
PWD_AUTH session and an authenticated variant of every Type 2 workflow.

## Implemented

### Credentials and session

- `nfc_type2::Type2Auth` holds the four-byte password and a mandatory expected
  PACK. `Debug` redacts both values.
- `Type2AuthSession` sends one PWD_AUTH, requires the configured PACK, and
  reuses the authenticated state for the rest of one service operation.
  `reset` is available after a reselection or field loss. The session lives in
  the request workflow; `Service` stores no credential.
- Wrong password returns `AuthenticationFailed`; a missing credential returns
  the new `NfcError::AuthenticationRequired` with an actionable message. A PACK
  mismatch is `VerificationFailed` and never includes either PACK value.

### Read path

- `read_tag` reads the capability container, then the configuration pages. A
  configuration read that fails without credentials is reported as a protected
  tag instead of a partial image. With credentials it authenticates and retries.
- If PROT is set and AUTH0 covers user memory, the session authenticates before
  the first protected quad. If PROT is clear, reads stay open and writes
  authenticate later.
- PWD and PACK page data is discarded on read and stored as zero, so a reader
  that returns the write-only pages cannot leak them into the image.

### Write path

- `write_page_verified` checks the current configuration and authenticates
  before writing any page at or above AUTH0. `write_area`, `write_ndef`,
  `format_ndef`, `lock_pages`, and `restore_image` preflight the whole target
  set, so a missing or wrong credential cannot leave unprotected pages written
  before the first protected page fails. They pass the same session, so one
  operation sends one PWD_AUTH.
- `PasswordUpdate` carries the new AUTH0, ACCESS, PWD, and PACK. Programming
  validates AUTH0 as a user-memory page or `0xFF`, rejects CFGLCK and reserved
  ACCESS bits, and requires the current credentials when any configuration page
  is protected.
- The write order is PWD, PACK, CFG1, CFG0 (AUTH0/ACCESS last). The service
  then authenticates with the new credentials, reads both configuration pages
  back, and compares them. A failure after a commit reports the committed pages
  and the resulting password/configuration state.
- Clearing protection uses AUTH0 `0xFF`, ACCESS `0x00`, and the current
  credentials.

### Dumps and backups

`read_tag` masks PWD/PACK pages, and project JSON masks them on both export and
load. This keeps old files loadable while removing any secret a previous file
or reader returned. A password configuration backup therefore contains the
configuration pages but not PWD or PACK.

### CLI and GUI

- `nfctl` accepts global `--type2-password` and `--type2-pack`, with
  `--current-pwd` and `--current-pack` aliases, or the
  `NFC_FOUNDRY_TYPE2_PASSWORD` and `NFC_FOUNDRY_TYPE2_PACK` environment
  variables. Supplying only one of the pair is refused.
- All Type 2 and NDEF commands use the same `Type2ReadRequest`/builder fields.
  `type2 password` treats the pair as the current credentials.
- The GUI Protection page has masked Access PWD and Access PACK fields and
  masks the new PWD/PACK entries. The fields are cleared after a successful
  Type 2 read or mutation and are not part of `ui.json`.
- Page, NDEF, lock, restore, and password dialogs no longer print PWD or PACK
  values.

## Tests

- `nfc-type2`: credential redaction, one-shot session, wrong password, wrong
  PACK, protected read, protected page and NDEF writes, configuration change,
  protection clear, cancellation/refusal before mutation, dump masking, and
  bounded final-page reads.
- `nfc-transport` mock: PWD_AUTH state, reselection reset, read/write
  protection from CFG0/CFG1, and learning PWD/PACK when their pages are
  written.
- `nfc-app`: protected read/write/NDEF, wrong PACK before mutation, password
  change and clear, no secret in backups or errors, and one authentication per
  service operation.
- `nfctl`: incomplete credential pairs are refused and secrets are not echoed.
- GTK: protected read refusal, masked credential entry, protected page and
  NDEF writes, and clearing protection through the same dialogs.
- Hardware: `password_authentication_round_trip` passed on 2026-11-12 with a
  PN532 UART (`/dev/ttyUSB0`) and an owned NTAG216
  (`04 14 B5 7F DD 2A 81`). The tag started and ended with empty NDEF, no
  locks, and `AUTH0=0xFF`/`ACCESS=0x00`; the original CC, locks, and
  configuration matched after the round trip. PWD/PACK were then reprogrammed
  to the factory `FF FF FF FF` / `00 00` values, leaving the tag unprotected
  and empty.

## Problems and resolutions

- The first hardware run timed out on the authenticated read after the tag was
  protected. The service tried a protected READ first; that NAK left the tag
  ignoring PWD_AUTH until the next field cycle. With credentials supplied,
  `read_tag` now authenticates before the first READ.
- Programming `PACK=0000` did not write the PACK page. `write_page_verified`
  compared the requested PACK with the zero read-back of the write-only page
  and treated it as unchanged. PWD and PACK writes are now always issued, and a
  regression test covers a zero PACK.
- The hardware test was adapted from NTAG215/vCard to an NTAG21x with any
  original NDEF state. The PN532/NTAG216 run used an empty tag, wrote a Text
  record through authentication, and restored the empty state.

## Deviations and decisions

- The expected PACK is mandatory rather than optional. The card already rejects
  a wrong password, but checking PACK proves the caller selected the expected
  tag/configuration. The CLI therefore requires the PACK alongside the
  password; `--current-pwd` and `--current-pack` are aliases for that pair.
- Programming restricts AUTH0 to user memory or `0xFF`. Reading still supports
  a legacy configuration whose AUTH0 protects the configuration pages, because
  the configuration is read before authentication and retried with the supplied
  credentials.
- `ACCESS` writes reject CFGLCK and reserved bits. Permanently locking the
  configuration is outside this slice.
- The old password hardware test was replaced by a full authenticated round
  trip. It remains `#[ignore]` and gated by `NFC_FOUNDRY_HARDWARE_WRITE=yes`
  and an owned tag.

## Remaining blocker

The ACR122U authenticated-access record is pending. The PN532 UART record is
in the hardware matrix; the test still requires an owned tag with known
PWD/PACK.
