# Phase 19d plan: recovery and system surfaces

Status: complete 2026-11-11. Depends on 19a's Classic workbench and the same
worker/test harness.

Goal: expose the remaining Classic operations and the system surfaces:
keyscan, native recovery, diagnostics, activity, and settings.

## Scope

### Key scan

- Add a Key Scan action/page for MIFARE Classic.
- Candidate sources: the current key store, built-in common keys, and a text
  dictionary; the request uses `ClassicKeyscan::new` with the selected
  candidates.
- Key types: A, B, or both.
- Show progress, attempts, verified count, and a per-sector result table.
- Save discovered keys to the key store (merge, no duplicate keys) or export
  `.dic`; never silently replace existing entries.
- Cancellation uses `CancelToken`; the reader lease remains exclusive.

### Recovery

- Add a Recovery page over `Service::recover` and the existing
  `nfc_recover::auto` classifier.
- Strategy selector: Auto, Darkside, Nested, Static Nested, Hardnested,
  FM11RF08S; expose known block/type, target block/type, hardnested tables
  directory, and max nonces where applicable.
- Show acquisition/filtering/brute-force progress from `RecoveryProgress`,
  elapsed time, and a Cancel action.
- A recovered key is shown with its verification status; offer save to the key
  store.
- Surface the engine's honest limitations: darkside may report no repeated
  nonce sample on host-driven readers; at the time of this phase FM11RF08S was
  detection-only when the reader could not schedule acquisition; hardnested
  requires tables. Phase 25.3 later added backdoor recovery; the
  without-backdoor path remains unclaimed.
- Recovery is read/authenticate-only. No card write is added.

### Diagnostics

- Add a Diagnostics page with:
  - application and `nfctl`/service version;
  - supported driver prefixes and reader list;
  - hardnested table path/resolution;
  - build/profile facts that are already available.
- No card I/O. No telemetry, network, or secret values.

### Activity

- Promote the session log to a full Activity page: timestamp, operation,
  result, backup path, and warnings; in-memory only.
- Clear action. Do not log key material or NDEF personal payloads by default.
  Card UIDs and paths are acceptable in the session log.

### Settings

- Persist GUI preferences under `$XDG_CONFIG_HOME/nfc-foundry/ui.json` with
  atomic `0600` writes:
  - Classic key store path;
  - backup directory;
  - hardnested tables directory;
  - common-key candidate toggle;
  - default dump format.
- Validate paths and reload the affected stores after saving. Do not silently
  ignore a malformed preference file.
- Advanced mode stays session-only and is not persisted.
- The language preference is added in 19e.

## Order

1. Settings model and persistence (needed by keyscan/recovery paths).
2. Key scan flow and tests.
3. Recovery flow, progress/cancel, save result.
4. Diagnostics, Activity, Settings pages.
5. Mock/Xvfb tests and docs.

## Acceptance

- Keyscan finds a scripted key, reports attempts/verification, merges into the
  store, and can export `.dic`.
- Recovery classification and each strategy render progress; Cancel stops
  before the next reader I/O; a recovered key is verified and saveable.
- Darkside/FM11RF08S limitations are visible, not reported as generic
  failures.
- Diagnostics performs no card I/O.
- Activity contains no key material or NDEF payload by default.
- Settings survive restart and invalid paths are reported.
- `cargo fmt`, workspace clippy with `-D warnings`, workspace tests, gated GTK
  scenario, and `reuse lint` pass.

## Non-goals

- NDEF/Type 2 screens (19b/19c).
- UID clone, factory format, or any new card-write operation.
- Telemetry, website, accounts, or auto-update.
- `zh-CN` and packaging (19e).

## First files

- `crates/gui/src/app.rs` — pages, recovery/keyscan state, progress.
- `crates/gui/src/settings.rs` — preferences model/path (port the removed
  `UiPreferences` shape).
- `crates/nfc-app/src/recovery.rs` — service entry point/progress.
- `crates/nfc-app/src/classic/keyscan.rs` — result and candidate source.
- `crates/nfctl/src/main.rs` — CLI parity for keyscan/recover/diag.
