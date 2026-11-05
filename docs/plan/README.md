# Phased implementation plan

Status legend: `done`, `in progress`, `pending`, `conditional`, `backlog`.

| Phase | Scope | Status |
|---|---|---|
| 00 | Repository transition: archive, rebrand, GPL/REUSE, libnfc-ng fork | done |
| 01 | `libnfc-ng` pure library, Linux build, AUR package | pending |
| 02 | MyGo shell and service decoupling | pending |
| 03 | `nfctl` unified CLI | pending |
| 04 | Native Go Gen1A; remove `nfc-mfsetuid` | pending |
| 05 | Type 2 / NTAG21x and NDEF | pending |
| 06 | Reader expansion and capability matrix | pending |
| 07 | ISO14443-4 APDU and DESFire read | pending |
| 08 | FeliCa / Type 3 NDEF scope | conditional |
| — | Card emulation research for mobile Linux | backlog |

## Rules

- One phase at a time unless work is genuinely independent.
- Every phase needs a spec in `docs/specs/` before implementation and a note in
  `docs/notes/` when it is complete.
- Do not advertise a reader or card family until its hardware validation is
  recorded.
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
