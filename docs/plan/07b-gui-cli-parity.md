# Phase 07b: GUI parity with the CLI

## Goal

Every user-facing operation exposed by `nfctl` is available in the MyGo GUI.
Both surfaces call the same application services and apply the same preflight,
confirmation, backup, cancellation, and read-back verification rules. The GUI
adds no protocol logic and no second implementation of a workflow.

## Original gaps (closed)

| CLI surface | GUI today | Work |
| --- | --- | --- |
| `recover nested`, `recover darkside`, `recover hardnested` | No entry point; the Keys tab only scans the catalog | Add recovery modes, engine availability, authorization, progress, and cancellation |
| `apdu --raw` | No ISO-DEP card family; unknown cards show only Details | Add ISO-DEP detection and a diagnostic APDU panel |
| `desfire info`, `apps`, `files`, `read` | No DESFire family or view | Add read-only DESFire information, application, file, and file-read views |
| `ndef read`, `ndef write` on Type 4 | GUI recognizes only Classic and Type 2 | Add Type 4 detection and reuse the NDEF read/write UI |
| `diag` | Only `nfc-foundry --self-check` from a terminal | Add a GUI diagnostics view for the same report |

Already covered: reader list/refresh, card facts, Classic read/write/dump/
restore/keyscan, Type 2 read/write/dump/restore/copy/erase/lock/password, NDEF
on Type 2 and Classic, `uid set`, key catalog add/delete/import/export, and
task cancellation. Destructive Classic operations remain behind Advanced mode.

Phase 10 key lifecycle surfaces:

| CLI surface | GUI today |
| --- | --- |
| `classic dump --scan`, `classic restore --scan` | **Scan keys before reading** checkbox and the Keys tab **Scan keys** action |
| `recover --name`, `--key-set` | Recovery review modal with a key-set name and per-key names |
| `recover --no-save` | GUI always defers persistence to the review modal; **Discard** is the equivalent |
| `keys rename` | **Rename** action on the selected key |
| `keys sets` and `keys set create`, `rename`, `delete`, `export` | **Key sets** panel with save-verified, rename, delete, and export actions |
| `keys reload` | **Reload key store** button |
| `keys set export --format dic` or `keys` | Export format selector and native save dialog |

CLI-only by design: `--json`, `--jsonl`, `--show-keys`, `--timeout`, exit
codes, and shell pipelines. The GUI needs equivalent user actions, not those
flags: preflight dialogs for dry runs, reveal controls for keys, a visible
timeout state, and a cancel action.

## Work

### 1. Key recovery

- Expose `StartMFoC`, `StartMFCUKPipeline`, and `StartHardnested` through
  `desktop.Backend`; prefer the existing five-step `StartKeyRecovery` service
  where it gives one consistent progress model.
- Add a recovery panel on Classic cards: mode selection, engine availability
  and version, seed-key state, reader/capability errors, authorized-card
  confirmation, and the CLI-equivalent `--yes` gate.
- Show engine stdout/stderr, per-step progress, and a cancel action. Stop card
  polling and release the reader before the engine starts; reopen and verify
  afterwards.
- Persist recovered keys through the existing key store and refresh the key
  matrix and workbench when verification finishes.

### 2. ISO-DEP detection and raw APDU

- Extend `cardFamily` with ISO/IEC 14443-4, DESFire, and Type 4 families; add
  tabs only when the card and reader expose the required capability.
- Add a diagnostic APDU panel with hex input, send, response data, status
  words, and copy. Every send requires an explicit action; do not add
  convenience buttons for write commands.
- Keep the raw APDU path visibly diagnostic. It must not imply that arbitrary
  APDU writes are validated or reversible.

### 3. DESFire read-only

- Add information, application, file-list, and file-read views selected by AID
  and file ID.
- Keep the phase read-only: no key changes, authentication writes, formatting,
  or file writes.
- Report unsupported or unreadable files as errors, never as empty data.

### 4. Type 4 NDEF

- Detect the Type 4 NDEF application and capability container, then reuse the
  existing NDEF rendering and editor components.
- A write requires a full-image backup, a typed confirmation, and read-back
  verification. Show the capability container before the first write.
- Update the card-support matrix and hardware-validation notes when the path is
  tested.

### 5. Diagnostics

- Add a GUI diagnostics view or menu action that runs the same
  `internal/diagnostic` report as `nfctl diag` and `nfc-foundry
  --self-check`.
- Show reader/runtime/engine availability and capability results without
  touching a card.

### 6. Parity guard

- Keep the matrix in this phase current for every CLI command.
- Add a UI test for every new GUI workflow, plus a check that the CLI help
  surface and the matrix have no unclassified entries.
- A CLI change that adds or changes a user-facing operation must update the
  GUI and this matrix in the same change.

## Milestones

1. Recovery GUI with all three engines and the five-step progress model.
2. ISO-DEP family, raw APDU, and read-only DESFire.
3. Type 4 NDEF read and guarded write.
4. Diagnostics view.
5. Parity audit: no unclassified CLI commands, tests and documentation green.

## Acceptance

- Every command in `nfctl --help` has a GUI equivalent, except the
  machine-output and automation flags listed above.
- Recovery shows the same authorization, engine, seed-key, and safety rules as
  the CLI and leaves the reader usable after success, failure, and cancellation.
- APDU, DESFire, and Type 4 views appear only for compatible cards and readers;
  unsupported operations are shown as unavailable, not failed.
- Type 4 writes have backup, explicit confirmation, and read-back verification.
- English and Simplified Chinese strings are complete; `go tool mygo i18n
  check .` passes.
- `go test ./...`, UI smoke tests, `go vet ./...`, and `reuse lint` pass.
- Hardware-specific paths have an explicit hardware test record or a stated
  validation gap.

## Non-goals

- No protocol or device logic in `internal/desktop`.
- No `--json` or `--jsonl` output mode in the GUI.
- No DESFire write, key-change, or format workflow in this phase.
- No promise that arbitrary raw APDUs are safe or reversible.
- No card-emulation UI; that remains the conditional Phase 09.

## Risks and dependencies

- External engines are PN532 UART-only for `darkside`; `nested` and
  `hardnested` need at least one verified seed key. The GUI must state these
  limits before starting and must not retry silently.
- Recovery releases the reader while the GUI is polling. The device manager
  must stop polling, keep the UI blocked, and recover cleanly on cancellation.
- Type 4 NDEF writes and DESFire reads need hardware validation on the
  available readers; hardware-free fixtures cover framing and validation only.

## Completion

Closed on 2026-11-08. The GUI now exposes key recovery (automatic, nested,
darkside, hardnested), raw APDU diagnostics, read-only DESFire views, Type 4
NDEF read/write through the shared NDEF editor, and the self-check report from
the application menu. See `docs/notes/07b-gui-cli-parity.md`.

APDU, DESFire, and Type 4 NDEF still lack hardware validation; the GUI states
that limitation and does not advertise support beyond the capability matrix.
Machine-readable CLI output remains CLI-only by policy.
