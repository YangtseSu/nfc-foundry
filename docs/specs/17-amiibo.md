# Spec: controlled Amiibo / NTAG215 workflow

Status: implemented 2026-11-15. The decryption and re-signing path was
validated against a user-supplied Amiibo Pack and its key files. A full write
was validated on PN532 UART with a discardable Amiibo-writable NTAG215; the
ACR122U record is pending. No Nintendo key material or Amiibo data is shipped
or committed. Plan: [README](../plan/README.md#17-amiibo--ntag215-read-decrypt-encrypt-write-user-keysdata).

## Problem

An Amiibo is an NTAG215 tag whose user memory is encrypted and signed with
per-tag keys derived from Nintendo master keys and the tag UID. The raw Type 2
page editor can read and write the bytes, but it cannot tell whether the data
is a valid Amiibo, cannot decrypt it, and cannot prepare a dump for a different
tag UID. Reading the raw pages without the user's key files and treating them
as plaintext produces unusable output.

The product must expose read, decrypt, encrypt, and write without shipping
Nintendo keys or Amiibo data, and without turning the Type 2 editor into an
unrestricted cloning tool.

## Scope

Supported:

- NTAG215 tags with the canonical 540-byte Type 2 layout and capability
  container `F1 10 FF EE` (504 bytes user memory).
- Loading the user's 160-byte combined retail key file
  (`key_retail.bin` = 80-byte data key + 80-byte tag key) or the two 80-byte
  files (`unfixed-info.bin`, `locked-secret.bin`).
- Decrypting a raw 532/540/572-byte dump and verifying both HMAC-SHA256
  signatures.
- Encrypting and signing a 520/540-byte plaintext dump.
- Reading a live tag into a 540-byte raw dump and decrypting it in memory.
- Writing a source dump to an owned, writable magic or blank NTAG215. The
  source is re-signed for the selected tag's current UID; UID pages 0 and 1
  are never written.
- `nfctl amiibo ...` and the GTK Type 2 Amiibo tab.

Not supported:

- Shipping, downloading, deriving, or embedding Nintendo master keys, Amiibo
  identity databases, or Amiibo data.
- Changing a physical Amiibo tag, a locked tag, or a tag whose configuration is
  permanently locked.
- Type 4, FeliCa, or Skylanders figures.
- Editing Amiibo settings or AppData in this phase. The workflow decrypts and
  re-encrypts user-supplied bytes; it does not interpret game save layouts.
- Importing or emulating Amiibo data on a phone or Proxmark3.
- Reporting a written tag as valid when the signatures, locks, or identity
  changed.

## Format

The tag dump is 540 bytes (135 NTAG215 pages). Crypto covers the first 520
bytes; the final 20 bytes are the dynamic lock page and the NTAG
configuration pages. A raw dump longer than 540 bytes is truncated to its first
540 bytes; a 532-byte dump is padded with eight zero bytes for the write-only
PWD/PACK pages. A plaintext dump is 520 bytes (internal plaintext only) or 540
bytes (plaintext followed by the raw 20-byte tail).

The internal plaintext layout is the amiitool layout:

- `0x000..0x008`: raw pages 2 and 3 (BCC1/lock and capability container).
- `0x008..0x028`, `0x1B4..0x1D4`: data and tag HMACs.
- `0x028..0x04C`, `0x04C..0x1B4`, `0x1DC..0x208`: capability container,
  identity block, settings, and application data.
- `0x1D4..0x1DC`: raw UID pages 0 and 1.
- `0x1DC..0x1E4`: Amiibo identification block (game/character ID, variant,
  figure type, model number, series, format version).

The final 20 plaintext bytes are kept byte-for-byte so decrypt followed by
encrypt reproduces the input dump.

## Crypto

Both master key records are 80 bytes: a 16-byte HMAC key, a 14-byte type
string, RFU, magic-byte count, 16 magic bytes, and a 32-byte XOR pad. The data
record uses `unfixed infos`; the tag record uses `locked secret`.

For each operation:

1. Build the 64-byte seed from the plaintext write counter, UID pages, and the
   data at `0x1E8..0x208`.
2. Prepare the type string, seed prefix, magic bytes, seed middle, and
   XOR-padded seed tail.
3. Run HMAC-SHA256 with the master HMAC key over `u16be(counter) || prepared`
   for counters 0 and 1; the first 48 bytes are AES key, AES IV, and HMAC key.
4. Encrypt or decrypt `0x188` bytes at internal offset `0x02C` with AES-128
   counter mode using a big-endian 128-bit counter initialized from the IV.
5. Verify or regenerate the tag HMAC over plaintext `0x1D4..0x208` and the
   data HMAC over plaintext `0x029..0x208`, in that order.

A decryption that does not verify both signatures is refused. The workflow
never reports `signature valid` from key derivation alone.

## UID binding

The UID is read from pages 0 and 1: three UID bytes, BCC0, four UID bytes, then
BCC1 in page 2. A source dump may belong to another tag. For a write, the data
is decrypted, its internal UID pages and page-2 BCC1 are replaced with the
selected tag's UID and BCC values, and the data is encrypted and signed again.
Pages 0 and 1 are not written, so a magic tag does not need a UID write.

## Read and file operations

- `amiibo read` reads a complete live NTAG215 image and, when key files are
  supplied, decrypts it in memory. A partial image is refused.
- `amiibo decrypt` takes a raw dump, verifies both signatures, and writes the
  plaintext file. Without an output path it only reports the identity.
- `amiibo encrypt` takes a plaintext dump, validates the UID/BCC and
  identification block, encrypts and signs it, and writes the raw dump.
- File operations never need a reader.

## Write gate

`nfctl amiibo write` and the GTK Write Amiibo action require:

- Advanced mode, an explicit confirmation, and a live selected card.
- A complete NTAG215 image whose capability container can still become the
  Amiibo container, or the standard `E1 10 3E 00` container on a tag whose
  page-3 write proves it accepts the Amiibo bytes.
- A source dump that decrypts and verifies, or a plaintext dump whose UID and
  identification block are structurally valid.
- A target configuration that is not permanently locked and a capability
  container that is blank, already the Amiibo container, a bitwise superset,
  or the standard NTAG215 `E1 10 3E 00` container. The latter two are only
  candidates: the page-3 write and read-back prove whether the tag accepts the
  Amiibo bytes before user data is written.
- Every target page to be written is unlocked under the current locks. This
  rejects a physical Amiibo and any already-locked tag before a backup.
- The current PWD/PACK when the target configuration protects the write range.
- A card identity and full 540-byte target image fingerprint captured by the
  preflight.

The preflight returns a plan with the target UID, source UID, numeric identity,
the 133 pages to write, the derived PWD/PACK policy, warnings, the planned
backup path, a one-shot token, and a 120-second expiry. The plan never prints
the password or PACK.

## Write sequence

The execution consumes the token, rechecks the card identity and the full
target image, writes the backup, then writes in this order:

1. Capability container page 3. A genuine NTAG215 rejects this exact change and
   the operation stops before any user page is touched.
2. User pages `4..129` (encrypted source data).
3. PWD page 133 and PACK page 134.
4. Static lock page 2 with the target BCC1 and the source lock bytes.
5. Dynamic lock page 130.
6. Authenticate with the new PWD and require PACK `80 80`.
7. Configuration pages 131 and 132 (AUTH0 `04`, ACCESS `5F`).

Every readable page is read back immediately. PWD/PACK read as zero, so they
are verified by PWD_AUTH instead. The final verification reselects the card,
requires the same UID/ATQA/SAK, reads all pages, compares pages `0..133` with
the planned image, authenticates with the new credentials, decrypts the written
image, and requires the same identity and plaintext as the source.

The new PWD is the public Amiibo algorithm B value for the target UID and the
PACK is `80 80`. The configuration write is irreversible: CFGLCK becomes set.
The plan and confirmation state this. A failure stops at the first page and
names the backup; no automatic restore is attempted because lock and
configuration bits cannot be cleared.

## CLI

```
nfctl amiibo read [--output PATH]
nfctl amiibo decrypt --input PATH [--output PATH]
nfctl amiibo encrypt --input PATH [--output PATH]
nfctl amiibo write --input PATH [--plain] [--advanced] [--dry-run]
```

`--amiibo-keys` accepts the 160-byte combined file; `--amiibo-data-key` and
`--amiibo-tag-key` accept the two 80-byte files. Write requires `--advanced`
for dry-run and execution, and confirmation (`--yes` for non-interactive use).
`--plain` selects a plaintext source instead of a raw dump.

## GUI

The Type 2 workspace adds an Amiibo tab:

- Key file chooser and source dump chooser.
- Read Amiibo: reads the live tag and shows UID, product, signature state, and
  numeric identity. Save raw dump writes the 540-byte image.
- Decrypt dump and Encrypt dump file actions.
- Write Amiibo: requires Advanced mode, a live card, keys, and a source dump.
  The plan dialog shows the target UID, source UID, identity, page count,
  derived-credential policy, irreversible lock/configuration warning, backup
  path, and verification plan. Confirm starts the same service methods as
  `nfctl`.

## Safety

- The user supplies the master keys and the dump; none are stored by the
  application. Key bytes are redacted from `Debug`, logs, errors, and JSON.
- File paths and dumps are the user's; the application only writes the
  requested output and its own backup.
- A partial live read, non-NTAG215 layout, locked target, invalid source, or
  changed card is refused before any write.
- The backup is written before the first page write.
- A write never reports success from the command acknowledgement; the final
  full read, PWD_AUTH, and decrypted identity comparison are the success
  criteria.
- No UID page write is part of this workflow.

## Tests

- `nfc-amiibo`: synthetic master keys and a 540-byte tag produced by an
  independent reference implementation; decrypt, re-encrypt round trip,
  tampering, UID rebinding, BCC errors, identity parsing, key-file parsing, and
  key redaction.
- `nfc-app`: read and decrypt a scripted NTAG215, file decrypt/encrypt,
  write-plan gates (Advanced, locked, wrong product, wrong key, partial read,
  protected target), execution order, backup, PWD/PACK verification, read-back
  failure, card change, and single-use token.
- `nfctl`: mock read/decrypt/encrypt/write, Advanced and confirmation gates,
  dry run, JSON contract, and secret redaction.
- GTK: Amiibo tab visibility, key/source selection, signature failure, plan
  dialog, confirmation, locked-tag refusal, and verified write state.
- Local validation against a user-supplied Amiibo Pack: 278 dumps verified and
  re-encrypted byte-for-byte; no key or dump values are committed.
- Hardware: live read/decrypt and an owned magic NTAG215 write. PN532 UART
  passed on 2026-11-15; the ACR122U record is pending.

## Required checks

`cargo fmt --all --check`, workspace Clippy with `-D warnings`, workspace
tests, gated GTK tests, and `reuse lint`.
