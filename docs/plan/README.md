# Phased implementation plan

Status legend: `done`, `in progress`, `pending`, `conditional`.

| Phase | Scope | Status |
|---|---|---|
| 00 | Repository transition: archive, rebrand, GPL/REUSE, libnfc-ng fork | done |
| 01 | `libnfc-ng` pure library, Linux build, AUR package | done |
| 02 | MyGo shell and service decoupling | done |
| 03 | `nfctl` unified CLI | done |
| 04 | Native Go Gen1A; remove `nfc-mfsetuid` | done |
| 05 | Type 2 / NTAG21x and NDEF | done |
| 05b | GUI Type 2 page and NDEF writes | done |
| 05c | Classic GUI writes and advanced bulk write | done |
| 05d | Advanced Type 2 protected GUI writes | done |
| 05e | NDEF record catalogue and decoded views | done |
| 05f | NDEF UX and format completion | done |
| 05g | NDEF on MIFARE Classic | done |
| 05h | Magic Type 2 UID/backdoor operations | pending |
| 05i | NTAG copy and lifecycle operations | done |
| 06 | Reader expansion and capability matrix | pending |
| 07 | ISO14443-4 APDU and DESFire read | in progress |
| 08 | FeliCa / Type 3 NDEF scope | conditional |
| 09 | Card emulation for mobile Linux (scope to be defined) | conditional |

## Feature policy

- Default to implementing every useful capability that is feasible with the
  current transport and hardware model.
- "Not hardware-validated" gates advertising and default enablement, not the
  implementation. Build the code, test it hardware-free, and gate destructive
  hardware tests explicitly.
- "Dangerous" is handled with an Advanced mode, per-action confirmation,
  backups, and read-back verification, not by silently omitting the feature.
- Advanced mode may expose generic bulk writes and otherwise hidden destructive
  operations. It is a user-authorized UI layer, not a protocol bypass.
- Genuine Type 2 UID pages remain physically read-only; magic-clone UID work is
  a separate validated phase.
- Non-Linux targets, telemetry, and static linking of libnfc-ng or GPL engines
  remain out of scope.
- Full DPP/Easy Connect protocol integration is out of scope for this product.
  NFC Foundry may write and read DPP URIs as NDEF data, but it does not run DPP
  authentication or provisioning.

## Rules

- One phase at a time unless work is genuinely independent.
- Every phase needs a spec in `docs/specs/` before implementation and a note in
  `docs/notes/` when it is complete.
- Do not advertise a reader or card family until its hardware validation is
  recorded.
- Do not narrow libnfc-ng's default driver set. Compile all upstream drivers;
  select a validated profile explicitly in the application package/CI.
- Card emulation is a conditional future phase (Phase 09), not a global
  non-goal. Do not write "No emulation" as a product-wide constraint.
- Do not add Windows/macOS, AppImage/Flatpak, Wails, telemetry, or a website.
- Keep `reuse lint` green.
- The CLI is English-only; the GUI is English-first with a complete Simplified
  Chinese translation.

## Definition of done for a phase

- Code and tests merged.
- `go test ./...` and the relevant hardware test pass.
- `reuse lint` passes.
- Docs, packaging, and support matrix updated.
- No temporary scaffolds or dead code left behind.
