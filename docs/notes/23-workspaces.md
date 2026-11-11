# Note: card-family workspaces

## Problem

The grouped tab row still mixed card-family pages with shared features:

- `Card` was Classic-only, while `Type 2` and `NDEF` were family-adaptive.
- `Dumps` held Classic Diff/Restore, the header menu held Save/Load, and the
  Type 2 Memory tab held another Save/Load/Restore. There was no single answer
  to "where are dumps for this card?".
- `Keys`, `Recovery`, and Classic `Dumps` were visible while a Type 2 card was
  selected, and Type 1 pages were visible for a Classic card.

## Implemented

### Workspace selector

The header now carries a compact `MIFARE Classic` / `NFC Forum Type 2` `MenuButton`. It switches
the top-level workspace stack, updates its label, and moves the shared NDEF
view into the selected workspace. `Read card` selects the workspace from the
card family automatically; the manual selector exists for loading dumps and
inspecting without a live card. `←` returns to the last workspace from the
hidden System page.

### Classic workspace

```text
Card │ NDEF │ Keys │ Recovery │ Dumps
```

- `Card` is the former `Card` block table.
- `Keys` owns `Key store` and `Key scan`.
- `Dumps` owns `Save dump…`, `Load dump…`, and the `Diff` / `Restore` sub-tabs.
- Save/Load were removed from the `☰` menu.

### Type 2 workspace

```text
Pages │ Protection │ NDEF │ Dumps
```

- `Pages` is the former Memory page without the dump toolbar or restore card.
- `Protection` keeps configuration, locks, and password.
- `NDEF` hosts the shared NDEF view moved from the Classic workspace.
- `Dumps` owns `Save dump…`, `Load dump…`, and `Choose project image…` /
  `Restore image…`.

### NDEF gating

The shared NDEF view renders only the active workspace's card facts:

- Classic renders Classic MAD/NDEF from the loaded dump.
- Type 2 renders CC/TLV/NDEF from the Type 2 session.
- Write/format dispatch checks the workspace before choosing the Classic or
  Type 2 service path.

## Verification

- `cargo fmt --all --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test --workspace`: passes.
- `reuse lint`: clean.
- Gated GTK scenario:
  - switches Classic and Type 2 workspaces;
  - checks every Classic and Type 2 tab;
  - opens Diagnostics from the main menu and returns with `←`;
  - switches back with the header workspace menu and verifies the NDEF view is
    parented in the Classic slot and absent from the Type 2 slot;
  - keeps the Type 2 Protection focus/selection and recovery layout checks.
