# Spec: phase 19d recovery and system surfaces

Status: complete 2026-11-11. Plan:
[19d-recovery-system.md](../plan/19d-recovery-system.md). Update 2026-11-12:
FM11RF08S is no longer detection-only; phase 25.3 added backdoor recovery.
See [25c-fm11rf08s.md](25c-fm11rf08s.md).

## Key scan

- Candidates: key store, common keys, and a text dictionary.
- Types: A, B, or both. The service verifies a found key by reading the
  sector trailer.
- Results are per sector, with attempts and verified counts.
- Discovered keys merge into the key store; duplicates are ignored. `.dic`
  export remains available.
- Cancel uses a `CancelToken`; no write occurs.

## Recovery

- Strategies: Auto, Darkside, Nested, Static Nested, Hardnested, FM11RF08S.
- Inputs: known block/key type, target block/key type, tables directory, and
  max nonces where applicable.
- Progress uses `RecoveryProgress` via a `ProgressSink`; events reach GTK
  through a channel and are rendered on the main context.
- Cancel is available during acquisition, filtering, and brute force.
- A recovered key is shown only after service verification; save to the store
  is explicit.
- Limitations are rendered verbatim from the engine result:
  darkside no-sample, FM11RF08S detection-only, missing hardnested tables.
- Recovery is read/authenticate-only. No card mutation.

## Diagnostics

- Version, supported driver prefixes, reader enumeration, and hardnested table
  resolution.
- No card I/O, network, telemetry, or secret material.

## Activity

- Session-only list: timestamp, operation, result, warnings, backup path.
- Clear action.
- Key material and NDEF personal payloads are not logged by default.

## Settings

- `$XDG_CONFIG_HOME/nfc-foundry/ui.json`, atomic write with `0600`.
- Fields: key store path, backup directory, hardnested tables directory,
  use-common-keys toggle, default dump format.
- Missing file = defaults; malformed file = reported error, not silently
  discarded.
- Advanced mode and recovery cancellation are session-only.

## Tests

- Mock keyscan and recovery fixtures, including cancellation and verification.
- UI tests for progress/cancel, save-key, settings persistence, diagnostics,
  and activity redaction.
- Gated `NFC_FOUNDRY_GUI_TESTS=1` + Xvfb; normal tests stay hardware-free.
