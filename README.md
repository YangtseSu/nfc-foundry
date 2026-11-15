# NFC Foundry (Rust)

NFC Foundry is a Linux-only NFC reader workbench for MIFARE Classic and NFC
Forum Type 2 / NDEF cards, written in Rust.

## Status

**Release 0.3.0: the AUR source package, GitHub CI binary tarball, source
archive, and hardware matrix are in place.** PN532 UART passed every operation
Release 0.3.0 adds the read-only China T-Union and EMV payment-card
adapters, their `nfctl` commands, and the GTK T-Union/Bank card workspaces.
in its certified scope, including UFUID UID change and seal application. The
ACR122U passed its certified scope: selection, Classic reads, Type 2 reads,
raw REQA, Gen1A same-value block 0 write, recovery, and DESFire inventory,
3K3DES/AES authentication, and secure-messaging reads. ACR122U Classic data writes, ACR122U Type 2 write/NDEF, ACR122U
Classic NDEF, one-way locks, passwords, trailer writes, and ACR122U Type 4 are
not advertised because no release hardware record exists. Classic 4K read/write
is validated on PN532 UART; the ACR122U 4K path is not. Ultralight EV1 MF0UL11,
FM11RF08S backdoor recovery, and a 7-byte UID-changeable NTAG213 clone are
validated on PN532 UART. DESFire EV3 inventory, factory-key authentication, and
plain/MACed/enciphered file reads are validated on both readers; key
verification and authenticated UID reads are validated on ACR122U. The Type 4
NDEF read is exposed in CLI and GUI, and NDEF provisioning, guarded delete,
replace, and FormatPICC are implemented. Generic Standard/Backup file writes
are validated on PN532 UART and ACR122U for Plain, MACed, and Enciphered
modes. Every DESFire data write plans a JSON backup path before the first
byte; application and file deletes show a planned manifest path. Application
and file management (create/delete applications and all six file types,
settings changes with Advanced mode and typed confirmation, free memory,
manifest export) is validated on PN532 UART and
ACR122U. Value credit/debit/limited-credit, record append/update/clear, and
Transaction MAC verification are validated on PN532 UART; the ACR122U row is
pending. Key settings reads and changes, application key and PICC master key
changes, re-authentication, and key-version read-back are validated on PN532
UART under DES, AES EV1, and AES EV2; the ACR122U row is pending. Multiple key
sets and key rolling are blocked on the non-public EV2/EV3 key set encoding.
Transaction Timer configuration is blocked on the non-public PDCap2.2
encoding. Type 4
read/write is
validated on
PN532 UART but has no CLI or GUI surface yet. See
[Hardware matrix](docs/hardware-matrix.md).
Both first-release readers are implemented: PN532 UART and the ACR122U over
direct USB/CCID or PC/SC. The direct path claims the CCID interface with
`nusb`; the PC/SC path opens the exact named reader through `pcscd` with
`SCARD_SHARE_DIRECT` and the CCID escape control. Both carry PN532 commands in
the ACR122 pseudo-APDU and support selection, Classic authentication and block
access, raw bytes/bits, and a Gen1A manufacturer-block write capability.
Hardware validation covers Classic and Type 2 reads, a raw 7-bit REQA, and a
same-value Gen1A block 0 write verified by reselection and read-back. The
ACR122U PC/SC rows add Classic dump/keyscan, data-block and trailer writes,
UID change/restore, Classic NDEF write/restore, recovery parity, Type 2
read/NDEF write, and Type 4/DESFire NDEF read/write. The Classic workflow layer covers
key stores, common-key authentication, partial dumps, single-block reads and
writes, restores, key scans, and atomic project-format backups. Reads, scans,
and recovery capture verified sector keys into the card-scoped version-2 key
store, and project dumps are version 2 with keys, reader, and timestamps.
`nfctl` calls the
shared `nfc-app` methods. Every write checks
access conditions, saves a backup, writes the block, reads it back, and
rechecks card identity; restore requires confirmation, refuses to change
manufacturer block 0, writes data blocks before sector trailers, and stops at
the first failure by default. The GUI workbench stages edits with undo and
revert; selective restores accept an explicit block set or changed blocks only,
and both surfaces preview a write plan with block, trailer, access, and expiry
details before executing a one-shot token. Native recovery now covers Crypto1,
the `mfkey` solvers, parity-based nonce acquisition, nested and static-nested
recovery, and darkside collection, and hardnested with the PM3 state tables and
64-lane bit-sliced solver, exposed as
`nfctl recover auto|darkside|nested|staticnested|hardnested`. Both readers
complete software Crypto1 authentication, collect hardnested encrypted nonces
with fast select, and verify recovered keys. `recover auto` is a full autopwn:
it scans every sector, seeds with an EV1 signature key or host-driven
darkside, recovers the remaining keys with nested then hardnested, and
read-verifies the result. Darkside acquisition follows MFCUK's host-driven
sequence; it needs a card whose PRNG repeats a tag nonce.
Static encrypted nonce cards use the published FM11RF08S backdoor to acquire
clear nonces and recover keys natively; the without-backdoor distance path is
not claimed on host-driven readers. EV1/QL88 signature detection uses
the originality-signature block as a known key when no supplied key works.
The hardware-free NDEF core is complete: a strict message/record codec with
short, normal, and chunked layouts; Type 2 TLV framing with unknown and
proprietary preservation; Text, URI, Smart Poster, vCard, Wi-Fi WSC, Bluetooth
LE OOB, and DPP templates; and message/TLV capacity checks. The Type 2 / NTAG
adapter is implemented: CC and product layouts, static/dynamic locks, NTAG
configuration, project dumps, NDEF TLV preservation, and verified page and
NDEF writes over both readers. `nfctl type2|ndef` uses the shared `nfc-app`
services, with backups, read-back verification, and confirmation dialogs for
one-way locks and configuration. A PN532 UART
NTAG215 read plus authorized NDEF write/read/restore round trip is recorded for
2026-11-10. Authenticated access after AUTH0 is validated on PN532 UART with
an owned NTAG216 on 2026-11-12; the ACR122U record, lock path, and ACR122U
password path remain pending. The read path was validated earlier on both
readers. Classic NDEF maps NDEF onto MAD1,
MAD2, and NFC sectors: the adapter parses the directory and per-sector GPB,
reads and writes the mandatory NDEF Message TLV with access checks, and formats
a blank card after a confirmation and backup. `nfctl classic ndef` uses
the same services. The Classic mapping has no capability-container record; the
GPB carries the mapping version and access state. The GPUI Kit shell was
removed on `feat/gtk4-ui`. The GTK4 workbench in `crates/gui` provides the
shell, real reader selection, Classic read/table/inspector with sector-grouped
rows, masked hex/ASCII columns, and per-sector key/edit summaries, verified
block write, dump save/load, Keys, Key scan, Recovery, Diff, Restore, a shared
NDEF message workbench with multi-record editing, RTD forms, `.ndef` load/save,
verified write and format for Classic and Type 2, Type 2 Memory/Protection with
user-page and Advanced writes, locks, password configuration, and project
dump/restore, plus Diagnostics, Activity, and persisted Settings. The interface is complete in English and Simplified
Chinese (`zh-CN`), with an immediate switch that preserves the current page,
reader, card image, and selection; the choice persists in `ui.json` and the
system locale is used on first run. Every shipped `nfctl` card and file
operation has a GUI path with the same gates; see the
[parity matrix](docs/design/parity.md). The package installs both binaries, the
hicolor icon set, and the desktop entry. Phase 24 S1 (verified-key capture,
card-scoped key verification, and project dump v2 metadata), S2 (editable
workbench, selective restore, and write preflight), and S3a (standalone UID /
block-0 write) are complete. The UID path writes only four-byte UID MIFARE
Classic 1K magic cards, prefers the Gen1A protected writer on ACR122U and
PN532 UART, falls back to an ordinary CUID/Gen2 block-0 write, and reselects
and verifies the complete block 0, BCC, and sector trailer after every attempt.
The path is hardware-validated on PN532 UART and ACR122U with an owned Gen1A
card, on PN532 UART with an owned CUID/Gen2 card through the ordinary block-0
backend, and on PN532 UART with an owned UFUID. UFUID sealing is implemented
as a separate irreversible Advanced operation: it proves the Gen1A backdoor,
backs up sector 0, writes the canonical seal configuration, and requires the
backdoor to stop answering while the UID and sector data stay unchanged. The
seal command was applied on the owned UFUID with owner approval; the card is
now permanently sealed. The first automated verification returned a false negative while the backdoor
still answered immediate probes. Verification now polls for two consecutive
negative probes, covered by a delayed-probe test. The sealed card reads 64/64
blocks with block 0 unchanged, and a second seal preflight is refused. A clean
automated rerun is impossible on this card because the seal is permanent. S3b
(restore with UID) is complete:
a full restore runs under one preflight token, verifies the restored sector 0,
and only then writes the target UID. It is hardware-validated on PN532 UART and
ACR122U with an owned Gen1A Classic 1K. S4 (reader lifecycle and capability
model) is complete: `nfc-app` polls card presence with a bounded removal
tolerance, reports connecting/polling/busy/recovering/error/disconnected state,
reconnects with exponential backoff, and holds one exclusive operation lease.
The reader rail offers explicit Connect and Disconnect: Disconnect closes the
reader, releases the cross-process lock, and keeps the inspected bytes
inspection-only until the next read. Drivers
publish their capabilities; UID writes and explicit active recovery strategies
fail closed on a reader without the matching capability, and card removal
invalidates the GUI image and disables writes. S6 adds Type 2 authenticated
access:
one PWD_AUTH session per operation covers protected reads, page writes, NDEF
writes/formats, lock updates, restore, and configuration changes, and can clear
AUTH0 back to 0xFF. It is hardware-validated on PN532 UART with an owned
NTAG216; the ACR122U record is pending. S5 (surface and diagnostic gaps) is
complete: the Classic Details tab and `nfctl card health` share one health
model over the dump and key store, and the GUI adds the live workbench dirty
set; the key catalog shows built-in
common keys, deletes stored entries, and surfaces the line-numbered import
report; MIFARE Mini is identified from SAK `0x09` without a layout or
read/write claim; `nfctl diag --json` and the GUI diagnostics page share a
deterministic self-check with version, commit, build date, platform, drivers,
selected-reader firmware and capabilities, and hardnested table status; and
both recovery surfaces require authorization before the engine starts.
Remaining work is the Type 4 read surface, the rest of phase 25, and phases
13-16. Phase 17 is implemented: the Amiibo decrypt/encrypt path is validated
against user-supplied dumps, and a full NTAG215 write/read-back passed on PN532
UART; the ACR122U write record is pending.

## Product scope

Two first-class domains:

1. **MIFARE Classic workbench**: discover readers, select cards, authenticate,
   read/write/dump/restore, keys, diff, clone, factory format, UID safety, and
   native key recovery.
2. **NDEF**: read, write, format, and protect NDEF data on Type 2 / NTAG and
   MIFARE Classic; a Type 4 / ISO-DEP adapter and application workflows are
   implemented and validated on PN532 UART, with the ACR122U path pending and
   no CLI or GUI surface yet.

Read-only ISO-DEP adapters are available separately from the two domains:

- `nfctl tunion read` and the GTK `China T-Union` workspace show the
  application, masked card number, balance, and transaction/travel records on
  PN532 UART and both ACR122U transports.
- `nfctl emv read` and the GTK `Bank card` workspace show the PPSE application
  list, selected AID/label, masked PAN, expiry, cardholder name when present,
  AIP/AFL/ATC facts, and record count on PN532 UART and ACR122U direct USB.

Neither adapter has a credit, purchase, top-up, PIN, or write path. EMV
application cryptograms are discarded and never sent to an issuer.

Amiibo is an application on the Type 2 path: read and decrypt with user key
files, encrypt a user dump, and write a re-signed image to an owned unlocked
magic NTAG215. No Nintendo keys or Amiibo data ship with the product.

Supported hardware for the first release:

- PN532 UART (`pn532_uart`)
- ACR122U direct USB/CCID (`acr122_usb`)
- ACR122U PC/SC (`acr122_pcsc`)

Out of scope for the first release: PN53x USB, generic or PC/SC-only readers,
Proxmark3 device control, Windows, macOS, containers, AppImage/Flatpak/Snap,
telemetry, a browser-first UI, and a GUI command terminal or shell launch.

Post-release work is listed in the phase plan: phase 13 is the pending RF and
transport prerequisite; phases 14-16 are optional read-only work covering
Type 3/4B NDEF, EMV public data, China T-Union, FMCOS/Calypso, and ICAO 9303
eMRTD. Phase 14a (China T-Union) is implemented and validated on PN532 UART
and both ACR122U transports; phase 14b (EMV public data) is validated on
PN532 UART and ACR122U direct USB. The rest of phase 14 and the Type B paths
still need phase 13. Phase 16 remains blocked on an authorized SAM. Phase 17 is the
controlled Amiibo path with user-supplied keys and data. The read-only phases
add no write or transaction path; Amiibo writes are Advanced,
user-key/user-data only, and limited to owned unlocked magic NTAG215 tags. No
hardware support is claimed until validated.

## Technical stack

- Rust latest stable, edition 2024.
- GUI: GTK4-rs (`gtk4` + `libadwaita`) in `crates/gui`, one frontend only,
  decided in [ADR 0006](docs/adr/0006-gtk4-rs.md). The GPUI Kit implementation
  was removed and is not a fallback.
- Transport: pure Rust drivers over `serialport` (PN532 UART), `nusb`
  (ACR122U USB/CCID), and `pcsc` over system `libpcsclite` (ACR122U PC/SC). No
  C shim, no CGO, no `libnfc`.
- Recovery: native Rust, ported from the current
  [RfidResearchGroup/proxmark3](https://github.com/RfidResearchGroup/proxmark3)
  implementation. The old `mfoc`/`mfcuk`/`mfoc-hardnested` binaries are
  differential test oracles only.
- CLI: `nfctl`, English-only, shares the application services with the GUI.
- License: GPL-3.0-or-later.

## Workspace

```text
crates/
  nfc-core            # card identity, errors, shared primitives
  nfc-transport       # Reader/Transport traits
  nfc-drivers         # pn532_uart, acr122_usb, acr122_pcsc
  nfc-classic         # geometry, BCC, access, value, dump, diff, exchange
  nfc-recover         # Crypto1 and Classic key-recovery attacks
  nfc-ndef            # NDEF/TLV/message/record core
  nfc-type2           # Type 2 / NTAG page and NDEF adapter
  nfc-amiibo          # Amiibo key records, crypto, NTAG215 image, UID rebinding
  nfc-classic-ndef    # MAD / Classic NDEF adapter
  nfc-type4           # ISO-DEP/APDU CC and NDEF file read/write
  nfc-desfire         # DESFire native commands, secure messaging, mock card
  nfc-pboc            # read-only ISO 7816-4 public data; T-Union and EMV profiles
  nfc-app             # device manager, services, workflows, events
  nfctl               # CLI
```

Dependency direction:

```text
nfctl -> nfc-app -> nfc-recover / nfc-classic / nfc-type2 / nfc-classic-ndef
      -> nfc-amiibo / nfc-desfire / nfc-pboc / nfc-ndef -> nfc-core
nfc-drivers -> nfc-transport
```

## Install

Arch Linux AUR source package:

```sh
cd packaging/aur
makepkg -sri
```

Binary tarball from the GitHub release:

```sh
tar -xzf nfc-foundry-0.3.0-x86_64.tar.gz
./nfc-foundry-0.3.0-x86_64/bin/nfctl diag
```

The package and tarball install `nfc-foundry` and `nfctl`, the ACR122U udev
rule, the hardnested tables, the hicolor icon set, `nfc-foundry.desktop`,
licenses, and notices. All paths use the `nfc-foundry` namespace, so another
NFC reader application can stay installed; only one process may own a reader at
a time. See [Packaging](docs/packaging.md).

## Build

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

The GTK4 application is a workspace member. Build and run it with:

```sh
cargo build -p nfc-foundry
cargo run -p nfc-foundry
```

It needs GTK 4.10 or newer, libadwaita 1.5 or newer, and libpcsclite
development/runtime libraries. On Arch Linux: `gtk4`, `libadwaita`,
`hicolor-icon-theme`, `pcsclite`; on Debian/Ubuntu: `libgtk-4-dev`,
`libadwaita-1-dev`, `hicolor-icon-theme`, `libpcsclite-dev`. The language
selector is in Settings; `nfctl` stays English under every locale.

Hardware tests never run by default. They require an explicit feature and
environment variable; see `docs/testing.md`.

## CLI machine output

`nfctl` keeps human output on stdout and diagnostics on stderr. For scripts:

```sh
nfctl --mock list --json
nfctl --device pn532_uart:/dev/ttyUSB0 scan --json
nfctl --device pn532_uart:/dev/ttyUSB0 classic read --block 4 --json \
  | jq -r .result.hex
nfctl --device pn532_uart:/dev/ttyUSB0 classic keyscan --jsonl
nfctl --timeout 30s --device pn532_uart:/dev/ttyUSB0 \
  classic dump --output card-dump.bin --json
nfctl --mock classic keys export --all-candidates --output all.dic
```

- `--json` writes one envelope: `ok`, `command`, `device`, `card`, `warnings`,
  `error`, and a command-specific `result`.
- `--jsonl` streams compact progress events and one terminal result.
- `--timeout 30s` bounds the whole operation; `--verbose` adds reader detail.
- Reader selection is `--device`, then `NFC_FOUNDRY_DEVICE`, then the only
  enumerated reader; several readers without a choice exit 2.
- Stable exit codes: 0 success, 1 operation, 2 usage, 3 validation, 4
  hardware/permission/busy, 5 reserved, 6 cancelled, 7 internal.
- Keys, Type 2 PWD/PACK, DESFire key material, and dump payloads never appear
  in machine output. `classic keys export` requires `--output` with `--json` or
  `--jsonl`.
- Raw, MCT, and Proxmark3 dumps write a `<image>.nfcx.json` sidecar with card
  identity, read times, and sector keys; `--no-sidecar` disables it. Project
  dumps keep that metadata inline.
- `classic keys export --all-candidates` writes the sorted union of stored
  keys and the built-in dictionary without changing the store; the default is
  stored-only.

Read [docs/cli.md](docs/cli.md) for every command name, result shape, error
code, and JSONL event.

## Mock reader

The CLI can run without hardware against scripted readers:

```sh
cargo run -p nfctl -- --mock list
cargo run -p nfctl -- --mock --device mock:classic-1k scan
cargo run -p nfctl -- --mock --device mock:ntag215 card info
```

The mock Classic 1K carries a complete BCC-valid image, so the workflow
commands can be exercised without a card:

```sh
cargo run -p nfctl -- --mock --device mock:classic-1k classic dump \
  --output /tmp/card-dump.bin
cargo run -p nfctl -- --mock --device mock:classic-1k classic read --block 4
cargo run -p nfctl -- --mock --device mock:classic-1k \
  --backup-dir /tmp/nfc-backups classic write --block 4 \
  --data 000102030405060708090A0B0C0D0E0F
cargo run -p nfctl -- --mock --device mock:classic-1k classic keyscan
```

`classic dump` defaults to `--format auto`: a complete image is written as a
Proxmark3 raw `.bin`, and a partial image as the lossless NFC Foundry project
`.nfc-foundry.json`. `--format raw`, `proxmark3`, `mct`, and `project` select a
format explicitly; raw, Proxmark3 JSON, and MCT are refused for partial images
because unknown bytes cannot be represented. The project format is version 2
and carries sector keys, the reader connstring, and read timestamps; raw,
Proxmark3 JSON, and MCT are image-only and lose that metadata. Use
`--format project` when the dump must preserve keys. Backup files always use
the project format.

The mock Classic NDEF 1K carries a valid MAD1 image and a Text record:

```sh
cargo run -p nfctl -- --mock --device mock:classic-ndef classic ndef info
cargo run -p nfctl -- --mock --device mock:classic-ndef classic ndef read
cargo run -p nfctl -- --mock --device mock:classic-ndef classic ndef write --text hello
```

The mock NTAG215 carries a complete 135-page NTAG215 image with an empty NDEF
TLV:

```sh
cargo run -p nfctl -- --mock --device mock:ntag215 type2 info
cargo run -p nfctl -- --mock --device mock:ntag215 type2 read --page 4
cargo run -p nfctl -- --mock --device mock:ntag215 ndef write --text hello
cargo run -p nfctl -- --mock --device mock:ntag215 ndef read
cargo run -p nfctl -- --mock --device mock:ntag215 type2 dump --output ntag.json
```

The mock DESFire carries a free-access 32-byte standard file:

```sh
cargo run -p nfctl -- --mock --device mock:desfire \
  desfire write --aid F00D01 --file 01 --offset 4 --data 11223344
```

`nfctl classic keys list|import|add|export` manages the legacy JSON key store.
Pass `--key-store <path>` to choose it explicitly; otherwise the default is
`$XDG_CONFIG_HOME/nfc-foundry/classic-keys.json`. The primary per-card format
is a Proxmark3 `.dic` file under `$XDG_DATA_HOME/nfc-foundry/keys` (default
`~/.local/share/nfc-foundry/keys`). A dump and its key file share a base name
across `dumps/` and `keys/`; saving a dump merges the keys used by the read
into the paired `.dic`, and loading a dump or restoring from one loads that
file automatically. Dumps, NDEF messages, and pre-write backups use
`dumps/`, `ndef/`, and `backups/` under the same data directory; file dialogs
open the matching folder.

The key scan checks three standard keys on first run
(`FFFFFFFFFFFF`, `A0A1A2A3A4A5`, `D3F7D3F7D3F7`), remembers the checked set,
and offers the extended, EV1 signature, common, and loaded dictionary groups
unchecked. Reads try stored keys and then the three default keys; the full
dictionary belongs to an explicit key scan. `keys import` reports added,
duplicate, and ignored lines and continues past malformed entries. `keys
export --format dic` writes the Proxmark3 dictionary dialect: unique uppercase
12-hex keys, one per line. `.dic` cannot carry sector, A/B, or provenance, and
imported `.dic` keys apply to every sector as `any`. The JSON store keeps
sector, A/B, and card-scoped verifications for imported keys; duplicate
entries from different sources collapse to one key.

## Hardware reader

Device selection uses an exact connstring, taken from `--device`,
`NFC_FOUNDRY_DEVICE`, or the only enumerated reader. `nfctl list` reports
serial ports as `pn532_uart:` candidates, with USB serial adapters first and
legacy `/dev/ttyS*` nodes without detected hardware skipped, because a UART
adapter cannot be identified without opening it. Several readers require an
explicit choice rather than a guess. One process owns a reader at a time: a
second `nfctl` exits 4 with `reader_busy`, and the workbench shows the same
busy error, until the holder disconnects or exits. Opening validates the PN532
firmware:

```sh
cargo run -p nfctl -- list
cargo run -p nfctl -- --device pn532_uart:/dev/ttyUSB0 scan
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 cargo run -p nfctl -- scan
```

`LIBNFC_DEVICE` is not read; set `NFC_FOUNDRY_DEVICE` instead.

Card workflow example:

```sh
cargo run -p nfctl -- --device pn532_uart:/dev/ttyUSB0 \
  --key FFFFFFFFFFFF classic dump --output card-dump.bin
cargo run -p nfctl -- --device pn532_uart:/dev/ttyUSB0 \
  --key FFFFFFFFFFFF classic restore --input card-dump.bin --yes
```

PN532 UART firmware identification, Classic 1K selection/authentication/read,
Type 2 selection/read, raw 7-bit REQA, Classic dump, key scan, temporary block
write, and restore are hardware-validated. Writes remain gated by
`NFC_FOUNDRY_HARDWARE_WRITE=yes`; see [Testing](docs/testing.md), the
[phase 02 note](docs/notes/02-pn532-uart.md), and the
[phase 04 note](docs/notes/04-classic-workflows.md) for the records.

A Xiaomi wallet full MIFARE Classic 1K emulation was read 64/64 and a data
block was written, verified, and restored on 2026-11-12; the phone dump
matched the source card byte for byte. The phone UID is copy-only; an external
reader cannot write it. See
[the phone note](docs/notes/26-phone-emulation.md).

ACR122U selection is an exact bus/address connstring:

```sh
cargo run -p nfctl -- list
cargo run -p nfctl -- --device acr122_usb:001:017 scan
```

Recovery is read-only and verifies every returned key:

```sh
cargo run -p nfctl -- \
  --device acr122_usb:001:017 \
  --key D3F7D3F7D3F7 \
  recover nested --known-block 4 --target-block 0 --target-type A
```

`recover auto` is a full autopwn over every sector and needs no block inputs:
it scans the key store and common keys, seeds with an EV1 signature key or
host-driven darkside, then runs nested with hardnested fallback and
read-verifies each recovered sector. `recover staticnested` uses the
static-nonce path, `recover hardnested` loads the PM3 tables and runs the
bit-sliced solver with `--max-nonces` and throttled progress, and `recover
darkside` reports a no-sample failure when the card never repeats a tag nonce
across eight darkside probes.

Hardnested tables are read from `/usr/share/nfc-foundry/hardnested_tables`,
the checkout's `resources/hardnested_tables`, or
`NFC_FOUNDRY_HARDNESTED_TABLES` (also `--hardnested-tables`).

`pcscd` holds the same CCID interface, so stop it before using the direct USB
path and restart it afterwards. The ACR122U was validated for firmware
identification, Classic 1K read, Type 2 read, raw 7-bit REQA, and a same-value
Gen1A manufacturer-block write; see the
[phase 05 note](docs/notes/05-acr122-usb.md).

When `pcscd` owns the reader, use the exact PC/SC reader name instead:

```sh
cargo run -p nfctl -- list
cargo run -p nfctl -- \
  --device 'acr122_pcsc:ACS ACR122U 00 00' \
  scan
```

PC/SC enumeration lists only ACR122U readers. `pcscd` starts on the first
PC/SC client when its socket is enabled; `nfctl diag` reports whether the
service and the reader are reachable. See the
[phase 28 note](docs/notes/28-acr122u-pcsc.md).

Type 2 and NDEF commands share the same device selection:

```sh
cargo run -p nfctl -- --device acr122_usb:001:017 type2 info
cargo run -p nfctl -- --device acr122_usb:001:017 type2 read --page 4
cargo run -p nfctl -- --device acr122_usb:001:017 ndef write --text "hello"
cargo run -p nfctl -- --device acr122_usb:001:017 ndef read
cargo run -p nfctl -- --device acr122_usb:001:017 \
  type2 lock --pages 4 --yes
cargo run -p nfctl -- --device acr122_usb:001:017 \
  --type2-password 11223344 --type2-pack AABB \
  ndef write --text "hello"
```

Type 2 reads are hardware-validated on both readers. The PN532 UART path also
passed an authorized NTAG215 NDEF write/read/restore round trip with backups
and read-back verification. A protected tag uses `--type2-password` and
`--type2-pack`, or the equivalent environment variables, for reads and writes;
the GUI uses the masked Protected access fields. The ACR122U write record and
one-way locks still need the hardware record described in
[Testing](docs/testing.md); authenticated access is validated on PN532 UART
with an NTAG216.

Classic NDEF commands use the same readers:

```sh
cargo run -p nfctl -- --device pn532_uart:/dev/ttyUSB0 classic ndef info
cargo run -p nfctl -- --device pn532_uart:/dev/ttyUSB0 classic ndef write --text hello
cargo run -p nfctl -- --device pn532_uart:/dev/ttyUSB0 \
  classic ndef format --yes
```

Formatting replaces sector keys and access bits on every NFC sector. On
2026-11-10 the PN532 UART path formatted a blank Classic 1K (UID `6A 31 5E 28`),
wrote and read back a Text record, restored the original dump with zero
failures, and matched the pre-test dump byte-for-byte. The ACR122U record is
still pending because no ACR122U was connected.

## GUI

The GTK4 workbench lives in `crates/gui`. It includes the shell, real reader
selection with live connection state, capabilities, and explicit
Connect/Disconnect, Classic
read/table/inspector, a Details tab with the shared card health model, verified
block write, dump save/load
with format selection, Keys, Key scan, Recovery, Diff, Restore, a family-aware
NDEF message workbench with multi-record editing for Text, URI, MIME, Wi-Fi,
vCard, Bluetooth LE, DPP, Smart Poster, and raw records, `.ndef` load/save, and
verified write/format, Type 2 Memory/Protection, and
Diagnostics/Activity/Settings screens. A Type 2 read shows page roles, locks,
CC, configuration, and NDEF state; user pages and Advanced pages use the
service write path, and locks/password/restore/format require the destructive confirmation
dialog. Writes, restores, and NDEF mutations pass the loaded image's card
identity to the service and are refused with `CardChanged` before any backup if
the live card differs. Key scan authenticates candidates per sector and merges
only on request. Recovery exposes every native strategy with acquisition,
filtering, and brute-force progress, elapsed time, cancellation, verified-key
saving, and the engine limitations verbatim. Activity is a session-only record
that omits key material and NDEF payloads. Every long operation also appears in
the Activity task card with elapsed time, optional numeric progress, and
`Cancel task`; cancellation stops before the next reader exchange, and a write
already issued still runs its read-back. `Disconnect` keeps the inspected image
as `Inspection only` and disables writes until a new read. Settings persist the
key store, backup directory, hardnested tables, default-key group, and default
dump format in `$XDG_CONFIG_HOME/nfc-foundry/ui.json` with atomic `0600`
writes.
Type 4 read stays out of the UI; the PN532 hardware record exists as of
2026-11-12. The `zh-CN` catalog and packaging remain in progress. The
application icon and desktop entry are in `resources/` for the packaged
application.

## Documentation

- [Architecture](docs/architecture.md)
- [Plan](docs/plan/README.md)
- [CLI contract](docs/cli.md)
- [Hardware](docs/hardware.md)
- [Recovery](docs/recovery.md)
- [NDEF](docs/ndef.md)
- [Testing](docs/testing.md)
- [Hardware matrix](docs/hardware-matrix.md)
- [Packaging](docs/packaging.md)
- [ADRs](docs/adr/)

## Safety

Destructive operations require a backup, an explicit confirmation, and
read-back verification. `nfctl` asks `Continue? [Y/n]` and accepts `y` or
`yes`; Enter also confirms, and `n` or `no` cancels. Pass `--yes` for
non-interactive use. NDEF lock, configuration, password, and capability
writes are Advanced operations.
