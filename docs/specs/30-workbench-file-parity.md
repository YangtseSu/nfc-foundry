# Spec: 30 file parity, sidecars and candidate export

Status: 30.2 and 30.3 implemented 2026-11-15. 30.1 pending.

Plan: [30-workbench-file-parity.md](../plan/30-workbench-file-parity.md).

## Scope of this record

This spec covers the file-interchange half of phase 30:

- 30.2: `<image>.nfcx.json` sidecars for image-only dump exports;
- 30.3: an explicit stored-plus-built-in key candidate export.

30.1 (sector grouping and the ASCII column) is not implemented here and stays
pending in the phase plan.

## Naming decision

Phase 19a removed the old application's `nfcx` project name, and phase 24
required any raw sidecar to use the `nfc-foundry` namespace. Phase 30
explicitly reintroduces `.nfcx.json` as a **sidecar suffix**, not as a project
or image name. The current decision is:

- project images stay `*.nfc-foundry.json`;
- image-only sidecars are `<image path>.nfcx.json`, e.g.
  `card-dump.bin.nfcx.json`;
- no other generated file uses `nfcx`.

This supersedes the phase 19a/24 filename rules only for the sidecar suffix.
The old application's `card-dump.nfcx.json` project file remains legacy; it is
not produced and is not auto-detected as a dump.

The sidecar `layout` value uses the existing project spelling `classic_1k` /
`classic_4k`, not the plan example's `classic1k`.

## Sidecar contract

Fields:

```json
{
  "version": 1,
  "complete": true,
  "layout": "classic_1k",
  "card": {"uid": "04A2248A", "atqa": "0004", "sak": 8},
  "device": "mock:classic",
  "startedAt": 1762819200,
  "finishedAt": 1762819205,
  "keys": [
    {"sector": 0, "type": "A", "key": "FFFFFFFFFFFF", "source": "read_context"}
  ]
}
```

- Metadata only. The image bytes are authoritative and sidecar fields never
  regenerate or patch a block.
- Version 1 is current. Any other or missing version parses but is reported as
  unusable with a warning; metadata is not applied.
- `keys` accepts the project shape (`type` + `key` + optional `source`) and the
  old shape (`keyA` / `keyB`). Old-shape entries map to separate A/B
  `DumpKey` values. Unknown key source labels fall back to `manual` through
  `KeyStore::record_dump_keys`.
- A legacy `blocks` member is accepted only when its count matches the declared
  layout (and, for an object map, every block address is in range); its content
  is ignored. A wrong count, an invalid block number, or a non-array/non-object
  value rejects the sidecar.
- Duplicate sector/key slots, out-of-range sectors, and malformed keys are
  rejected by the shared dump metadata validation.

## Write behavior

- Applies to `raw`, `mct`, and `proxmark3` exports. `project` keeps the same
  metadata inline and never gets a sidecar.
- Enabled by default. `nfctl classic dump --no-sidecar` disables it; the GUI
  always writes it for image-only formats.
- Written atomically next to the image through `nfc-app`'s atomic writer with
  file mode `0600`.
- A sidecar failure is a warning. The image write stays successful and the
  command exits 0.
- `nfctl classic dump` reports `sidecarPath` in the envelope and prints
  `sidecar: <path>` in human output. A project export reports `null`.
- The paired `.dic` key file behavior from phase 19a is unchanged.

## Load behavior

- Project JSON is authoritative and never consults a sidecar.
- Raw, MCT, and Proxmark3 loads look for `<path>.nfcx.json`.
- A missing sidecar is normal and silent.
- A malformed sidecar, an unreadable file, an unknown version, or a
  layout/completeness/card mismatch produces a warning and loads the image
  without metadata. The image bytes are never rejected because of the sidecar.
- On a match, the sidecar supplies the card identity when the image has none
  (raw and MCT), plus device, timestamps, and sector keys. A sidecar key set is
  recorded through the existing `record_dump_keys` path in the CLI health and
  restore flows and the GUI dump/restore loads.
- `nfctl classic restore` and `nfctl card health` print the sidecar path and
  key count. `classic read --block` is a single-block read with no image path,
  so it has no sidecar field. `classic diff` stays an image comparison and does
  not apply sidecars.

## Candidate export

- `KeyStore::candidate_keys()` returns the unique union of stored keys and the
  57 built-in common keys, sorted by uppercase hexadecimal. Stored provenance
  wins when a key is both.
- `Export all candidates…` in the GUI keys page writes the `.dic` candidate
  list next to the existing stored-only `Export .dic…`.
- `nfctl classic keys export --all-candidates` keeps the existing `dic`/`json`
  formats:
  - `.dic`: one uppercase key per line, sorted;
  - JSON: `{"version":1,"scope":"all_candidates","keys":[{"key","source"}]}`
    with `source` equal to `stored` or `builtin`.
- The default remains stored-only. The result reports `keys` and `scope`; the
  GUI/CLI never write built-in keys into the persistent store and never log key
  values.
- Machine modes still require `--output`.

## Tests

- `crates/nfc-classic/tests/exchange.rs`: sidecar round trip, legacy
  `keyA`/`keyB`, legacy blocks accepted/ignored, unknown version, duplicate
  slots, sector bounds, missing keys.
- `crates/nfc-app/src/classic/dump.rs`: path suffix, atomic `0600` write,
  project skip, missing sidecar, raw card/key restore, corrupt sidecar,
  layout/card mismatch, unknown version.
- `crates/nfc-app/src/classic/keys.rs`: union/dedup, built-in-only and stored
  provenance, deterministic order, `.dic` shape, no store mutation.
- `crates/nfctl/tests/cli.rs`: raw dump writes a `0600` sidecar, `card health`
  consumes it, `--no-sidecar`, project skip, sorted `--all-candidates` `.dic`
  and JSON, stored-only default, store unchanged.
- `crates/nfctl/tests/cli_contract.rs`: candidate export report fields and no
  key material in machine output.
- GTK scenario: sidecar metadata and verification records on load, project
  precedence, mismatched-sidecar rejection, export-all button and store
  immutability.

## Non-goals

- Restoring the old workbench shell or changing card-write gates.
- Writing keys or metadata into a card.
- A new dump image format.
- Making sidecars authoritative over image bytes.
