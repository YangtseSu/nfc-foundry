# Note: phase 26.9 DESFire application and file management

Date: 2026-11-13. Spec:
[26i-management.md](../specs/26i-management.md). Plan:
[26-desfire-workbench.md](../plan/26-desfire-workbench.md).

## Implemented

- `nfc-desfire` adds `DeleteFile` (`0xDF`), `ChangeFileSettings` (`0x5F`),
  `CreateTransactionMACFile` (`0xCE`), typed `ApplicationSpec`, and
  `TransactionMacFileSpec`. Access rights gain nibble encoding, named parsing
  (`key0`..`key13`, `free`, `denied`), and raw decoding.
- `CreateTransactionMACFile` sends a plaintext header and enciphers
  `TMACKey || TMACKeyVersion`; a legacy DES/2K3DES session is refused because
  it cannot protect the key. `ChangeFileSettings` sends a plaintext file number
  and protects `FileOption || AccessRights` according to the file's current
  Change right: free access is plain without authentication, a key right is
  enciphered with that key, and denied is refused.
- `nfc-app` adds `desfire_create_application`, `desfire_delete_application`,
  `desfire_create_file`, `desfire_delete_file`,
  `desfire_change_file_settings`, and `desfire_free_memory`. Every create
  checks the existing ID, reads free memory, refuses content above it, and
  reads the new settings back. A Value file's initial value is checked with
  `GetValue`. Every delete requires Advanced mode and confirmation, writes a
  JSON manifest atomically before the command, and verifies the ID is gone.
- `nfctl desfire` adds `memory`, `app-create`, `app-delete`, `file-create`,
  `file-delete`, and `file-settings`. The GTK DESFire workspace adds a
  `Manage` tab with the same fields and typed `DELETE` confirmation for both
  deletion paths; the Advanced switch moved to that tab.
- The scripted card answers application/file create and delete, file settings
  changes, free memory, GetVersion, GetValue, and EV1 enciphered management
  commands, including the CRC32 and response MAC.

## Verification

Passed:

```text
cargo test -p nfc-desfire --lib
cargo test -p nfc-app --test desfire_management
cargo test -p nfctl --test cli
cargo test -p nfc-foundry --bin nfc-foundry
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

New unit tests cover access-word encoding/parsing, management payloads, and an
AES-session round trip over the scripted card. Service tests cover application
creation, duplicate and zero-memory refusal, all six file types, capacity
refusal, settings read-back, manifests for file and application deletion, and
gate ordering. CLI tests cover creation, free-memory output, file creation and
capacity refusal, free-change settings, manifests, and Advanced gating. The
GTK test covers the Manage tab, sensitivity, access-word parsing, and file
spec parsing.

## Problems and resolutions

- A Value file created with 7 returned 0 in `GetFileSettings` while
  `GetValue` returned 7. The third settings field is the LimitedCredit amount,
  not the current value; the field was renamed `limited_credit_value`, the
  parser no longer bounds it by the value limits, and the create read-back
  checks the initial value with `GetValue`.
- `CreateTransactionMACFile` returned `0x917E` (length error) with a plaintext
  22-byte payload. The card updates the Transaction MAC key, so it requires the
  key and version to be enciphered. The command now sends a plaintext header
  and an enciphered data part, matching the EV3 sample implementation.
- `ChangeFileSettings` returned `0x917E` with a plaintext 4-byte payload. A
  key-based Change right requires an enciphered command; a free Change right
  requires plain communication without authentication. The service selects the
  protection from the current settings and refuses a mismatched key number.
- The first hardware run failed at the Value file because the scripted card
  had mirrored the old assumption. The mock now reports the LimitedCredit
  amount as zero and implements `GetValue`, and its management commands handle
  EV1 enciphered payloads.
- The first hardware run had a Type 2 tag on the PN532 and the DESFire card on
  the ACR122U. The PN532 run was repeated after the card was moved and passed;
  the reader had also enumerated and read the Type 2 tag normally.

## Hardware

PN532 UART `pn532_uart:/dev/ttyUSB0` and ACR122U direct USB/CCID
`acr122_usb:001:002`, 2026-11-13, `pcscd` stopped, owned MIFARE DESFire EV3 8K
(random UID `04 4D 09 A2 BB 17 90`). The gated
test formatted the PICC (free memory 8192), created reserved AES `F0 0D 08`
with two keys, created Standard, Backup, Value, Linear Record, Cyclic Record,
and Transaction MAC files through `Service::desfire_create_file`, changed the
Standard file to enciphered with `read free, write key 0, rw denied, change key
0`, deleted all six files and the application through the service with
manifests in a temporary directory, and confirmed `applications: none`:

```text
application F0 0D 08 created
keys: 2 · AES
free: 8192 → 8096 bytes
file 01: standard data · plain · 16 bytes
file 02: backup data · plain · 16 bytes
file 03: value · plain · limits -50..50 · limited credit value 0
file 04: linear record · plain · 4-byte records · 0/4
file 05: cyclic record · plain · 4-byte records · 0/2
file 06: transaction MAC · plain · read free, write denied, rw denied, change key 0 · key option 0x02 · key version 1
file 01: before standard data · plain; after standard data · enciphered; change applied
file 01..06 deleted with manifests
application F0 0D 08 deleted
applications: none
```

Free memory stayed at 7648 bytes after the deletions, as DESFire keeps deleted
application and file space until `FormatPICC`. The manifest files record the
settings and raw settings bytes only.

The CLI then ran the same management cycle on both readers:

```text
$ nfctl --device <connstring> desfire app-create --aid F00D09 \
    --key <8-byte zero> --keys 1 --app-key-type aes
application F0 0D 09 created
keys: 1 · AES
free: 7648 → 7584 bytes

$ nfctl ... desfire file-create --aid F00D09 --file 01 --type standard \
    --communication plain --access-rights 0000 --size 16 \
    --key <16-byte zero> --key-type aes
file 01 created
type: standard data · plain · read key 0, write key 0, rw key 0, change key 0 · 16 bytes

$ nfctl ... desfire file-settings --aid F00D09 --file 01 \
    --communication enciphered --access-rights 0000 \
    --key <16-byte zero> --key-type aes
before: standard data · plain
after: standard data · enciphered
change: applied

$ nfctl ... desfire file-delete --aid F00D09 --file 01 --advanced --yes \
    --manifest /tmp/nfc-desfire-file-manifest.json
file 01 deleted
manifest: /tmp/nfc-desfire-file-manifest.json

$ nfctl ... desfire app-delete --aid F00D09 --key <8-byte zero> \
    --advanced --yes --manifest /tmp/nfc-desfire-app-manifest.json
application F0 0D 09 deleted
not a byte-for-byte restore

$ nfctl ... desfire info
applications: none
```

Both readers produced the same results.

The GTK Manage tab is covered by the UI test; it was not driven against
hardware in this slice.

## Next

26.10 adds value/record/transaction writes. 26.11 adds key management, 26.12
the NDEF replace and FormatPICC paths, and 26.13 the shared write gates and
journal.
