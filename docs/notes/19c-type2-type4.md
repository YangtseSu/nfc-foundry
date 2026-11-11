# Note: phase 19c Type 2 / NTAG and Type 4 read

Plan: [19c-type2-type4.md](../plan/19c-type2-type4.md). Spec:
[19c-type2-type4.md](../specs/19c-type2-type4.md).

## Implemented

### Service

- `Type2ReadResult` and `Type2NdefResult` carry the selected `CardInfo`, so the
  GUI keeps the identity the image was composed from.
- Every Type 2 mutation request now has an optional `expected_card`:
  page write, NDEF write, NDEF format, restore, lock, and password. The check
  runs after cancellation/confirmation and before the read, backup, or card
  I/O. A mismatch is `CardChanged`.
- `Service::read_card` selects the card once and dispatches to the Classic dump
  or the Type 2 image read. Unknown card families are refused.
- A capability-container page write is validated before the write: the
  resulting CC must parse and must keep the same product/data-area size. After
  a verified CC write, the image's parsed `cc` is updated, so NDEF capacity and
  read-only state follow the card. A size/product change is refused before the
  card is touched.

### Workbench

- `Card` remains the Classic image. A new `Type 2` tab contains `Memory` and
  `Protection`; the global `NDEF` tab renders the active card family.
- `Read card` uses `Service::read_card`. A Type 2 read clears the Classic image
  and rows, stores a `Type2Session`, and switches to the Type 2 tab. A Classic
  read clears the Type 2 session. A failed read disables writes on the previous
  image.
- Memory shows the page table with page, role, lock/access state, and data.
  Unknown pages render `—`; a partial read keeps its warnings. The default
  filter shows manufacturer, user, and reserved pages. CC, static/dynamic lock,
  configuration, and PWD/PACK pages appear only in Advanced mode.
- The page inspector shows the role, lock state, UID/BCC facts, CC size and
  access, NDEF data-area offset, configuration fields, and write gate. PWD and
  PACK are marked write-only.
- User-page writes use the ordinary confirmation. Advanced page writes require
  Advanced mode and a destructive confirmation dialog. Both call
  `Service::type2_write_page` with the session identity and refresh the page
  table, inspector, card summary, and NDEF facts from the result.
- Memory save uses the Type 2 project JSON format. Load detects the
  `nfc-foundry-type2` marker before the Classic formats and shows the image as
  inspection-only. Restore exposes user-memory, configuration, capability
  container, and lock-byte includes, defaults to user memory only, and requires
  the confirmation dialog. The service backs up the live tag and reads every written
  page back.
- Protection shows the parsed configuration. The lock form parses page lists
  (`4,8,16-20`), previews the exact static/dynamic lock bytes before/after, and
  applies through `Service::type2_lock` after the confirmation. The
  password form programs AUTH0/ACCESS/PWD/PACK through `Service::type2_password`
  and reports the PWD_AUTH verification.
- Advanced mode resets on every card read, reader refresh/change, and loaded
  dump. The confirmation response handler is the execution gate; the dialog is
  built only after the action's preconditions pass.
- The NDEF tab reuses `NdefDraft`/`NdefMessage` from 19b. For a Type 2 card it
  shows product/UID, CC bytes, data-area size, capacity, locked pages, and
  password protection. `Write NDEF` is gated on a live complete data area, a
  writable CC, no locked data page, no password-protected data page, an
  encodable message, capacity, and a message different from the stored TLV.
  `Format NDEF area…` requires the confirmation dialog and removes proprietary TLVs,
  matching the service behavior. NDEF writes pass the session identity.
- Type 4 has no surface. No hardware record exists for a reader/card path, so
  the read-only view and a matching `nfctl` command are left out.

## Verification

- `cargo fmt --all --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test --workspace`: passes; the GTK scenario takes its skip branch
  without the gate.
- `env -u WAYLAND_DISPLAY NFC_FOUNDRY_GUI_TESTS=1 xvfb-run -a env
  GDK_BACKEND=x11 cargo test -p nfc-foundry -- --nocapture
  gtk_workbench_scenarios`: passes.
- `reuse lint`: clean.

The gated scenario adds a scripted NTAG213 and covers:

- page roles, unknown-page rendering, Advanced filtering, and CC/config facts;
- a user-page write with cancel and read-back, and a locked-page refusal;
- an Advanced CC write with the confirmation dialog, its `image.cc` refresh, and the
  resulting read-only NDEF gate;
- one-way locks with the byte preview and the already-locked state;
- AUTH0/ACCESS/PWD/PACK programming with PWD_AUTH verification;
- Type 2 NDEF write, format, read-only, unformatted, locked-page, partial-area,
  and identity-change gates;
- Type 2 dump save/load, restore with cancel and confirmation, restore
  identity refusal, and the include-option gate.

Unit tests cover NTAG213/215/216 page-row visibility and roles, lock previews,
page/hex parsing, NDEF area/lock/password helpers, CC-write validation, and the
service read dispatch. The GTK scenario also checks that Advanced, lock,
password, and restore dialogs name the card, values, and backup.

## Limits

- No hardware was touched. All Type 2 writes, locks, password changes, and NDEF
  round trips ran against the scripted transport. This note does not add a
  hardware support claim.
- Type 4 remains software-only with no reader/card record, so it is absent from
  the GUI and CLI.
- The product has no "authenticate with PWD" workflow. A tag whose AUTH0 covers
  the NDEF data area or lock pages is shown as protected and denied for writes
  rather than silently attempting an unauthenticated write.
- A password or PACK page write is visible in Advanced mode but the dedicated
  Protection form is the supported path because it programs and verifies the
  whole configuration.
