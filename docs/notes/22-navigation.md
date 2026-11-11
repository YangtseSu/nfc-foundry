# Note: header menu and grouped primary navigation

## Problem

The primary tab row had eleven peers: card workspaces, Classic tools, dump
file operations, and application settings all at the same level. The header
held only `Read card` and a text `Actions` menu, while the tab row scrolled
horizontally with localized labels.

## Implemented

### Primary tabs

The main switcher now has six pages:

- `Card`
- `Type 2`
- `NDEF`
- `Keys` (sub-tabs `Key store`, `Key scan`)
- `Recovery`
- `Dumps` (sub-tabs `Diff`, `Restore`)

`Keys`, `Dumps`, and the hidden `System` page use a nested `GtkStack` with its
own `GtkStackSwitcher`. The group switchers clear selectable-label selections
and take focus when their tab changes, like the Type 2 tabs.

### Header

- `Read card` remains the primary action.
- The text `Actions` button is replaced by a `☰` main menu with sections:
  Reader (Refresh readers), Dump (Save/Load dump), System (Diagnostics,
  Activity, Settings), and About NFC Foundry.
- `About NFC Foundry` opens `AdwAboutDialog` with the application name, version,
  GPL-3.0-or-later license, and repository link.
- A `←` back button is packed at the start of the header and is visible only
  while the hidden `System` page is shown. It returns to the last primary page.
- The main stack tracks the last primary page and updates the back button on
  every visible-child change.

### System pages

`Diagnostics`, `Activity`, and `Settings` are added to the main stack with
`add_named`, not `add_titled`, so they do not appear in the primary switcher.
The menu switches to `system` and selects the requested sub-tab. Diagnostics
re-renders when its system sub-tab becomes visible.

## Verification

- `cargo fmt --all --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `cargo test --workspace`: passes.
- `reuse lint`: clean.
- Gated GTK scenario (`gtk_workbench_scenarios`):
  - iterates only the six primary pages;
  - clicks the main menu's Diagnostics item, checks the system sub-tab, back
    button visibility, no label focus, and no label selection;
  - clicks `←` and returns to `Card`;
  - keeps the Type 2 Protection focus and selection checks.

Superseded by [23-workspaces.md](23-workspaces.md): the primary tabs are now
card-family workspaces with the workspace selector in the header.
