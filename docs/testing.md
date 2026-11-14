# Testing strategy

## Layers

1. **Pure unit tests**: card geometry, BCC, access bits, value blocks, dump,
   diff, NDEF/TLV, templates, validation.
2. **Property tests**: codec round trips, arbitrary malformed bytes, length
   boundaries, unknown TLV preservation.
3. **Transport tests**: mock reader with scripted cards, authentication
   failures, timeouts, disconnects, cancellation, card changes.
4. **Application tests**: workflows, backups, preflight, read-back verification,
   error mapping, CLI contracts.
5. **UI integration tests**: GTK4 views, focus, keyboard, overlays,
   destructive confirmations, empty/loading/error states.
6. **Hardware tests**: explicit, environment-gated, never run by `cargo test`.

## Hardware gates

Hardware tests are compiled only with the `hardware` feature and run only when
the environment selects them:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfc-drivers --features hardware -- --ignored
```

Required gates:

- `NFC_FOUNDRY_HARDWARE=1`: enable hardware tests.
- `NFC_FOUNDRY_DEVICE`: exact connstring.
- `NFC_FOUNDRY_HARDWARE_KEY_2` / `NFC_FOUNDRY_HARDWARE_KEY_3`: optional extra
  12-hex card key candidates, used when one card has sectors with different
  keys.
- `NFC_FOUNDRY_HARDWARE_KEY`: 12-hex MIFARE Classic key; defaults to
  `FFFFFFFFFFFF`.
- `NFC_FOUNDRY_HARDWARE_KEY_TYPE`: `A` or `B`; defaults to `A`.
- `NFC_FOUNDRY_HARDWARE_BLOCK`: Classic block for read/write tests; defaults to `4`.
- `NFC_FOUNDRY_HARDWARE_SECTOR0_KEY`: optional 12-hex Key A for manufacturer block 0
  when it differs from `NFC_FOUNDRY_HARDWARE_KEY`.
- `NFC_FOUNDRY_HARDWARE_WRITE=yes`: allow any write. The phase 02 test
  writes the original value back and refuses block 0 and sector trailers. The
  phase 04 workflow test changes one ordinary data block temporarily and writes
  the original back.
- `NFC_FOUNDRY_HARDWARE_RECOVERY=yes`: allow active recovery testing.
- `NFC_FOUNDRY_HARDWARE_NDEF_FORMAT=yes`: allow the Classic NDEF
  format/write/restore hardware round trip. It also requires
  `NFC_FOUNDRY_HARDWARE_WRITE=yes`.
- `NFC_FOUNDRY_HARDWARE_BACKUP_DIR`: preserve backups.

The PN532 read test is ignored unless selected:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfc-drivers --features hardware -- --ignored
```

Run it once with a MIFARE Classic card and once with a Type 2 tag; it reads the
selected card and reports the block or pages. The same-value write test is
compiled only with the additional `hardware-write` feature and requires
`NFC_FOUNDRY_HARDWARE_WRITE=yes`:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
cargo test -p nfc-drivers --features hardware-write -- --ignored
```

A missing card fails with an instruction to place one; it is not skipped. Never
write a card by default. A test that cannot run must report the missing gate,
not skip silently in release validation.

The phase 02 hardware record covers firmware identification, Classic 1K
selection/authentication/read, Type 2 selection/read, and a raw 7-bit REQA with
a 16-bit ATQA response on a PN532 (CH340 UART, firmware 1.6). Wrong-reader and
missing-card errors were checked on hardware. The same-value write needs
explicit authorization and was not run. `cargo test --workspace` uses only the
scripted link and never touches hardware.

The phase 03 core tests are hardware-free. Deterministic vectors cover access
codes, value blocks, BCC, trailers, 1K/4K geometry, diff, and malformed dump
files. Property tests round-trip access and value codecs, walk every block in
both layouts, and round-trip raw, project, MCT, and Proxmark3 images. Real MCT
and Proxmark3 client file interoperability remains a release check.

The phase 04 workflow tests are hardware-free. `nfc-app` tests use a scripted
transport with per-sector key rules, ignored writes, failing writes, and
cancellation hooks. `nfctl` integration tests run the built binary against the
mock Classic image.

The phase 29.1 CLI contract tests are hardware-free and run the built binary in
an isolated `HOME`/`XDG_CONFIG_HOME`/`XDG_DATA_HOME`. `crates/nfctl/tests/cli_contract.rs`
covers the exact envelope key set, card facts, typed results for every command
family, key redaction, export refusal, exit codes, JSONL progress plus one
terminal result, deadline expiry, dry-run rejection and planning, human
stdout/stderr separation, and sandboxed backup paths. Unit tests in
`error.rs`/`runtime.rs` cover the exhaustive `NfcError` classification, the
deadline flag, durations, and output-mode pre-scan. See
[cli.md](cli.md) for the contract.

The phase 29.2 reader-lock tests are hardware-free. `nfc-app` covers exclusive
contention, release, stale-file reuse, private directory/file modes, stable
file names, and the device manager refusing to open a reader held by another
lock. `nfc-drivers` covers PN532 by-id preference and ACR122U serial identity.
`crates/nfctl/tests/cli_contract.rs` runs two real processes against one mock
reader: the loser exits 4 with `reader_busy`, and the reader is usable after
the holder is stopped. Every CLI sandbox sets `XDG_RUNTIME_DIR` so lock files
stay inside the test directory. See [29b-reader-lock.md](specs/29b-reader-lock.md).

The phase 29.3 device-selection tests are hardware-free. Unit tests in
`crates/nfctl/src/select.rs` cover flag precedence, environment selection,
single-reader auto-selection, an empty environment value, zero readers,
several readers with named choices, and unknown values from each source.
`cli_contract.rs` runs the built binary with an isolated environment:
`NFC_FOUNDRY_DEVICE` selects a mock reader, `--device` overrides an invalid
environment value, an invalid environment target exits 2 with `usage`, and the
default multi-reader mock exits 2 instead of guessing. The GTK scenario
refreshes readers and asserts the selected name and exact connstring in the
reader panel. See [29c-device-selection.md](specs/29c-device-selection.md).

The phase 29.4 Connect/Disconnect tests are hardware-free. `nfc-app` covers
disconnect stopping polling, closing the reader, releasing the cross-process
lock, and the next read reopening the reader. The GTK scenario clicks
`Disconnect`, asserts the `Disconnected` status and disabled write gates, then
reconnects and reads again. See
[29d-gui-connect-disconnect.md](specs/29d-gui-connect-disconnect.md).

The phase 29.5 cancellation tests are hardware-free. The GTK scenario cancels
a read, key scan, recovery, Classic NDEF write, and restore from the Activity
task card and asserts `Cancelled` plus one Activity row; it also checks the
cancel action's token and phase transition and the terminal `Succeeded` state
after a read. `nfc-app` and `nfc-type2` cancel a token inside a write and assert
that the read-back still runs, so a committed write is never reported without
verification. The GTK scenario requires a display:

```sh
NFC_FOUNDRY_GUI_TESTS=1 xvfb-run -a cargo test -p nfc-foundry --bin \
  nfc-foundry app::tests::gtk_workbench_scenarios -- --exact
```

See [29e-cancellation-tasks.md](specs/29e-cancellation-tasks.md).

The phase 29.6 developer-tools tests are hardware-free and cover both sides of
the feature. A default build asserts that `pn532` and `apdu` are absent from
help and rejected by the parser; a `developer-tools` build asserts that they
are listed, that the console prints its bypass warning and leaks neither a
candidate key nor a Type 2 password, and that `apdu --raw` returns
`data`/`sw`/`statusOk` without echoing the request. `nfc-app` covers a console
exchange with no card and the ISO-DEP requirement for a raw APDU. See
[29f-developer-tools.md](specs/29f-developer-tools.md).

The phase 08 NDEF tests are hardware-free. `nfc-ndef` has deterministic tests
for the Text RTD example, short/normal/chunked record layouts, all 36 URI
prefixes, TLV length forms through 65,535 octets, unknown TLV and padding
preservation, capacity boundaries, vCard/WSC/BLE/DPP payloads, and malformed
records and TLVs. Property tests cover message and chunked encode/decode round
trips, arbitrary-byte decode stability, TLV stream round trips, and unknown TLV
preservation. The crate depends only on `nfc-core`; it never touches hardware.

The phase 09 Type 2 tests are hardware-free. `nfc-type2` covers CC parsing,
product layouts, static/dynamic lock positions, configuration, NDEF state and
TLV preservation, project dumps, partial reads, verified writes, ignored
writes, locks, PWD_AUTH verification, and restore validation. `nfc-app` adds
scripted workflows for backups, Advanced confirmations, NDEF write/read/format,
lock denial, password configuration, and CC mismatch. `nfctl` exercises
`type2 info|write|dump|restore|lock` and `ndef write|read|format` against the
mock NTAG215. Driver unit tests assert the Type 2 READ CRC (`30 04 26 EE`) and
raw-mode restoration for both transports.

The phase 24a verified-key tests are hardware-free. `nfc-core` fixes the
`UID/ATQA/SAK` identity format; `nfc-classic` covers project version 1/2,
metadata validation, and metadata round trips; `nfc-app` covers key-store
version 1/2, card-scoped verification replacement, verified-key candidate
ordering, tolerant import reports, read capture, keyscan partial capture, and
dump metadata registration. `nfctl` covers read/dump capture, project metadata,
and import reports; the GTK scenario reads a card and asserts the key store and
its on-disk copy both gain the verified keys. Raw/MCT/Proxmark3 exports remain
image-only and are tested as lossy for metadata.

The phase 24c UID tests are hardware-free. `nfc-app` covers the ordinary
CUID/Gen2 backend, the Gen1A backend when the access bits deny a data write,
the Gen1A-to-standard fallback order, an unchanged target, a wrong sector-0
key, broken access bits and BCC, an ignored write, a lost acknowledgement that
the read-back confirms, card disappearance after the write, cancellation, a
card change before execution, token reuse and expiry, and an immutable-UID
card that changes nothing. `nfctl` covers the Advanced and confirmation gates,
dry run, invalid UID input, and a successful mock write with reselect evidence.
The GTK scenario covers the Advanced switch, the target dialog, cancel, the
plan dialog, and the automatic re-read. The hardware UID test is
environment-gated. It passed on 2026-11-11 on PN532 UART with a Gen1A magic
Classic 1K: UID `FA B9 CF F4` -> `FA B9 CF F5`, reselect and full block-0/BCC
verification, then restored. The ACR122U service path is not recorded.

The phase 25.1 Classic 4K record passed on 2026-11-12 on PN532 UART
(`pn532_uart:/dev/ttyUSB0`) with a Fudan FM11RF32N, UID `59 98 C9 C5`:
`nfctl scan` selected `MIFARE Classic 4K`; the read test reported 40/40
sectors and a 256/256-block project dump with 40/40 verified Key A keys; block 4
changed, read back, restored, and the final dump diff was 0 changed with 0
trailer changes. The card was unformatted, so the Classic NDEF round trip
reported `no MAD; state=Unformatted`, then formatted MAD2 with 3355-byte
capacity, wrote 3 blocks, and restored 47 written/208 unchanged blocks with 0
failures and a final diff of 0 changed. The static encrypted nonce probe
reported `the card does not return a static encrypted nonce`. No code change
was needed. The ACR122U 4K row remains unrecorded.

The phase 25.2 Ultralight EV1 record passed on 2026-11-12 on PN532 UART with
an MF0UL11, UID `04 43 47 4A 75 22 91`: GET_VERSION selected the 20-page EV1
layout without a CC; 20/20 pages and the EV1 configuration were readable. The
OTP CC `E1 10 06 00` was written after explicit confirmation, the NDEF capacity
became 45 bytes, and an empty NDEF state was verified. A Text record wrote,
read back as 28 bytes, and cleared to empty. Password protection with
`AUTH0=4`/`ACCESS=0x80` made an unauthenticated read fail, accepted the correct
PWD/PACK for 20/20 pages, refused a wrong PACK, and cleared to
`AUTH0=0xFF`/`ACCESS=0x00` with VCTID `0x05` preserved. The MF0UL21 layout and
128-byte OTP CC are unit-tested only; no UL21 card was available. The ACR122U
GET_VERSION and EV1 rows remain unrecorded.

The phase 25.3 FM11RF08S record passed on 2026-11-12 on PN532 UART with UID
`9C 81 95 D4`: the static encrypted profile was detected; the backdoor sample
`nt=5B296CC7 nt_enc=A456DE20 par_err=0F` matched the known-key decryption, and
the 59970-candidate solver list contained `FFFFFFFFFFFF`, matching PM3's
`staticnested_1nt` count for the same vector. `nfctl recover fm11rf08s`
recovered 32/32 sector keys and read-verified 16/16 sectors with no writes. The
without-backdoor distance path was not reliable under host-driven timing and is
not claimed. The ACR122U row remains unrecorded.

The phase 25.4 Type 2 UID record passed on 2026-11-12 on PN532 UART with a
7-byte UID-changeable NTAG213 clone: the same-value page 0/1/2 probe succeeded
with the UID unchanged; the guarded workflow changed `04 00 E3 C2 00 00 02` to
`04 00 E3 C2 00 00 03`, reselected and read back pages 0/1/2, then restored the
original UID. A fresh 45/45-page dump had a page JSON identical to the original;
static lock bytes remained `00 00`. The ACR122U row remains unrecorded.

The phase 25.5 DESFire record passed its inventory step on 2026-11-12 on PN532
UART: `nfctl desfire info` reported MIFARE DESFire EV3, version 51.0, storage
`0x1A`, key settings `0F 01`, and no applications. Legacy `Authenticate`
(`0x0A`) with a DES zero key and D40 send mode then completed mutual
authentication; AES EV2 and 3K3DES zero keys failed. The earlier 54-candidate
dictionary failed on framing, not on the key. A reserved temporary application
`F0 0D 01` and standard file `01` were then created, a 16-byte payload was
written and read back, and the application was deleted; `GetApplicationIDs`
was empty afterwards. The test is idempotent and uses only the reserved AID.
The inventory itself performed no authentication or write. The GTK test
`desfire_workspace_switches_and_parses_inputs` covers workspace selection,
Advanced gating, and the AID/key parsers; it does not touch hardware.

The phase 25.5 NDEF provisioning path passed on PN532 UART on 2026-11-12
with the DESFire EV3 (random UID, observed `04 4D 09 A2 BB 17 90`). The gated
test refuses to start when an NDEF application already exists, checks the
Advanced/confirmation gates and a wrong-key refusal, formats the card, writes
and clears a Text record through Type 4, deletes the NDEF application, and
compares the final application list with the pre-format list:

```sh
NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfctl --features hardware-write \
  --test hardware_desfire_ndef -- --ignored
```

The run created `D27600` with the `E103`/`E104` files, verified the 15-byte
CC and empty NDEF file through Type 4, wrote/read/cleared one Text record, and
removed the application; the final list was empty. The CLI
`format-ndef`/`info`/`delete-ndef` cycle passed on the same card. The first
run stopped at `CommitTransaction` with `0x910C` (`NoChanges`); treating that
status as success, as PM3 does, let the next run pass. ACR122U
`acr122_usb:001:002` (`pcscd` stopped) passed the same provisioning/write/
delete test and the CLI read cycle on 2026-11-12.

Phase 26.12 extends the same test with the replace path: after a second Text
write it calls `Service::desfire_ndef_replace`, checks that the result holds
the previous message and an empty NDEF read-back, then deletes the
application. The same test file also holds
`format_picc_erases_every_application`, which creates a scratch application,
calls `Service::desfire_format_picc`, and verifies an empty application list
and the free-memory total. FormatPICC is a destructive full-card reset and
requires an additional environment gate; the test is not part of the ordinary
run:

```sh
NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_FORMAT=yes NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfctl --features hardware-write \
  --test hardware_desfire_ndef -- --ignored format_picc_erases
```

Both tests passed on PN532 UART and ACR122U `acr122_usb:001:002` (`pcscd`
stopped) on 2026-11-14. The service-level tests in
`crates/nfc-app/tests/desfire_ndef.rs` cover the gates, a missing or foreign
NDEF application, a failed second copy after the delete, and the non-factory
key reset without hardware.

Phase 26.13 adds the shared session preflight, planned backup and manifest
paths, the file-settings gate, and the structured Activity entry.
`crates/nfc-app/tests/desfire_write.rs` covers the JSON backup (content and
mode 0600), an unwritable backup path before `WriteData`, a pre-cancelled
token, a card change, and a reader loss. `desfire_management.rs` covers the
file-settings Advanced/confirmation gates and read-only inspection. The CLI
tests cover the file-settings dry run, the planned write backup path, and the
manifest path before deletion. On hardware, the 26.9 management test passed
with the new gate and the 26.8 secure-messaging write test passed with
persisted backups on PN532 UART on 2026-11-14:

```sh
NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_FORMAT=yes NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfctl --features hardware-write \
  --test hardware_desfire_management -- --ignored
NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_FORMAT=yes NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfctl --features hardware-write \
  --test hardware_desfire_secure_messaging -- --ignored
```

The Activity journal is session-only. A persistent log file is not part of
this slice. Both tests also passed on ACR122U `acr122_usb:001:002` with
`pcscd` stopped on 2026-11-14; the CLI file-settings and manifest gate cycle
passed on the same reader.

```sh
NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfc-desfire --features hardware --test hardware_legacy -- --ignored
```

The phase 26.4 authentication tests are hardware-free except for the gated
fixture. `nfc-desfire` matches the AN12343 EV2 ENC/MAC session-key vectors,
checks AES/3DES CBC round trips, key-length mapping, and key validation.
`nfctl/tests/hardware_desfire_auth.rs` creates reserved 3K3DES and AES
applications with zero test keys, authenticates with `0x1A`, `0xAA`, and
`0x71`, checks session key lengths and EV2 ENC/MAC separation, and verifies
wrong-key/key-number/family failures. The same test runs `verify_key`
(authentication plus GetKeyVersion) and `card_uid` (legacy CRC16, EV1 CRC32,
EV2 CMAC) for each family. It passed on PN532 UART and ACR122U on 2026-11-12;
the verification checks passed on ACR122U on 2026-11-13 and on PN532 UART on
2026-11-14. Set
`NFC_FOUNDRY_HARDWARE_KEEP_APP=1` to keep the fixtures for `nfctl desfire
verify-key` and `desfire uid`; the next run without it removes them.
Deleted DESFire applications keep EEPROM tombstones until FormatPICC; the test
requires `NFC_FOUNDRY_HARDWARE_FORMAT=yes` to format the test card when free
memory is low (or always when set):

```sh
NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_FORMAT=yes NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfctl --features hardware-write \
  --test hardware_desfire_auth -- --ignored
```

3K3DES/AES reads use EV1/EV2 secure messaging. The `nfc-desfire` unit tests
cover the CMAC and CRC primitives, AN12343 EV2 MAC/IV vectors, chained
responses, EV2 counter advancement, and tampered responses. The
`hardware_desfire_secure_messaging` test creates reserved 3K3DES and AES
applications with plain, MACed, and enciphered files, reads them through the
direct layer and `nfc-app`, then writes a known range to each file through
`Service::desfire_write` and reads it back, including a Backup file whose
commit precedes the read-back. The read record passed on PN532 UART and
ACR122U on 2026-11-12; the write record passed on PN532 UART and ACR122U on
2026-11-13. The same `NFC_FOUNDRY_HARDWARE_FORMAT=yes` rule applies:

```sh
NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfctl --features hardware-write \
  --test hardware_desfire_secure_messaging -- --ignored
```

For ACR122U use `NFC_FOUNDRY_DEVICE=acr122_usb:<bus>:<address>` and stop
`pcscd` first.

The phase 26.8 write path is covered without hardware by `nfc-desfire` unit
tests for the EV1/EV2 plain, MACed, and enciphered wire bytes and response
checks, `nfc-app/tests/desfire_write.rs` for access, range, backup, commit, and
read-back gates, and the `nfctl` and GTK tests for the two surfaces. The
`hardware_desfire_secure_messaging` write pass above is the hardware gate. The
same test also supports `NFC_FOUNDRY_HARDWARE_KEEP_APP=1` for manual
`nfctl desfire write` checks.

The phase 26.9 management path is covered without hardware by
`nfc-desfire` payload and scripted-card tests, `nfc-app/tests/desfire_management.rs`
for capacity, read-back, manifest, and gate checks, and the CLI/GTK tests. The
gated `hardware_desfire_management` test creates a reserved AES application,
creates all six file types, changes a Standard file's settings, deletes every
file and the application with manifests, and confirms the application list is
empty. It passed on PN532 UART and ACR122U on 2026-11-13; set
`NFC_FOUNDRY_HARDWARE_FORMAT=yes` to clear DESFire tombstones first:

```sh
NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_DEVICE=acr122_usb:<bus>:<address> \
cargo test -p nfctl --features hardware-write \
  --test hardware_desfire_management -- --ignored
```

The phase 26.10 value and record path is covered without hardware by
`nfc-desfire` unit tests for the command payloads, TMI padding, the TMV vector,
the commit MAC response, and the Transaction MAC file read exclusion,
`nfc-app/tests/desfire_value_record.rs` for limits, access, append/update/clear,
cyclic overwrite, record bounds, TMAC verification, and the unverified report,
and the CLI/GTK tests. The gated `hardware_value_record` test in `nfc-desfire`
creates a reserved AES application with Value, Linear Record, Cyclic Record,
and Transaction MAC files, recomputes every commit TMV, verifies the stored
Transaction MAC file, and deletes the application:

```sh
NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 cargo test -p nfc-desfire --features hardware \
  --test hardware_value_record -- --ignored
```

It passed on PN532 UART and ACR122U `acr122_usb:001:002` (`pcscd` stopped) on
2026-11-14 under AES EV1 and AES EV2. Set
`NFC_FOUNDRY_HARDWARE_KEY_TYPE=ev2` for the EV2 run and
`NFC_FOUNDRY_HARDWARE_FORMAT=yes` to erase DESFire tombstones first.

The phase 26.3 read path remains covered by unit
tests for the `GetValue` command shape and the `ReadRecords` payload, plus the
service check that a record response must be exactly `count * record_size`
bytes (short and long responses are rejected). The gated test in `nfctl/tests/hardware_desfire_files.rs`
creates a reserved application with a value file, a linear record file, and a
cyclic record file, reads them through `nfc-app`, rejects an over-request, and
removes the application. It passed on PN532 UART on 2026-11-12; set
`NFC_FOUNDRY_HARDWARE_KEEP_APP=1` to keep the fixture for manual CLI/GUI
inspection:

```sh
NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfctl --features hardware-write \
  --test hardware_desfire_files -- --ignored
```

The phase 26.6 Type 4 NDEF read is hardware-free in the CLI and GUI tests.
`nfctl --mock --device mock:type4-desfire desfire ndef-read` covers a Text
record, a proprietary File Control TLV, an unknown TLV, and `--output`. The
GUI formatter test covers the same fixture, and the DESFire workspace test
checks the Read NDEF control. On ACR122U the phase 25.5 NDEF round trip passed
and the CLI ran `format-ndef`, `ndef-read`, and `delete-ndef` on the same card;
`pcscd` must be stopped:

```sh
NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_DEVICE=acr122_usb:001:002 \
cargo test -p nfctl --features hardware-write \
  --test hardware_desfire_ndef -- --ignored
```

The phase 26.2 file-settings decoder is hardware-free. `nfc-desfire` covers
one valid and one malformed vector for each of the six file types, additional
access rights, truncation, trailing bytes, reserved communication mode 2,
reserved value flags, record-count inconsistency, and an unknown file type.
`nfc-app` tests the shared inventory renderer: a decoded standard file is
summarized, an undecodable entry is marked `undecoded`, and raw settings are
kept in both cases. On 2026-11-12 the NDEF application's `E103`/`E104`
standard files were decoded on PN532 UART through `nfctl desfire info`; the
Value, Record, and Transaction MAC layouts have unit vectors only. ACR122U
`acr122_usb:001:002` decoded the phase 26.5 standard and value fixture
settings.

The phase 10 Classic NDEF tests are hardware-free. `nfc-classic-ndef` covers
MAD1/MAD2 and CRC vectors, NFC AIDs, contiguity, GPB mapping version and
read-only state, TLV framing/capacity, long messages, leading proprietary TLV
preservation, and partial data areas. `nfc-app` adds scripted 1K and 4K format,
NDEF write/read, confirmation, complete-read enforcement, Key B preservation,
read-only refusal, unformatted refusal, and malformed MAD tests. `nfctl`
exercises `classic ndef info|read|write|format`.

The phase 11 Type 4 tests are hardware-free. `nfc-type4` covers NTAG 424 and
DESFire capability containers, read-only and proprietary access, malformed
lengths and versions, unknown TLV preservation, APDU offsets, empty and
populated NDEF files, MLe and MLc chunk boundaries, ignored writes, and
cancellation. Property tests feed arbitrary CC and NDEF file bytes and require
typed results instead of panics. `nfc-app` adds scripted read, write with a
pre-mutation backup, clear confirmation, read-only refusal without a backup,
ignored-write detection with a backup, dump, and cancellation. The driver
tests cover the PN532 UART and ACR122U APDU command shape, status-word split,
non-ISO rejection, raw-mode rejection, and the shared 247-byte command limit.
`MockReader` contains a stateful Type 4 tag so the adapter exercises the real
SELECT/READ/UPDATE sequence.

The gated Type 4 hardware test lives in `nfctl/tests/hardware_type4.rs`. The
read-only check needs a Type 4 / ISO-DEP tag:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfctl --features hardware --test hardware_type4 -- --ignored
```

The write/read/restore round trip needs the write gate and restores the
original NDEF message or empty state afterwards:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_BACKUP_DIR=/tmp/nfc-type4-backups \
cargo test -p nfctl --features hardware-write --test hardware_type4 -- --ignored
```

The read and write tests passed on 2026-11-12 on PN532 UART
(`pn532_uart:/dev/ttyUSB0`) with an NTAG 424 DNA `04 86 40 42 22 17 90`:
CC 2.0, 23 bytes, MLe 256, MLc 255, NDEF file `E104` with 256-byte capacity and
an empty state. A Text record was written, read back, cleared, and the empty
state re-read. ACR122U `acr122_usb:001:002` (`pcscd` stopped) passed the same
tests on 2026-11-14 against a DESFire EV3 with the AN11004 `D27600`
application: CC 2.0 (15 bytes), MLe 59, MLc 52, NDEF file `E104` 255 bytes;
Text write/read and restore to empty passed. Type 4 writes on a
random-UID DESFire compare ATQA, SAK, and the stable last four UID bytes
instead of the full UID.

The gated Classic NDEF hardware test lives in
`nfctl/tests/hardware_classic_ndef.rs`. The read-only check needs a Classic
card:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfctl --features hardware -- --ignored
```

The format/write/read/restore round trip requires both write tokens and restores
the original dump afterwards. It passed on PN532 UART on 2026-11-10 with a
blank Classic 1K, UID `6A 31 5E 28`; the restored dump matched byte-for-byte:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_NDEF_FORMAT=yes \
cargo test -p nfctl --features hardware-write -- --ignored
```

The gated Type 2 hardware tests live in `nfctl/tests/hardware_type2.rs`. The
read-only check needs a Type 2 tag:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=acr122_usb:001:017 \
cargo test -p nfctl --features hardware -- --ignored
```

The NDEF round trip additionally needs the write gate. It reads the original
image, formats, writes a Text record, verifies it, and restores the original
user memory from a backup:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=acr122_usb:001:017 \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_BACKUP_DIR=/tmp/nfc-type2-backups \
cargo test -p nfctl --features hardware-write -- --ignored
```

The password test additionally requires `NFC_FOUNDRY_TYPE2_PASSWORD` (8 hex
digits) and `NFC_FOUNDRY_TYPE2_PACK` (4 hex digits). It runs on an owned,
initially unprotected NTAG21x: it sets `AUTH0=0x20` with `PROT`, proves a wrong
PACK is refused, reads and writes NDEF through PWD_AUTH, clears protection with
the current password, restores the original user memory, and checks the
original NDEF state, lock bytes, and configuration. The read and NDEF round-trip
tests passed on a PN532 UART with an NTAG215 on 2026-11-10. The authenticated
round trip passed on a PN532 UART with an NTAG216 on 2026-11-12; the ACR122U
record remains pending.

Hardware workflow checks:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfctl --features hardware -- --ignored
```

The read-only test dumps the card and scans Key A per sector. The temporary
write test requires the additional feature and gate:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
NFC_FOUNDRY_HARDWARE_KEY=FFFFFFFFFFFF \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_BLOCK=4 \
cargo test -p nfctl --features hardware-write -- --ignored
```

It changes one ordinary data block, writes the original value back, and
verifies the final read-back. It refuses block 0 and sector trailers.

ACR122U hardware checks use the same gates with an `acr122_usb:` connstring.
Stop `pcscd` first because it holds the same CCID interface:

```sh
sudo systemctl stop pcscd
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=acr122_usb:001:017 \
cargo test -p nfc-drivers --features hardware -- --ignored
```

The Gen1A write additionally requires `hardware-write` and two confirmations:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE=acr122_usb:001:017 \
NFC_FOUNDRY_HARDWARE_KEY=<sector-key> \
NFC_FOUNDRY_HARDWARE_SECTOR0_KEY=<sector-0-key> \
NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_GEN1A_CONFIRM=yes \
cargo test -p nfc-drivers --features hardware-write -- --ignored
```

The test writes the current manufacturer block back unchanged unless
`NFC_FOUNDRY_TEST_UID` is set, then reselects and verifies block 0. It refuses a card
that is not a four-byte UID MIFARE Classic 1K and a block with an invalid BCC.
Restart `pcscd` afterwards when the PC/SC transport is needed.

ACR122U PC/SC checks need a running `pcscd` and no direct-USB client:

```sh
NFC_FOUNDRY_HARDWARE=1 \
NFC_FOUNDRY_DEVICE='acr122_pcsc:ACS ACR122U 00 00' \
cargo test -p nfc-drivers --features hardware --test hardware_acr122_pcsc -- --ignored
```

The Gen1A row uses the same `hardware-write` feature plus
`NFC_FOUNDRY_HARDWARE_WRITE=yes` and `NFC_FOUNDRY_HARDWARE_GEN1A_CONFIRM=yes`.
The same connstring works for the `nfctl` workflow tests. A stopped service
yields no PC/SC devices; `nfctl diag` reports it without hiding the direct USB
reader.

The phase 05 hardware record on 2026-11-10 used an ACR122U at
`acr122_usb:001:017` with internal PN532 firmware 1.6:

- Classic 1K `FA B9 CF F4`, block 4 with Key A `D3F7D3F7D3F7`:
  `03 2B D1 01 27 54 02 65 6E 4E 46 43 20 46 6F 75`;
- Type 2 tag `04 34 4C C2 83 5C 80`, pages 4..7:
  `03 4B D2 0A 3E 74 65 78 74 2F 76 63 61 72 64 42`;
- raw 7-bit REQA: ATQA `04 00` in 16 bits;
- Gen1A manufacturer block 0 same-value write on the owned magic Classic 1K
  card, verified by reselection and read-back; the UID stayed
  `FA B9 CF F4`;
- several open/close cycles; a physical unplug/replug was not exercised.

The phase 28 PC/SC record on 2026-11-14 used
`acr122_pcsc:ACS ACR122U 00 00`, internal PN532 firmware 1.6, with `pcscd`
active through socket activation:

- enumeration listed `ACR122U PC/SC` next to `ACR122U USB`;
- Classic 1K `FA B9 CF F4`: dump 64/64 blocks and 16/16 sectors, keyscan 44
  attempts and 16 verified Key A, data-block write/restore (block 4), trailer
  write/restore (block 11, GPB `40` → `BF` → `40`), UID change/restore through
  `nfc-app`, Gen1A same-value block 0 write, Classic NDEF write/read/restore;
- raw 7-bit REQA returned ATQA `04 00` in 16 bits;
- Recovery parity returned a 32-bit tag nonce;
- NTAG215 `04 34 4C C2 83 5C 80`: 135/135 pages, NDEF vCard decoded, 33-byte
  Text record written/read, original user memory restored;
- DESFire EV3 8K: inventory, AN11004 `D27600` provisioning, CC 2.0
  (15 bytes), MLe 59/MLc 52, empty NDEF read, Text record written/read, empty
  state restored, application deleted and `applications: none` verified.

The phase 04 hardware record on 2026-11-10 used a PN532 over
`/dev/ttyUSB0` and a MIFARE Classic 1K (`FA B9 CF F4`):

- full dump with common candidates: 64/64 blocks and 16/16 sectors readable;
- key scan: 16/16 sectors, 37 attempts, 32 verified keys;
- temporary write on block 4 with Key A `D3F7D3F7D3F7`: written, read back,
  original restored, read back; two `0600` project backups;
- restore of the full dump after the temporary change: 1 written, 62
  unchanged, one `0600` pre-write backup;
- restore of the unchanged dump: 0 written, 63 unchanged.

The same run also exposed two hardware-only issues: repeated PN532 selection
needed an RF field cycle, and block 0 needs sector 0 authentication. Both are
now covered by tests and documented in
`docs/notes/04-classic-workflows.md`. Trailer writes and a 4K card run remain
future hardware checks.


Recovery hardware checks are read-only and require an owned card:

```sh
NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_DEVICE=acr122_usb:001:017 NFC_FOUNDRY_HARDWARE_KEY=D3F7D3F7D3F7 cargo test -p nfc-recover --test hardware_recovery -- --ignored --nocapture
```

The suite checks parity-disabled partial authentication, software Crypto1
authentication, nested nonce distance, nested recovery of a known target key,
static-nested recovery, hardnested acquisition and key recovery, and darkside
repeated-nonce collection. The hardnested case is the long one: it loads the
PM3 tables, collects until all 256 first nonce bytes are present, and runs the
bit-sliced solver. `NFC_FOUNDRY_HARDNESTED_MAX_NONCES` caps the acquisition (default
4000) and `NFC_FOUNDRY_HARDNESTED_TABLES` selects a table directory. Place a
MIFARE Classic 1K card on the reader. The current test fixture uses block 4 Key
A `D3F7D3F7D3F7` and target block 0 Key A `A0A1A2A3A4A5`.

Both transports complete full software authentication. The tests need the key
that actually belongs to the card in the reader: the two test cards share UID
`FA B9 CF F4` but use different sector keys. Set `NFC_FOUNDRY_HARDWARE_KEY` and
`NFC_FOUNDRY_HARDWARE_TARGET_KEY` for the card under test. Darkside collection over
either host-driven reader normally finds no eight-observation repeated-nonce
group and must report that result; it does not fabricate a key.

The phase 06 hardware record on 2026-11-10 used two MIFARE Classic 1K cards
that share UID `FA B9 CF F4` but have different keys.

ACR122U at `acr122_usb:001:017`:

- parity probe `07 04 A6 75`, 32 bits, parity `0 0 1 0`;
- software authentication `nt=F7159B6A`;
- nested measured distance `160`, window `156..164`;
- nested, static-nested, and auto recovered sector 0 Key A
  `A0A1A2A3A4A5` from block 4 Key A `D3F7D3F7D3F7` and verified it;
- hardnested collected 1840 frames / 1805 distinct nonces, reached 256/256
  first bytes, estimated 2.309e8 states, tested 729232 state pairs, recovered
  the sector 0 Key A target, and verified it with a fresh authentication;
- darkside found no repeated-nonce sample in 200 tight-loop attempts.

PN532 UART at `pn532_uart:/dev/ttyUSB0` with canonical keys
`FFFFFFFFFFFF`:

- parity probe returned a 32-bit nonce;
- software authentication completed;
- nested measured distance `128`, window `124..132`;
- nested, static-nested, and auto recovered sector 5 Key A
  `FBBECDF6304E` from block 4 Key A `FFFFFFFFFFFF` and verified it;
- hardnested collected 1816 frames / 1795 distinct nonces, reached 256/256
  first bytes, tested 2152066416 state pairs through the all-bitflip path,
  recovered the same target key, and verified it with a fresh authentication;
- darkside found no repeated-nonce sample.

## Hardware failure triage

A silent or malformed reader response is a driver bug until the reference path
proves otherwise. Before reporting a hardware blocker:

1. Verify the same exact port with the known-good stack:

   ```sh
   LIBNFC_DEVICE=pn532_uart:/dev/ttyUSB0 nfc-list -v
   ```

2. Capture the reference exchange:

   ```sh
   LIBNFC_DEVICE=pn532_uart:/dev/ttyUSB0 \
   LIBNFC_LOG_LEVEL=3 nfc-list
   ```

3. Compare the reference host frames with the Rust driver byte-for-byte. Check
   the wake-up preamble, command item values, authentication payload, and card
   state transitions.
4. Check the validated Python scripts and the previous Go shim for the
   workflow-level sequence before concluding the adapter is unsupported.

If the reference fails too, record the exact error and the gates used. If the
reference works, fix the Rust driver and add a regression test.

## Required checks

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
reuse lint
```

For changed GTK screens, add GTK4 UI integration tests and a manual window
check for keyboard, focus, theme, zoom, and CJK input. The GTK app needs
`gtk4`/`libadwaita` development libraries to build; on a headless host, use a
private Xvfb display (for example `Xvfb :99 -screen 0 1400x920x24` with
`GDK_BACKEND=x11`). The phase 19a–19d workbench scenarios use a scripted
transport (Classic and NTAG213) and are gated:

```sh
env -u WAYLAND_DISPLAY NFC_FOUNDRY_GUI_TESTS=1 \
  xvfb-run -a env GDK_BACKEND=x11 \
  cargo test -p nfc-foundry gtk_workbench_scenarios
```

Without `NFC_FOUNDRY_GUI_TESTS=1` the test takes its skip branch so
`cargo test --workspace` still passes on a machine without a display. The same
gate covers the phase 19e language scenario: it switches to `zh-CN`, asserts
the immediate relabeling, persistence, and preservation of page/reader/card/
selection, runs a Classic read and a Type 2 read in Chinese, opens a Chinese
write dialog (heading, body, responses, close response), and fails if any
English chrome remains in the visible window or dialog. It then switches back
to English.

Phase 19e also adds an `i18n` unit test module: every catalog key has a
non-empty English and Chinese value and the same placeholder count, exact and
pattern lines translate both ways, multi-line labels keep their breaks, locale
detection prefers the saved choice, and preference codes round-trip. Settings
tests cover language persistence and rejection of an unknown code. The parity
matrix in `docs/design/parity.md` is checked against the `nfctl` command tree
and the GUI page/control tree; the audit added the GUI NDEF message export.

The navigation scenario visits every shell tab, checks the empty and busy
states, and verifies that the primary action is focusable and that the focus
chain advances. Escape/close behavior is asserted through the write dialog's
`cancel` close response.

For a visual Chinese check, run the built application on a private Xvfb
display with a temporary config and capture the window:

```sh
Xvfb :97 -screen 0 1500x1000x24 &
DISPLAY=:97 XDG_CONFIG_HOME=/tmp/nfc-zh GDK_BACKEND=x11 \
  target/debug/nfc-foundry &
DISPLAY=:97 import -window root /tmp/nfc-zh.png
```

Use `{"language":"zh-CN"}` in `/tmp/nfc-zh/nfc-foundry/ui.json`. Check CJK
glyphs, tab scrolling, and truncation; the 19e record used this capture.

The 19d scenarios cover keyscan progress, merge, and `.dic` export; the
recovery limitation surface and verified-key saving; diagnostics without card
I/O; activity timestamps, redaction, and clear; and settings persistence with
`0600` permissions plus invalid-path rejection. `nfc-app` adds a per-sector
keyscan progress assertion and recovery candidate expansion. `nfctl` covers
explicit `--key-file` keyscan candidates with common keys disabled and the
reader/build facts in `diag`. Settings unit tests cover missing and malformed
files, permissions, and path validation.

Feature-gated code is outside the default `cargo test --workspace` graph. Check
both sides of a feature:

```sh
cargo test -p nfctl -p nfc-app --features developer-tools
cargo clippy -p nfctl -p nfc-app --all-targets --features developer-tools -- \
  -D warnings
```

Release packages and the AUR recipes use default features, so they never
compile `pn532 shell` or `apdu --raw`.

For hardware features:

```sh
cargo clippy -p nfctl --features hardware-write --all-targets -- -D warnings
```

For changed driver hardware tests, also run:

```sh
cargo clippy -p nfc-drivers --features hardware --all-targets -- -D warnings
```

## Fixtures

- Classic dump and codec fixtures: access, value, BCC, trailer, geometry, diff,
  and malformed project/raw/MCT/Proxmark3 inputs.
- NDEF message/TLV vectors, including malformed and chunked records.
- PM3 `mfkey32v2`, `mfkey32nested`, `mfkey64`, and `nonce2key` golden
  vectors, plus captured hardware nonce/key samples from both readers.
- Mock transport scripts for card selection, auth, read/write, timeout,
  disconnect, and card change.
- Classic workflow scripts with per-sector key rules, denied sector keys,
  ignored writes, failing writes, and cancellation after a chosen attempt.
- Proxmark3 `mfc v2` golden JSON (`crates/nfc-classic/tests/fixtures/`) and
  PM3 `.dic` vectors for comments, inline comments, duplicates, lowercase, and
  malformed lines.

## What not to test

Do not add tests that only prove a wiring call, a copy, a mock echo, a default
literal, a non-empty vector, or a bare `is_ok()`. Test observable behavior,
boundaries, invariants, transitions, precedence, and failures.

The 2026-11-11 hardware revisit ran the five Classic `nfctl` checks on both
readers with `--test-threads=1`. The tests cover full dump and Key A scan,
ordinary data-block write/restore, sector-trailer write/restore, standalone UID
change/restore, and full dump restore with UID. `NFC_FOUNDRY_HARDWARE_KEY_2`
and `NFC_FOUNDRY_HARDWARE_KEY_3` supply extra per-sector keys.
`NFC_FOUNDRY_HARDWARE_TRAILER` selects the trailer block (default 7). Both
readers passed. ACR122U `acr122_usb:001:002` additionally passed the Classic
NDEF format/write/read/restore test after `pcscd` was stopped.

Trailer writes preserve a masked Key A or Key B from the authenticated key and
verify the access bytes and GPB on read-back. Restore fills masked trailer
slots from the dump key metadata. A generic single-block trailer write with a
B-only access condition and a masked Key A is refused; use a dump restore,
which has the recorded key metadata.
