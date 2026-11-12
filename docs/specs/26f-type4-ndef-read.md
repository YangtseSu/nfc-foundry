# Spec: phase 26.6 DESFire Type 4 NDEF read

Status: complete 2026-11-13. Plan:
[26-desfire-workbench.md](../plan/26-desfire-workbench.md). Note:
[26f-type4-ndef-read.md](../notes/26f-type4-ndef-read.md).

## Scope

Read the NFC Forum Type 4 NDEF Tag Application through the existing
`nfc-type4` and `nfc-ndef` layers and expose the result in the CLI and the
GTK DESFire workspace. The slice is read-only: formatting, provisioning,
writing, and deleting stay on the phase 25.5 paths.

## Contract

- The Type 4 adapter selects the NDEF Tag Application by DF name, reads the
  capability container, reads the NDEF file, and decodes the records.
- A DESFire card with an NDEF application is reported as a Type 4 NDEF tag,
  never as an unformatted tag.
- A card without the NDEF Tag Application is an error that names the failed
  selection; it is not reported as an empty or unformatted NDEF tag.
- Malformed capability containers, TLV lengths, NLEN, record lengths, and
  record syntax are errors. Proprietary and unknown CC TLVs are listed, not
  discarded.
- The NDEF file read reports file ID, capacity, read/write access, read-only
  reason, message length, and decoded records.
- No key is needed for the read. Authentication and secure messaging are not
  part of the Type 4 read path.
- The CLI writes the raw NDEF message only when `--output` is given.

## Surfaces

- CLI: `nfctl desfire ndef-read [--output PATH]`.
- GUI: DESFire → NDEF → Read NDEF. The result shows the CC, mapping version,
  MLe/MLc, NDEF file facts, records, and extra CC TLVs. Read is available
  without Advanced mode; format and delete remain Advanced and confirmed.

## Verification

- CLI mock Type 4 tag with a Text record, a proprietary File Control TLV, and
  an unknown TLV.
- GUI formatter test over the same fixture and an NDEF read widget check in
  the DESFire workspace scenario.
- ACR122U hardware: the NDEF provision/write/restore/delete test passed, then
  `nfctl desfire format-ndef`, `desfire ndef-read`, and `desfire delete-ndef`
  ran on the same card; the application list returned to empty.
