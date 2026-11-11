# Phase 24 S5: surface and diagnostic gaps

Status: complete 2026-11-12.

## Goal

Close the remaining UI and release-visible gaps without adding a card path:
one details/health model over the loaded image and key store, a key catalog
that shows built-ins and deletes stored entries, MIFARE Mini identification, a
deterministic self-check shared by `nfctl` and the GUI, and an authorization
step before active recovery starts.

## Card details and health

`nfc_app::health` owns `CardHealth`, built by `classic_health(dump, keys,
dirty_blocks)`. The model is hardware-free and used by both surfaces:

- `identity`: UID text, UID length, ATQA, SAK, and inferred `CardType` from the
  image's recorded card identity. An image without identity reports none.
- `layout`, `readable_blocks`/`total_blocks`,
  `readable_sectors`/`total_sectors`, `skipped_sectors`, and `complete`.
- `errors`: invalid image geometry, invalid trailer access bits, and a BCC that
  does not match the recorded UID.
- `warnings`: an incomplete image, unreadable trailer bytes, a BCC that cannot
  be checked, and per-block read errors.
- `key_coverage`: `sectors`, `any_sectors`, `key_a_sectors`, `key_b_sectors`,
  and `both_sectors`, from `KeyStore::verifications_for` for the image's card.
- `dirty_blocks`: the workbench dirty set, sorted.
- `dump_keys`, `device`, `started_at`, and `finished_at` from the image
  metadata.

The GUI Details tab renders this model plus the image source (live card or file
path) and whether the image is live or inspection-only. `nfctl card health`
renders the same model from a saved image, optionally merging the image's
metadata keys into a temporary view of the key store. The CLI has no workbench,
so its dirty set is always empty.

## Key catalog

- Every stored entry is shown with sector, A/B, key, and provenance.
- Built-in common keys are shown as `all sectors`, `A+B`, `built-in` when the
  same key is not stored. They are not store entries and cannot be deleted.
- Delete removes exactly the selected stored entry, refreshes the table, and
  persists the store. `nfctl classic keys delete` performs the same operation by
  key, sector, and type.
- `nfctl classic keys list --builtin` prints the built-in candidates; without
  the flag it reports their count.
- The import report is shown in the key page: added, duplicates, ignored, and
  each line-numbered issue. The same counts are logged and returned by the CLI.

## MIFARE Mini

SAK `0x09` (cascade bit ignored) infers `CardType::MifareMini`. The type is
reported by `nfctl card info`, the GUI reader rail, and diagnostics. There is
no layout mapping: every read, write, restore, and UID path refuses a Mini card
with an explicit message. The hardware matrix gains no claim.

## Self-check

`nfc_app::diagnostics::SelfCheck` is serialized by `nfctl diag --json`. Fields:

- `app`: `name`, `version`, `commit`, `build_date`.
- `platform`: `os`, `arch`.
- `drivers`: `name`, `conn_string_prefix`.
- `readers`: `available`, `error`, `devices` (`name`, `conn_string`),
  `selected`, `status`, `firmware`, and `capabilities` (`classic_read`,
  `classic_write`, `type2_read`, `type2_write`, `raw_bytes`, `raw_bits`,
  `manufacturer_write`, `recovery_parity`).
- `hardnested_tables`: `available`, `path`, `error`.

`commit` and `build_date` come from `nfc-app/build.rs`, overridable with
`NFC_FOUNDRY_GIT_COMMIT` and `NFC_FOUNDRY_BUILD_DATE`; `SOURCE_DATE_EPOCH` is
honoured. Missing git, date, or overrides produce `unknown`.

The report enumerates readers and reports the firmware of a reader that is
currently open. It never selects, reads, or authenticates a card. It contains
no card identity, key material, sector data, or dump payload. The GUI
Diagnostics page shows the same fields plus platform, firmware, capabilities,
and the selected-reader status.

## Recovery authorization

`nfctl recover` prompts before the engine starts unless `--yes` is passed; a
non-interactive run without `--yes` fails with the confirmation error. The GUI
shows a neutral authorization dialog with the strategy, target, and selected
reader; no recovery thread, reader lease, or engine call starts before the
`confirm` response. Recovery remains authenticate/read-only.

## Acceptance

- Health values match `Dump`, `KeyStore`, and the dirty set; tests cover a
  complete image, a partial image, invalid access bits, a BCC mismatch, key
  coverage, and an image without recorded identity.
- Delete removes one stored entry and persists the store; built-in rows cannot
  be deleted.
- Mini reports `MIFARE Mini` and `Service::read_card` refuses it before any
  block read.
- The self-check JSON key set is tested, and the test rejects key material and
  dump fields.
- `nfctl recover` without `--yes` refuses; the GUI test proves no busy state is
  entered before the dialog confirmation.
