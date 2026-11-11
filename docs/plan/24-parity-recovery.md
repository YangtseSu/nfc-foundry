# Phase 24 plan: parity recovery

Status: planned 2026-11-11. This phase ports the remaining capabilities that
exist in the previous repository but have no Rust equivalent. It is deliberately
slice-based: each slice ships service, CLI, GUI, tests, and docs together.

## Why this phase exists

The Rust rewrite is ahead of the previous implementation in Type 2/NDEF,
Type 4 NDEF, native recovery, ACR122U support, dump interchange, CLI coverage,
and GTK workbench breadth. It is behind in a smaller set of service-level
capabilities:

- UID / manufacturer-block writes with a block-0 backup.
- An editable dump workbench with undo, revert, and partial-read merge.
- Selective restore, write-changes-only, and write preflight tokens.
- Verified-key capture and per-card key verification.
- Dump files that carry keys and key provenance.
- Reader polling, card removal events, reconnect, and capability reporting.
- Rich details/health and release self-check output.

The previous GUI did not expose UID writes, the workbench editor, write
preflight, diagnostics, or recovery. Those existed only in its service layer.
This plan treats them as product capabilities to finish, not as a request to
copy the old surface.

## Principles

- Service first. `nfc-app` owns the workflow; `nfctl` and GTK call the same
  methods with the same gates.
- No destructive path without backup, read-back, identity check, and explicit
  confirmation.
- Non-destructive data-model work lands before the destructive paths that
  depend on it.
- Every slice updates `docs/specs/`, `docs/notes/`, `docs/design/parity.md`,
  the hardware matrix when applicable, and the README status.
- No new external engine, C shim, or second frontend.
- No schema break: old key-store and dump-project files must still load.

## Slices and order

### S1: verified-key capture and dump key metadata

Spec: [24a-verified-keys.md](../specs/24a-verified-keys.md). Status: complete
2026-11-11. See [24a-verified-keys.md](../notes/24a-verified-keys.md).

Goal: a read, key scan, or recovery should leave the application knowing which
key authenticated each sector, and a saved dump should preserve that context.

Deliver:

- Key verification record keyed by card identity, sector, and key slot
  (A/B), with key value, source, and verification time.
- `ClassicReadResult` returns the authenticated sector keys. Read, key scan,
  and auto recovery merge verified keys into the active store and persist it
  atomically. Existing explicit save actions remain.
- Key-store JSON version 2 with a `verifications` list; version 1 still loads.
- Dump project version 2 with optional `keys`, `device`, `startedAt`,
  `finishedAt`, and per-block key sources. Version 1 still loads.
- Raw dump sidecar decision: if a sidecar is kept, it uses the
  `nfc-foundry` namespace and is written atomically; otherwise raw is
  explicitly lossy and the UI says so. The decision is recorded in the slice
  spec.
- Key import report: valid, duplicates, ignored, and line-numbered issues; a
  malformed line does not abort the rest of the dictionary.

Acceptance:

- A full read followed by an application restart still has the keys and
  verification records.
- A keyscan cancellation keeps already verified keys.
- A project dump round trip preserves keys and per-block provenance.
- Old v1 key stores and dump projects load unchanged; new files are version 2.
- No card write path is added in this slice.

### S2: editable workbench and selective restore

Spec: [24b-editable-workbench.md](../specs/24b-editable-workbench.md). Status:
complete 2026-11-11. See
[24b-editable-workbench.md](../notes/24b-editable-workbench.md).

Goal: restore the old workbench model that the rewrite dropped, then use it for
selective and changes-only writes.

Deliver:

- `nfc-app` workbench model: baseline and working dump, dirty blocks, hex/ASCII
  edit, undo, revert, mark-saved/mark-written, validation, and partial-read
  merge.
- Service APIs for load, save, edit, undo, revert, clear, and write preflight.
  The GUI receives snapshots only; no card I/O in view code.
- Selective restore: explicit block set and changes-only mode. Full-card
  restore remains the default.
- Write preflight: token, plan (blocks, trailers, access conditions), warnings,
  expiry, fingerprint recheck, and `--dry-run` CLI behavior. The GUI dialogs
  show the same plan before accepting.
- Block 0 remains non-writable in the ordinary workbench write path.

Acceptance:

- Workbench model tests cover edit, undo, revert, merge conflicts, dirty
  tracking, save/load, and post-write baseline updates.
- Selective restore writes only the requested blocks; changes-only writes only
  dirty blocks and refuses an empty plan.
- A stale preflight token or a changed card/dump fingerprint is refused before
  any write.
- GUI and CLI expose the same plan and gates.

### S3a: standalone UID / block-0 write

Spec: [24c-uid-write.md](../specs/24c-uid-write.md). Status: complete
2026-11-11. See [24c-uid-write.md](../notes/24c-uid-write.md). Hardware row is
pending.

Goal: a guarded, read-back-verified UID write for supported magic cards only.

Deliver:

- `nfc-app` UID service with preflight, block-0 backup, write, reselect, and
  full block-0/BCC verification.
- Standard CUID/Gen2 block-0 write where the card accepts it, with ACR122U
  Gen1A as the first protected-writer backend.
- 4-byte UID MIFARE Classic 1K only. The service verifies sector 0 access
  conditions, the current block-0 UID/BCC, and the target UID's BCC.
- Exact typed UID confirmation, explicit Advanced-mode requirement, and a
  pre-write backup record containing the old and new block 0.
- `nfctl classic uid` and the GTK dialog use the same service.
- PN532 Gen1A is not advertised by this slice unless a native PN532
  `ManufacturerWriter` is implemented and hardware-validated in the same
  change. External `nfc-mfsetuid` remains prohibited.

Acceptance:

- Mock tests cover success, unchanged UID, wrong sector-0 key, invalid access
  bits, write-response uncertainty, card disappearance after write, and
  cancellation.
- A hardware run on an owned magic card changes the UID, reselects, and
  verifies the complete block 0 and BCC.
- An ordinary immutable-UID card fails safely before any write.

### S3b: restore with UID

Goal: restore a complete dump and write the target UID last.

Deliver:

- The full-restore preflight validates the post-restore sector-0 access
  conditions and the sector-0 key that will survive.
- The plan includes the UID target and old block 0; confirmation includes the
  exact UID.
- Restore writes ordinary blocks and trailers first; the UID write runs only
  after the dump restore is verified.
- Failure after a partial restore reports which blocks and the UID state were
  committed, and never claims success.

Acceptance:

- A dump restore plus UID target completes on an owned magic card and is
  verified by reselection and a full read.
- A mismatch in card identity, dump layout, post-restore access, or UID
  confirmation stops before the first write.
- The result distinguishes dump failure from UID failure.

### S4: reader lifecycle and capability model

Goal: make card presence and reader recovery an application responsibility,
not a GUI refresh button.

Deliver:

- `nfc-app` reader monitor: poll interval, card-appeared/removed events, bounded
  removal misses, status (`connecting`, `polling`, `busy`, `recovering`,
  `error`), and reconnect with backoff.
- One exclusive operation lease; polling never touches the reader during an
  operation.
- `ReaderCapabilities` is populated by each driver: Classic read/write, Type 2
  read/write, raw bytes/bits, manufacturer write. Recovery and UID gates use
  the capabilities instead of assuming them.
- GUI reader panel shows connection state and capabilities; CLI `scan --watch`
  and normal operations use the same monitor.
- Card removal invalidates the live image and disables writes immediately.

Acceptance:

- Mock transport tests cover card appear, card removal after two misses,
  transient timeout, reconnect, and operation/poll mutual exclusion.
- GUI tests cover removal invalidating the image and disabling write actions.
- A driver without `manufacturer_write` cannot reach the UID service.

### S5: surface and diagnostic gaps

Goal: close the lower-risk UI and release-visible gaps.

Deliver:

- Card Details/health view: UID, length, ATQA, SAK, inferred type, dump source,
  layout, completeness, validation errors/warnings, key coverage, and dirty
  state.
- Key catalog: delete custom entries, show built-in/common keys, and surface
  the import report from S1.
- MIFARE Mini identification from SAK `0x09` (identity only; no layout claim).
- `nfctl diag --json` and GUI diagnostics with version, commit, build date,
  OS/arch, driver list, reader firmware when opened, hardnested table status,
  and reader capabilities.
- Recovery authorization confirmation before any attack starts.

Acceptance:

- Details and health values match the dump/key model.
- Delete removes custom keys only and persists the store.
- Mini cards report their type without claiming read/write support.
- Self-check JSON is stable and tested; no key material or dump payload is
  included.

## Explicit non-goals

- Type 4 CLI/GUI exposure; it stays hardware-gated.
- Type 2 `GET_VERSION`, `READ_CNT`, `FAST_READ`, `READ_SIG`, and authenticated
  access after AUTH0.
- Full clone and factory-format workflows. They become possible after S3, but
  need their own spec and hardware record.
- Type B, high-rate, and the rest of phase 13.
- External recovery engines or `nfc-mfsetuid`.
- Windows/macOS, packaging changes beyond the new diagnostics fields.

## Verification policy

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- GTK scenarios under `NFC_FOUNDRY_GUI_TESTS=1` for changed screens.
- `reuse lint`
- Hardware tests only with the existing environment gates; every new
  destructive path needs a recorded run before it is advertised.

## First implementation slice

S1, S2, and S3a are complete. S3b is next. S3a reuses the preflight token,
backup, and read-back patterns from S2; S3b requires S3a. S4 and S5 are
independent of S3 and can be scheduled after S1.
