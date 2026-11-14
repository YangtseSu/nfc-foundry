# Note: 30.2/30.3 dump sidecars and candidate export

Plan: [30-workbench-file-parity.md](../plan/30-workbench-file-parity.md). Spec:
[30-workbench-file-parity.md](../specs/30-workbench-file-parity.md).

Session 1 of phase 30 implements the file-interchange half. 30.1 (sector
grouping and ASCII) remains pending.

## Implemented

- `nfc-classic::exchange`: `SidecarLoad`, `encode_sidecar`, and `parse_sidecar`
  for `<image>.nfcx.json`. The parser accepts the current key shape, the old
  `keyA`/`keyB` shape, and legacy `blocks` after a layout/count check (object
  maps also validate every block address); unknown versions parse as unusable
  with a warning.
- `nfc-app::classic::dump`: `SIDECAR_SUFFIX`, `sidecar_path`, `write_sidecar`,
  `adopt_sidecar`, and `SidecarAdoption`. Sidecars are written through the
  existing atomic `0600` writer. Project images skip them. Adoption checks
  layout, completeness, and card identity before replacing metadata or the
  card; mismatches and parse failures only warn.
- `nfc-app::classic::keys`: `CandidateSource`, `KeyCandidate`,
  `KeyStore::candidate_keys`, and `KeyStore::export_candidate_dic`. The union is
  unique, sorted by uppercase hex, keeps stored provenance, and never mutates
  the store.
- `nfctl`: `classic dump --no-sidecar`, `sidecarPath` in the dump result,
  sidecar adoption in `classic restore` and `card health`,
  `classic keys export --all-candidates`, and `keys`/`scope` in the export
  result.
- GUI: save writes the sidecar for image-only formats and logs its path; load
  and restore load apply a matching sidecar; the keys page adds
  `Export all candidates…`. New labels are in the `zh-CN` catalog.
- Docs: CLI contract, workbench/classic design, README, plan status, and
  testing record.

## Decisions

- The sidecar suffix is `<image>.nfcx.json` per phase 30. This supersedes the
  phase 19a no-`nfcx` filename rule and the phase 24 `nfc-foundry` namespace
  rule for the sidecar only. Project images remain `*.nfc-foundry.json`.
- The sidecar `layout` value uses the existing `classic_1k` / `classic_4k`
  spelling, not the plan example's `classic1k`.
- Adoption treats the image as authoritative: a sidecar supplies metadata and
  a missing card identity only when layout, completeness, and known card
  identity agree. A raw/MCT image with no identity adopts the sidecar card so
  key verifications are scoped to the right card.
- `classic diff` does not consult sidecars; it compares image bytes only.
- Candidate export writes exactly one sorted `.dic` line per unique key. JSON
  adds `source` (`stored`/`builtin`) and `scope`, and the CLI never prints key
  material.

## Verified

Hardware-free test runs on 2026-11-15:

```sh
cargo test -p nfc-classic --test exchange            # 21 passed
cargo test -p nfc-app --lib classic::dump            # 8 passed
cargo test -p nfc-app --lib classic::keys::tests::candidate
cargo test -p nfctl --test cli image_exports_write_and_consume_sidecar_metadata
cargo test -p nfctl --test cli keys_export_all_candidates
cargo test -p nfctl --test cli_contract key_candidate_export
env -u WAYLAND_DISPLAY NFC_FOUNDRY_GUI_TESTS=1 xvfb-run -a \
  cargo test -p nfc-foundry --bin nfc-foundry \
  app::tests::gtk_workbench_scenarios -- --exact             # 1 passed, ~160 s
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                                      # all passed
reuse lint                                                  # compliant
```

## Limitations

- Sidecars are not validated against the image beyond layout, completeness, and
  card identity. Keys are recorded as verified for that card because they
  authenticated during the original read; the dump cannot re-authenticate them.
- `classic dump` still writes the paired `.dic` key file in addition to the
  sidecar. That duplication predates this slice and is unchanged.
- The GUI has no `--no-sidecar` equivalent; image-only saves always write the
  sidecar. A failed sidecar write is logged and does not fail the save.
