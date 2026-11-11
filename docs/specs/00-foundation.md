# Phase 00 spec: Rust foundation

Date: 2026-11-09

Historical note: this phase bootstrapped the GPUI Kit shell. The GUI stack is
now GTK4-rs (`gtk4` + `libadwaita`, [ADR 0006](../adr/0006-gtk4-rs.md)); the
GPUI implementation was removed in phase 19. This spec remains a record of the
original phase.

## Goal

Create a compiling Rust workspace with clear crate boundaries, documentation,
GPUI Kit shell, CLI skeleton, licensing, and the first architectural decisions.
No reader, recovery, or NDEF behavior is implemented in this phase.

## Deliverables

- Cargo workspace with the crates listed in `docs/architecture.md`.
- `rust-toolchain.toml`, workspace lints, release profile.
- `nfc-core` card/error/ISO 14443A primitives.
- `nfc-transport` reader/capability traits.
- `nfc-drivers` PN532 UART and ACR122U USB stubs returning explicit
  `NotImplemented`.
- `nfc-classic`, `nfc-recover`, `nfc-ndef`, `nfc-type2`, `nfc-classic-ndef`,
  `nfc-type4` domain type skeletons.
- `nfc-app` device manager/service skeleton.
- `nfctl` CLI skeleton with the planned command surface.
- GPUI Kit window with the two-domain shell.
- README, AGENTS, architecture, hardware, recovery, NDEF, testing, packaging,
  ADRs, plan.
- GPL-3.0-or-later license and REUSE metadata.

## Non-goals

- No hardware access.
- No card writes.
- No recovery algorithm.
- No NDEF codec.
- No packaging beyond the skeleton.

## Acceptance

- `cargo fmt --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo check -p nfc-foundry`
- `reuse lint`
- No `todo!` or `unimplemented!` placeholders.
- Every unimplemented operation returns `NfcError::NotImplemented`.
