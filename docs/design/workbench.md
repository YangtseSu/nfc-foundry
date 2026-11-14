# GTK4 workbench contract

This is the current screen contract for `crates/gui`. GTK4-rs (`gtk4` +
`libadwaita`) is the selected GUI stack in
[ADR 0006](../adr/0006-gtk4-rs.md); it replaces the removed GPUI rules.

## Shell

- One window, 1360×880 default, resizable.
- Header: `NFC Foundry` title, workspace selector (`MIFARE Classic` /
  `NFC Forum Type 2` / `DESFire`), primary `Read card` (`Refresh card` on
  DESFire), and a `☰` main menu. A `←` back button appears only
  while a system page is shown. The reader selector is not duplicated in the
  header.
- Left rail (fixed start pane, `GtkPaned`): reader menu with refresh,
  connection status, and capability profile, card identity/size/readability,
  key-store status, and selected block.
- The reader monitor polls through `nfc-app` with one exclusive operation
  lease. Removing the card, presenting a different card, or losing the reader
  invalidates the live image, resets Advanced mode, and disables writes while
  the inspected bytes stay visible.
- Workspaces: `Classic` shows `Blocks`, `Details`, `NDEF`, `Keys`, `Recovery`,
  and `Dumps`;
  `Type 2` shows `Pages`, `Protection`, `NDEF`, and `Dumps`; `DESFire` shows
  `Inventory`, `Read`, `Verify`, `NDEF`, `Manage`, `Values & records`, and
  `Keys`. Reads, key verification, and the authenticated UID read are
  ungated; NDEF formatting, replacement, deletion, and the Manage `Format PICC
  (erase all)` action stay Advanced. `Keys` owns
  `Key store` / `Key scan`; Classic `Dumps` owns Save/Load plus `Diff` /
  `Restore`; Type 2 `Dumps` owns Save/Load plus Restore. The NDEF view is
  shared and moved into the active workspace; it renders Classic MAD/NDEF for
  Classic and Type 2 CC/TLV/NDEF for Type 2.
- `Read card` detects a Classic or Type 2 card, selects the matching workspace,
  and selects the card or pages tab. On the DESFire workspace the same button
  is labelled `Refresh card` and only refreshes the unauthenticated inventory.
  The header selector switches manually for file-only work.
- `Diagnostics`, `Activity`, and `Settings` are not workspace tabs. The `☰`
  menu opens a hidden `System` page at the requested sub-tab; `←` returns to
  the last workspace. `About NFC Foundry` opens `AdwAboutDialog`.
- Status bar: operation text, spinner, and layout/card facts.

## Blocks

- `GtkColumnView` with `Block`, `Sector`, `Type`, `Status`, and `Data`.
  `Data` expands and shows all 16 bytes; unknown bytes are `??`.
- Selecting a row updates the inspector; the rail repeats the selection.
- Inspector: block number, sector, role, status, error text, decoded detail,
  hex entry, Read block, Write block, and the safety note.
- Decoded detail:
  - access group, its block range, C1/C2/C3 condition, and read/write/
    increment/decrement keys;
  - sector-trailer conditions for all four groups, trailer write permissions,
    Key A, Key B, and GPB;
  - value-block value and address when the 16 bytes form a valid value block;
  - manufacturer-block BCC status;
  - warnings for unknown bytes, unknown access bytes, and invalid access-bit
    complements.
- Empty state before a read: "No card loaded", reader + Read hint.
- Busy state: spinner, disabled reader menu and Read, disabled writes.

## Details

- Renders `nfc_app::classic_health` for the loaded Classic image: identity
  (type, UID, length, ATQA, SAK), layout, source (`live card` or the loaded
  path), live/inspection state, reader and read time when recorded,
  completeness (blocks and sectors), skipped sectors, verified key coverage,
  dump key count, dirty blocks, and validation errors/warnings.
- No card I/O and no key material. The model is shared with
  `nfctl card health`.

## Type 2

- The Type 2 workspace has `Pages`, `Protection`, `NDEF`, and `Dumps`. It is
  active after a Type 2 read or a loaded Type 2 project image; a Classic read
  clears it.
- Pages: identity/product summary, partial-read warnings, `Advanced mode`
  switch, page table (`Page`, `Role`, `Lock`, `Data`), and page inspector.
- Dumps: project save/load and restore options. The Pages tab no longer
  duplicates them.
- NDEF: the shared message editor, rendering Type 2 CC/TLV facts and the
  decoded record set. Locked/password-protected tags disable write with a
  specific reason; NDEF write preserves unknown TLVs, format removes them.
- The default page filter shows manufacturer, user, and reserved pages. CC,
  static/dynamic lock, configuration, and PWD/PACK pages appear only in Advanced
  mode. Unknown pages show `—`.
- Inspector: page, role, lock/access state, UID/BCC, CC version/size/access,
  NDEF offset, configuration fields, hex/ASCII, write feedback, and
  `Write page…`. PWD/PACK are labeled write-only.
- User-page writes use the ordinary confirmation. Advanced pages require
  Advanced mode and the destructive confirmation dialog.
- Protection: parsed configuration, a page-list lock form with exact
  static/dynamic lock byte preview, and AUTH0/ACCESS/PWD/PACK fields. Lock,
  password, restore, and Advanced page writes use the destructive confirmation
  dialog.
- Restore defaults to user memory; configuration, capability container, and
  lock bytes are opt-in. The live tag is backed up before the restore.
- Advanced mode is off by default and resets on card read, reader change,
  refresh, and loaded dump.
- A partial image, locked page, read-only CC, or password-protected page is
  refused with a specific feedback line. The service keeps its own gates.

## Write flow

- The write button is disabled for block 0, incomplete blocks, unchanged
  bytes, a loaded dump, or no live card.
- Confirmation names the block, sector, old bytes, and new bytes; it states
  that a project backup and read-back happen.
- Sector trailers use the destructive appearance and the service's
  `confirm_trailer` gate.
- The GUI passes the loaded dump's card identity to the service. A card swap
  is refused with `CardChanged` before the backup or write.
- Success reports the backup path in the log and a toast; the row and
  inspector update from the written bytes.

## Keys

- `GtkColumnView` with `Sector`, `Type`, `Key`, and `Source`. Stored entries
  show their provenance; built-in common keys appear as `all sectors`,
  `A+B`, `built-in` when the same key is not stored.
- Add one key: 12 hex digits, optional sector, `any`/`A`/`B`.
- Delete selected removes exactly the selected stored entry and persists the
  store. Built-in rows are read-only candidates and cannot be deleted.
- Import `.dic` text or JSON; save the JSON store; export a Proxmark3 `.dic`.
  The last import report shows added, duplicates, ignored, and line-numbered
  issues.
- Copy states that `.dic` export is lossy and does not carry sector, type, or
  provenance.
- Reads and writes use the same in-memory store; there is no second key
  source.

## Key scan

- Candidates: the current key store, an optional text/JSON dictionary, and the
  built-in common keys. Key A, Key B, or both are selectable.
- Progress reports the completed sector, attempts, and verified trailer reads;
  the result table lists `Sector`, `Key A`, and `Key B`.
- `Save discovered keys` merges into the JSON store without duplicates;
  `Export .dic…` writes only the scanned set. Neither replaces existing
  entries.
- Cancel uses the scan token; the reader lease stays exclusive.

## Recovery

- Strategies: Auto, Darkside, Nested, Static nested, Hardnested, FM11RF08S.
  Auto is the product path and takes no block inputs. The explicit strategies
  live in a collapsed `Advanced strategy options` expander with known
  block/type and target block/type; the hardnested table directory and
  acquisition cap are shared.
- Candidates: the key store plus the built-in common keys when enabled.
- A scrollable, read-only log under the action bar is the single progress and
  result surface. Phase transitions, per-sector keys, acquisition/filtering/
  brute-force state, and limitations are appended there; live counters are
  replaced in place. A one-second timer shows elapsed time.
- The footer keeps `Start recovery`, `Cancel`, and `Save verified keys`
  outside the scroll area, so the save action stays reachable with a long
  result. Cancel is available while the engine runs and is checked before the
  next reader exchange.
- `Start recovery` opens a neutral authorization dialog with the strategy,
  target, and selected reader. No worker or reader lease starts before the
  confirm response.
- Every key appears only after a fresh authentication; saving to the store is
  a separate action with `recovery` provenance and per-sector attribution.
- Engine limitations (darkside no-sample, FM11RF08S transport limits, missing
  hardnested tables) are logged verbatim and mirrored once in the status bar.

## Diagnostics

- Version, commit, build date, platform, supported driver prefixes,
  enumerated readers, selected-reader status, firmware of an opened reader,
  reader capability profile, resolved hardnested table path, and build
  profile/OS/architecture.
- The same fields drive `nfctl diag`; `nfctl diag --json` serializes the shared
  `SelfCheck` document.
- Reader refresh enumerates devices only; diagnostics performs no card I/O and
  shows no secrets.

## Activity

- `GtkColumnView` with `Time`, `Operation`, `Result`, and `Detail`.
- Session-only; warnings and backup paths appear in `Detail`. Key material and
  NDEF payloads are not logged.
- `Clear activity` empties the session list. Activity is the only audit surface; the left rail no longer duplicates it.

## Language

- English is the base language and fallback; `zh-CN` is complete.
- The language selector is in Settings and applies immediately. Relabeling
  happens in the existing window, preserving the current page, reader, card
  image, table selection, and open form values.
- The choice persists in `ui.json`. With no saved choice, `LC_ALL`,
  `LC_MESSAGES`, and `LANG` select the first language; everything other than a
  `zh` primary tag resolves to English.
- Protocol names, hex, UIDs, paths, counts, service diagnostics, and the
  activity log stay unlocalized. `nfctl` is English-only.

## Settings

- Fields: key store path, backup directory, hardnested table directory,
  common-key default, default dump format, and language.
- Preferences live in `$XDG_CONFIG_HOME/nfc-foundry/ui.json`, written
  atomically with mode `0600`. A malformed file is reported, not ignored.
- Saving validates paths, then reloads the key store and applies the backup
  directory and defaults. Advanced mode and recovery cancellation stay
  session-only.

## Diff

- Load before and after images without a reader.
- Classify equal/changed/unknown blocks with `nfc-classic::diff`; unknown
  bytes never count as equal.
- The table lists changed and unknown blocks with byte offsets and trailer
  field changes.
- The summary reports layout, changed/equal/unknown counts, trailer changes,
  and an identity change.

## Restore

- Load a dump, target the selected live reader, confirm, back up, restore, and
  read back.
- The service stops on the first failure unless the continue option is
  selected. Block 0 stays outside the ordinary write path.
- A result table lists every block as written, unchanged, unknown, or failed
  with the failure detail; the summary reports written blocks, trailers,
  unchanged blocks, skipped unknown blocks, the backup path, warnings, and
  failures.
- A source image with a different card identity is refused before the backup.
- After a restore the table is marked stale and writes are disabled until a
  fresh read.

## Dump save and load

- Complete 1K/4K image: default `Auto` resolves to Proxmark3 raw `.bin`.
- Partial image: `Auto` resolves to the project `.nfc-foundry.json`; raw,
  Proxmark3 JSON, and MCT choices are disabled with an explanation.
- Explicit choices: Proxmark3 raw, Proxmark3 `mfc v2` JSON, MCT text, NFC
  Foundry project.
- Default names: `card-dump.bin`, `card-dump.mfc.json`, `card-dump.mct`, and
  `card-dump.nfc-foundry.json`.
- Load detects the format from content and shows format plus warnings. A
  partial read is never reported as complete.

## NDEF

The tab is one message editor shared by Classic and Type 2. It is moved into
the active workspace and renders the active card family's NDEF facts.

- Card facts: a Classic card shows MAD sectors, mapping version, read/write
  state, capacity, and warnings; a Type 2 card shows product/UID, CC bytes,
  data-area size, capacity, NDEF state, locked pages, password protection, and
  warnings.
- Capacity card: used/total bytes, record count, free bytes, a meter, and
  `Valid NDEF` / `Over capacity` / `Invalid message` / `No records`.
- Records table: `#`, `Type`, `Preview`, `Size`. Selection drives the record
  inspector. `Load .ndef…`, `Reload from card`, `New message`, and
  `Add record…` sit in the table header.
- Record inspector: record ordinal, type, TNF, TYPE, ID, encoded size, payload
  hex, validity, `Edit record…`, `Remove`, and move-up/move-down.
- Preflight: `TLV valid`, `Fits user memory`, `Protected areas untouched`, and
  `Validate`. The protected note states that an NDEF write does not change
  Classic sector trailers/keys/access bits or Type 2 configuration/lock/
  password pages.
- Raw message: the `TLV 0x03` framing length and a selectable hex preview.
- Record kinds: Text (language, UTF-8/UTF-16), URI, MIME (media type and
  text/hex payload), Wi-Fi WSC (SSID, authentication, encryption, optional key
  and MAC), vCard, Bluetooth LE OOB, Wi-Fi DPP, Smart Poster, and a raw record
  form (TNF, TYPE, ID, payload). Unknown, external, empty, and malformed known
  records decode to the raw form and keep their exact bytes.
- The record dialog validates through the NDEF codec before changing the
  message; malformed input leaves the dialog open with the codec error.
  Template fields the form does not expose (WSC unknown attributes, Bluetooth
  LE extra AD structures, extra Smart Poster titles/nested records) are
  preserved on edit.
- Card reads, loaded dumps, and format reset the editor from that card. A dirty
  draft survives incidental refreshes; the header shows `Unsaved changes` /
  `No unsaved changes`. An explicit reload, load, or clear replaces the draft.
  `Save .ndef…` writes the editor message.
- Unformatted cards state that Format is required before writing. A Type 2
  format is destructive, removes proprietary TLVs, and uses the confirmation
  dialog.
- Type 2 write is refused for a partial data area, read-only CC, locked data
  page, capacity overflow, or an unchanged message. A password-protected data
  page uses the masked Protected access credentials; without them the write is
  refused before mutation. The service preserves other TLVs on a write.
- `Write NDEF` is enabled only for a live, complete read of a formatted,
  writable NDEF area with a non-empty, encodable message that fits the capacity
  and differs from the stored TLV. The confirmation names the record summary,
  sizes, project backup, and read-back.
- `Format NDEF area…` is destructive and names the area, the read-only choice,
  the backup, and the read-back. `Leave the NDEF area read-only` maps to the
  CLI `--read-only`.
- A loaded dump, partial image, read-only area, capacity overflow, identity
  mismatch, or busy operation disables mutation with a specific reason.
- Success refreshes the block table, NDEF page, card summary, and log from the
  service result without an extra card read; the editor baseline becomes the
  written message. Failure keeps the previous image and reports the error once
  in the toast and log.

## Workspace selector

The header selector is a compact `MenuButton` labelled `MIFARE Classic`,
`NFC Forum Type 2`, or `DESFire`. It switches the workspace stack and moves the
shared NDEF view into the Classic or Type 2 workspace. Reading a Classic or
Type 2 card selects the matching workspace automatically; DESFire is selected
manually and reads through its own application/file form. The manual selector
also covers loading dumps and inspecting pages without a live card.

## Main menu

The header `☰` menu groups application-level actions:

- Reader: Refresh readers.
- System: Diagnostics, Activity, Settings. These switch the hidden System page
  and show the header back button.
- About NFC Foundry: `AdwAboutDialog` with the application name, version,
  GPL-3.0-or-later license, and repository link.

## Copy and states

- English base copy in this slice; `zh-CN` is a completion requirement.
- Hover/focus/disabled/destructive/error states come from Adwaita; no custom
  colors.
- Errors appear in a toast and the session log; partial reads keep their
  warnings visible in the log and card summary.
- Escape dismisses the topmost popover or dialog.
