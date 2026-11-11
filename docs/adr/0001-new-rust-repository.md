# ADR 0001: New Rust repository

- Status: accepted
- Date: 2026-11-09
- Note: the GUI stack later moved from GPUI Kit to GTK4-rs; see
  [ADR 0006](0006-gtk4-rs.md). The repository decision is unchanged.

## Context

The existing product is Go + MyGo + C `libnfc-ng` + C recovery tools. The new
direction is an all-Rust application, library, drivers, recovery engine, and
GPUI Kit GUI. Keeping both stacks in one repository would require two
toolchains, two CI systems, two packaging models, and conflicting agent rules.

## Decision

Create a new repository, `nfc-foundry-rs`, with a Rust workspace. Keep the
existing repository tagged and available as a behavioral oracle and
hardware-regression reference. Do not share build outputs or runtime code.

## Consequences

- The old implementation remains buildable until the Rust rewrite reaches
  feature parity.
- Documentation, test data, and safety rules are copied and adapted; Go/C
  implementation code is not a dependency.
- The new repository owns its AGENTS.md, plan, specs, notes, and packaging.
