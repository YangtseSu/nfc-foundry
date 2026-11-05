# Archived NFCX material

This directory preserves the previous NFCX documentation so that implementation
history, hardware notes, and protocol details remain readable after the
project moved to NFC Foundry.

The code in this repository has also moved on. Treat everything here as
reference material, not as the current specification:

- `AGENTS.md`, `README.md`, `README.zh-CN.md`: the previous project and product
  description.
- `specs/`: previous step-by-step implementation specifications.
- `notes/`: previous implementation records and hardware findings.
- `toolchain/`: previous libnfc, mfoc, mfcuk, and hardnested build notes.
- `evidence/`: previous acceptance evidence.
- `deployment.md`, `release.md`, `docs-index.md`: previous distribution and
  documentation indexes.
- `github-workflows/`: previous GitHub Actions workflows.
- `website/`, `telemetry-worker/`: previous website and telemetry code.
- `images/`: previous screenshots and diagrams.

Current documents live in `docs/` and the repository root. The most useful
historical material to consult while implementing the new plan is:

- `notes/02-libnfc-toolchain.md`, `notes/03-libnfc-binding.md`,
  `notes/04-device-and-card-discovery.md`: PN532 UART bring-up and the
  `LIBNFC_DEVICE`/conffiles issue.
- `notes/05-mifare-classic-io.md`, `notes/06-dump-and-restore.md`: Classic
  memory geometry and write-safety checks.
- `notes/11-special-uid-cards.md`, `specs/11-special-uid-cards.md`: the Gen1A
  fallback sequence that this project will replace with native Go code.
- `notes/08-external-engine-framework.md`, `notes/09-mfoc-integration.md`,
  `notes/10-mfcuk-pipeline.md`, `notes/10b-hardnested-integration.md`: external
  engine ownership and result validation.
