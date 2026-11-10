# Phase 19a plan: Classic completion and Proxmark3 interchange

Status: complete 2026-11-11. This was the work package after the phase 19
first slice. Implementation record:
[notes/19a-classic-pm3.md](../notes/19a-classic-pm3.md).

Starting point: `feat/gtk4-ui` at `403e0f7` (clean). The GTK shell, real Classic
read/table/inspector, verified block write, project save/load, and read-only
Classic NDEF view are in place.

Goal: finish the Classic workbench to a usable, safe state and make dump/key
interchange compatible with Proxmark3. Do this before NDEF writes, Type 2/Type
4 screens, localization, or packaging.

## Workstream A: file contract first

The file contract changes once, before the Keys and dump UI depend on it.

### A1. Remove legacy `nfcx` naming

- No new file name, content field, UI label, or documentation uses `nfcx`.
- The current Rust project format already has no `nfcx` field; the old Go app
  defaulted to `card-dump.nfcx.json`. New saves never reuse that name.
- New default names:
  - `card-dump.bin` — Proxmark3 raw image;
  - `card-dump.mfc.json` — Proxmark3 `mfc v2` JSON;
  - `card-dump.mct` — MCT text;
  - `card-dump.nfc-foundry.json` — lossless project image.
- Old `.nfcx.json` files are legacy. Raw `.bin` and PM3 JSON files load
  normally. Add a legacy sidecar import only if a user needs an old metadata
  sidecar; do not make it a new export.

### A2. Dump export policy

- Complete 1K/4K image: default to **Proxmark3 raw `.bin`**. It is exactly
  PM3's raw dump size and works with PM3 restore.
- Need sector keys/access text: offer **Proxmark3 JSON** `mfc v2`
  (`.mfc.json`). Keep `FileType: "mfc v2"`, `Created: "NFC Foundry"`, and the
  existing `Card`/`blocks`/`SectorKeys` fields.
- Partial image: only the **project** format (`.nfc-foundry.json`). Raw and
  PM3 JSON cannot represent unknown bytes; never zero-fill them.
- MCT (`.mct`) remains an explicit export/import choice, not the default.
- Add an `auto` choice for CLI and GUI:
  - complete → raw;
  - partial → project.
- Backups continue to use the project format so block masks, statuses, and
  errors survive.
- GUI `Save dump…` gets a format selector and a completeness-sensitive
  default. Saving a partial image as a format that cannot represent it is
  disabled with an explanation.
- CLI `classic dump --format` defaults to `auto`; explicit `raw`, `proxmark3`,
  `mct`, and `project` remain available.
- Load keeps format detection; report warnings for identity-less raw/MCT
  images.

### A3. Key interchange

- Add PM3 `.dic` export: one uppercase 12-hex key per line, unique keys in
  candidate order.
- `.dic` import follows the PM3 dialect: blank lines, `#` comments, and inline
  comments are accepted; one 12-hex key per line.
- Keep the JSON key store as the internal source of truth for sector, A/B/any,
  and provenance. `.dic` cannot represent those fields; document that export
  is lossy and import adds wildcard `any` keys.
- Add `nfctl classic keys export --format dic|json` and the same action in the
  GTK Keys screen.
- A raw key-array `.bin` format is optional later; it is not required for the
  PM3 dictionary workflow.

### A4. Interchange tests

- Golden PM3 `mfc v2` JSON fixture: import, re-export, compare semantics.
- `.dic` tests: comments, inline comments, blank lines, duplicates, lowercase,
  malformed line.
- Raw 1K/4K round trip; partial project round trip; reject raw/PM3 export of a
  partial image.
- Assert new default names and `Created: "NFC Foundry"`; no `nfcx` anywhere.
- No hardware required.

## Workstream B: Classic safety and screens

### B1. Write identity guard

- Add an expected card identity to the service write request (preferred) and
  compare it with `operation.card()` inside the operation. A UI pre-scan alone
  leaves a swap race.
- On mismatch return `NfcError::CardChanged` before backup or card write.
- Pass the loaded dump's card from the GUI for block writes and restores. Keep
  `None` for CLI writes that intentionally target the current card.
- Apply the same check to restore and Classic NDEF writes where a loaded image
  provides the identity.
- Tests: mock card swap between read and write; no backup is created.

### B2. Keys screen

- List the internal key store: sector, A/B/any, key, provenance.
- Import `.dic`/text/JSON; add one key; save the store.
- Export `.dic` for PM3.
- Show the lossy nature of `.dic` export and the wildcard behavior on import.
- Use the same store for reads and writes; no second key source.

### B3. Inspector completion

- Decode sector-trailer access bits and show the four access groups.
- Decode value blocks.
- Show BCC status for the manufacturer block.
- Show the block's access group and any trailer/access warnings.
- Keep the existing hex editor and write gates unchanged.

### B4. Diff screen

- Load before/after dumps without a card.
- Classify equal/changed/unknown blocks; show trailer details.
- Use `nfc-classic::diff`; no duplicate comparison logic.

### B5. Restore screen

- Load a dump, target the selected reader, confirm, back up, restore, read
  back.
- Keep the service's stop-on-first-failure policy and explicit continue
  option.
- Block 0 remains outside the ordinary write path.
- Report per-block results and the backup path.

### B6. Dump UI

- Save/load raw, PM3 JSON, MCT, and project images.
- Default by completeness as in A2.
- Show format detection and warnings; a partial read is never reported as
  complete.

### B7. GTK UI tests

- Add a mock/test entry point so UI tests do not touch hardware.
- Run on headless Xvfb.
- Cover: selection updates inspector; block 0/unchanged/no-live-card gates;
  trailer confirmation; reader-refresh invalidation; identity refusal; save
  format selection; diff/restore confirmation and cancel.
- Keep unit tests for presentation/format helpers.

## Order

1. A1–A4: naming, dump policy, `.dic`, and tests.
2. B1: write identity guard (safety before more write surfaces).
3. B2: Keys screen (uses the new `.dic` path).
4. B3: inspector completion.
5. B4–B6: diff, restore, dump UI.
6. B7: UI tests.
7. Next phase: NDEF document/schema, CLI JSON, then GTK NDEF writes.

## Acceptance

- Complete dump saves as raw `.bin` by default; PM3 JSON export has the
  expected `mfc v2` fields; partial dump saves only as project JSON with a
  clear warning.
- `.dic` export/import matches PM3 dictionary behavior.
- No new saved file or UI label contains `nfcx`.
- A card swap between read and write is refused before backup/write.
- Keys, diff, restore, and inspector screens pass their UI tests.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
  warnings`, `cargo test --workspace`, and `reuse lint` pass.
- Hardware verification remains read-only unless a write is explicitly
  authorized.

## Verification (2026-11-11)

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
  warnings`, `cargo test --workspace`, and `reuse lint` pass.
- GTK scenarios pass under Xvfb:
  `NFC_FOUNDRY_GUI_TESTS=1 xvfb-run -a cargo test -p nfc-foundry
  gtk_workbench_scenarios`.
- Hardware was not touched; no write hardware authorization was needed for
  this work package.

## Non-goals

- NDEF writes/format, Type 2/Type 4 screens, `zh-CN`, packaging, and
  Windows/macOS. Those stay in phase 19/later phases.
- A second GUI frontend.
- Replacing the lossless project format with PM3 formats.

## First files to open in the new session

- `crates/nfc-classic/src/exchange.rs` — dump formats and PM3 JSON.
- `crates/nfc-app/src/classic/keys.rs` — `.dic` import/export.
- `crates/nfc-app/src/classic/write.rs` — expected-card guard.
- `crates/gui/src/app.rs` — save format selector, Keys, diff, restore.
- `crates/nfctl/src/main.rs` — `classic dump --format auto`, keys export.
- `docs/design/workbench.md` — screen contract.
