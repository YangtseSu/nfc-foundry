# Phase 25 plan: owned-card support

Status: 25.1–25.4 complete 2026-11-12; 25.5 inventory and factory-key
authentication complete, formatting and user-facing writes not implemented.
Hardware:
PN532 UART first; ACR122U records after a reader is available. This phase
covers the five cards already owned for the project: Fudan FM11RN32N
(FM11RF32N, 4K Classic-compatible), Fudan FM11RF08S, MIFARE Ultralight EV1,
MIFARE DESFire EV3 8K, and a UID-changeable NTAG213 clone.

The goal is to make each card usable through the existing product model, not to
add a second protocol stack. The cards map onto Classic, Type 2, Classic
recovery, and Type 4 NDEF. The only new protocol surface is the minimal DESFire
native command set needed to inventory and NDEF-format an EV3 8K.

## Why this phase exists

The Rust rewrite already has most of the code, but several paths are unproven
or incorrectly modeled for these cards:

- Classic 4K geometry, dumps, diff, restore, and MAD2 NDEF exist, but the
  hardware matrix still records 4K as `not recorded`.
- FM11RF08S is detected and then refused: `fm11rf08s_attack` returns
  `Unsupported` after the static encrypted nonce check.
- Type 2 maps a 48-byte CC to the legacy Ultralight layout and a 128-byte CC to
  `Generic`; there is no `GET_VERSION`, no Ultralight EV1 product profile, and
  the configuration model is NTAG21x-specific.
- The DESFire EV3 8K has no NDEF Tag Application (`6A82`), and the product has
  no DESFire native identity, inventory, or formatting path.
- Type 2 write policy explicitly refuses manufacturer/UID pages, and no guarded
  UID workflow exists for a UID-changeable clone.

## Principles

- Service first. `nfc-app` owns every workflow; `nfctl` and GTK call the same
  methods with the same gates.
- Read-only inventory before any mutation. Record UID, ATQA, SAK, version,
  capacity, CC, and current state for every card first.
- Every mutation has a backup, a read-back, a card-identity check, and an
  explicit confirmation. The only exceptions are operations that cannot be
  backed up; those are listed in the irreversibility section and need a typed
  confirmation.
- No new external engine. Proxmark3 remains a differential test oracle only;
  formatting and undo must work without it.
- The transport, Crypto1, parity, and APDU layers are sufficient for Classic
  and Type 2. DESFire EV2 authentication and secure messaging need pure-Rust
  AES-128 and CMAC crates; those are the only new dependencies in this phase.
- Every slice updates its spec, note, CLI/GUI parity row, hardware matrix row,
  and README status.
- A card is advertised only after its hardware row is recorded on a supported
  reader.

## Irreversible operations

This table is part of the plan, not an implementation detail. It is the answer
to "what can this phase permanently change?".

| Operation | Card | Reversible? | Gate |
|---|---|---|---|
| Classic data-block writes and NDEF data | FM11RN32N | Yes, from a full dump plus key backup | Full dump, keys, read-back, identity check |
| Classic block 0 / trailer access-bit writes | none in this phase | Not applicable | Excluded; UID and trailer writes are not part of phase 25 |
| FM11RF08S nonce collection and key verification | FM11RF08S | Yes, no card writes | Known key required; read-only otherwise |
| Type 2 user-page and config writes | Ultralight EV1 | Config/PWD/PACK are EEPROM and recoverable with the current password; page 3 OTP CC, lock bits, `CFGLCK`, and `AUTHLIM` are not | Backup first; never set one-way locks, `CFGLCK`, or `AUTHLIM`; write PWD/PACK before AUTH0 |
| EV1 NDEF formatting | Ultralight EV1 | No: the CC is written into OTP page 3 | Only when the card is blank; separate typed confirmation; skip when a valid CC already exists |
| DESFire NDEF formatting | DESFire EV3 8K | No full-card binary backup. The operation is undoable by deleting the NDEF application with the PICC master key (`DeleteApplication`; PM3 `hf mfdes deleteapp`). That is not a byte-for-byte restore and the transaction counter still advances | Only on an unformatted owned card; keep and verify the PICC key before any change; Advanced mode; typed confirmation; no key changes; no brute force |
| NTAG213 UID change | UID-changeable clone | Intended yes: write the original UID back. Only if the clone accepts a second UID write | Back up pages 0-2 and the full image; no page 2 lock-bit writes; typed confirmation; abort after the first failed write |
| Genuine NTAG213 UID attempt | none | Write is refused by the card | Do not run unless explicitly requested; a refused write changes nothing |

The DESFire EV3 8K has no byte-for-byte restore, but the planned formatting is
undoable by deleting the NDEF application with the PICC master key. Keep that
key, and record the application ID before creating it. A broken half-created
application can still be deleted this way; a failed PICC authentication changes
nothing. `FormatPICC` is not the restore path and is not part of this phase.
Ultralight EV1 formatting is conditionally irreversible (OTP CC); it is skipped
when the card already carries a valid CC. The UID clone experiment is intended
to be reversible, but the first run is the test that proves it, so it is
treated as a risk-bearing operation.

## Slices and order

The order minimizes risk: prove existing surfaces first, then add one new
product profile, then the recovery solver, then the two destructive workflows.

### 25.1: FM11RN32N Classic 4K validation

Status: complete 2026-11-12 on PN532 UART with FM11RF32N UID `59 98 C9 C5`.
See [25a-classic-4k.md](../notes/25a-classic-4k.md).

Goal: prove the existing Classic 4K path on the owned card and fix any
assumptions that only held for 1K.

Deliver:

- Read-only inventory: select, UID/ATQA/SAK, 40-sector/256-block geometry,
  full dump, per-sector key scan.
- Temporary data-block write, read-back, restore, and a final full-dump diff.
- Classic NDEF on the 4K layout only when the card is blank and a full backup
  exists; MAD2 and GPB handling are verified.
- Static encrypted nonce probe. If the FM11RF32N shares the FM11RF08S
  behaviour, record it and route the card to 25.3 instead of pretending the
  standard attacks apply.
- No block 0, no trailer access-bit changes, no UID writes in this slice.

Acceptance:

- 256/256 blocks and 40/40 sectors read with verified keys.
- One data block changes, reads back, restores, and diffs zero.
- NDEF format/write/read/restore succeeds on a blank card, or the slice records
  that the card is not NDEF-formatted and leaves it unchanged.
- Hardware matrix gains a dated PN532 row with the exact UID.

### 25.2: MIFARE Ultralight EV1

Status: MF0UL11 complete 2026-11-12 on PN532 UART; MF0UL21 unit-tested only.
See [25b-ultralight-ev1.md](../notes/25b-ultralight-ev1.md).

Goal: model MF0UL11 and MF0UL21 explicitly instead of guessing from the CC
size.

Deliver:

- `Reader::type2_version` (`GET_VERSION`, `0x60`) implemented by PN532 UART,
  ACR122U, and the mock reader.
- Product identification from the version response and CC: MF0UL11, MF0UL21,
  legacy Ultralight, NTAG21x, or generic Type 2.
- Explicit layouts:
  - MF0UL11: 20 pages, user 4..15, configuration 16..19.
  - MF0UL21: 41 pages, user 4..35, extended lock byte at page 36, configuration
    37..40.
- EV1 configuration semantics: preserve `MOD`/`STRG_MOD_EN` and `VCTID`;
  read/write `AUTH0`, `ACCESS`, `PWD`, and `PACK`; never write page 36 byte 3
  (`0xBD`).
- Type 2 read/dump/restore, NDEF write/read, and password set/clear for both
  variants.
- CC handling is OTP-aware: formatting is offered only when page 3 has no
  valid CC, and the confirmation states that the CC bits are one-way.

Acceptance:

- Both variants are identified without relying on the CC size alone.
- Full page image, including configuration and extended lock bytes, round
  trips for data and configuration, except one-way bits.
- NDEF write/read passes on an already-formatted EV1; a blank EV1 is formatted
  only with the separate OTP confirmation.
- PWD/PACK set, wrong PACK refused, authenticated access, clear, and original
  configuration restored.

### 25.3: FM11RF08S key recovery

Status: complete 2026-11-12 on PN532 UART; ACR122U not recorded.
See [25c-fm11rf08s.md](../notes/25c-fm11rf08s.md).

Goal: replace the detection-only strategy with a native recovery path.

Evidence: the transport has parity frames, Crypto1 state, and
`lfsr_recovery32`. The without-backdoor distance prediction was tested on
hardware and is not reliable under host-driven timing. The published backdoor
(`0x64`/`0x65` and the PM3 backdoor keys) decrypts the nested nonce directly,
so acquisition needs no target key and no reader scheduling.

Deliver:

- Implement custom-command authentication and nested exchange for the backdoor.
- Collect, for every target sector and key type, the clear nested nonce `nt`,
  the regular encrypted nonce `nt_enc`, and the parity-error mask.
- Port `staticnested_1nt` from PM3 `tools/mfc/card_only/` into `nfc-recover`,
  reusing `lfsr_recovery32`; keep the GPL notice. Add the optional 2x1nt
  cross-filter only if the first candidate volume is impractical.
- Try reused/supplied keys before the solver, verify candidates online, merge
  verified keys into the key store, and expose progress and cancellation.
- Route `RecoveryStrategy::Fm11rf08s` and auto classification to the new path
  when the static encrypted profile is confirmed.
- Keep an explicit error for cards whose backdoor does not answer.

Acceptance:

- The backdoor clear nonce matches a known-key decryption, and the solver
  candidate set contains that key.
- Recover 32/32 sector keys on the owned card and read-verify 16/16 sectors.
- Cancellation stops collection/verification without leaving recovery mode on.
- The solver produces the same candidate count as the PM3 tool for the same
  `uid/nt/nt_enc/parity` vector.
- No external process is used.

### 25.4: UID-changeable NTAG213

Status: complete 2026-11-12 on PN532 UART for a 7-byte clone; ACR122U not
recorded. See [25d-type2-uid.md](../notes/25d-type2-uid.md).

Goal: add a guarded UID workflow for the owned magic clone without weakening
the read-only rule for genuine Type 2 tags.

Deliver:

- Hardware experiment on the owned clone: the 7-byte NTAG213 clone accepted
  same-value writes to pages 0, 1, and 2 with the UID unchanged.
- A guarded workflow that:
  - reads and backs up the full image and pages 0-2;
  - validates the target UID and recomputes BCC0/BCC1;
  - writes page 1, page 2 (BCC1 plus the preserved internal byte and lock
    bytes), then page 0;
  - reads back, reselects, verifies ATQA/SAK/UID, and restores the original UID
    on any failure;
  - refuses cards whose UID pages are not writable or whose layout is not the
    standard 7-byte NTAG213 layout.
- Advanced mode plus typed confirmation, a dry-run plan with exact old/new
  pages, and no change to the existing page-write policy.
- CLI and GUI parity: `type2 uid` and a Type 2 Advanced UID dialog.

Acceptance:

- On the owned clone, old UID -> new UID -> reselect -> restore old UID, with
  45/45 pages and the original page JSON after the final restore.
- A genuine NTAG213 refuses the write and shows no mutation.
- Lock bits are unchanged after the run.
- ACR122U remains unrecorded.

### 25.5: DESFire EV3 8K NDEF enablement

Status: inventory, factory-key authentication, and a reserved temporary
application/file round trip complete 2026-11-12; NDEF formatting and
user-facing writes are not implemented. See
[25e-desfire.md](../notes/25e-desfire.md).

Goal: make the owned unformatted EV3 8K usable with the existing Type 4 NDEF
workflow, and make its real state visible before any change.

Deliver:

- A pure `nfc-desfire` crate over `Reader::transceive_apdu`. The same session
  layer serves inventory, formatting, and undo; no PM3 process is involved:
  - `GetVersion`, `GetApplicationIDs`, `SelectApplication`, `GetFileIDs`,
    `GetFileSettings`;
  - `Authenticate` (`0x0A`) with legacy DES/2K3DES and D40 send mode for the
    factory-blank card; `AuthenticateEV2First` (`0x71`) for AES-keyed cards;
  - `CreateApplication`, `CreateStdDataFile`, `WriteData`, `CommitTransaction`.
- Read-only inventory: card version, application IDs, and file IDs/settings.
  Works on the current unformatted card without authentication.
- AN11004 formatting on an unformatted card: create the NDEF Tag Application
  (`D2760000850101`), create the CC file `E103`, create the NDEF file `E104`,
  write the 15-byte CC, commit, then reselect and read the CC through the
  existing Type 4 path.
- Formatting is Advanced, typed-confirmation, and refused when a valid NDEF
  application already exists. The dialog names the application/files, the
  key source, and the fact that no full backup exists.
- `DeleteApplication` as the undo path: record the created AID, require the
  PICC master key, and delete only the NDEF application created by this
  product. This is the same operation as PM3 `hf mfdes deleteapp`, implemented
  natively over the authenticated session. It is not a general application
  manager.
- No DESFire key change, no user-facing file CRUD beyond the NDEF bootstrap,
  no brute force or default-key dictionary. The hardware test uses primitive
  create/write/read/delete commands on the reserved test AID only.
- Type 4 NDEF read/write/restore after formatting uses the already validated
  NTAG 424 path.
- The exposed surfaces are the read-only `nfctl desfire info`,
  `nfctl desfire read`, and the GTK `DESFire` workspace with `Inventory` and
  `Read` tabs. There are no DESFire write controls.
- This slice only bootstraps NDEF. A general DESFire application/file/key
  workbench is separate scope, because NDEF is only one application on the
  card.

Acceptance:

- Inventory on the owned card reports the DESFire EV3 version and the absence
  of the NDEF application.
- With the factory DES key, formatting creates a readable Type 4 CC and a
  writable NDEF file.
- A Text record writes, reads back, and restores the original empty state.
- A wrong key is refused without retry loops and without creating anything.
- The undo path deletes the created NDEF application and leaves
  `GetApplicationIDs` empty, matching the pre-format inventory. PM3
  `hf mfdes deleteapp` reproduces the same result with the same PICC key.

Follow-on read work (file-settings decoding, Value/Record reads, EV1/EV2
secure messaging, and DESFire Type 4 NDEF) is tracked in
[phase 26](26-desfire-read.md). This slice remains the NDEF bootstrap only.

## Cross-cutting work

- `nfctl` commands and GUI paths for every new operation; no CLI-only card
  operation. CLI output is English-only; GTK strings go into the `zh-CN`
  catalog.
- Hardware gates follow `docs/testing.md`: environment-gated tests, explicit
  write tokens, no hardware in `cargo test --workspace`.
- Test fixtures: scripted 4K card, MF0UL11/MF0UL21, a UID-writable Type 2
  clone, FM11RF08S nonce vectors, and a DESFire EV3 with and without the NDEF
  application.
- Docs: one spec and one note per slice, hardware matrix rows, README status,
  and the recovery/TLV docs when those formats change.
- Verification before a slice is complete: `cargo fmt --check`, clippy with
  `-D warnings`, workspace tests, the slice's hardware test, and `reuse lint`.

## Open questions for hardware

- Does the FM11RF32N return the static encrypted nonce profile, and does it
  need the 4K sector map in the solver?
- Which Ultralight EV1 variant is owned, and does it ship with a valid CC?
- Which UID clone variant is owned: direct-write pages 0/1, 7-byte or 4-byte
  UID, and does it recompute BCC1?
- The EV3 PICC key is legacy DES/2K3DES all zero on the owned factory-blank
  card; `AuthenticateEV2First` with AES-128 is not available in that state.

These questions are answered by the read-only inventory before any irreversible
step.
