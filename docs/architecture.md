# Architecture

## Shape

NFC Foundry is a Rust workspace with two first-class business domains over one
transport and one application service layer.

```text
        nfctl
          |
      nfc-app
   /     |      \
Classic  Recovery  NDEF
   |       |        |
 nfc-classic  nfc-recover  nfc-ndef
   \       |        /
       nfc-transport
            |
        nfc-drivers
     /       |        \
pn532_uart  acr122_usb  acr122_pcsc
                \        /
                 acr122 (shared protocol)
```

Dependencies point downward. A lower crate never depends on a GUI feature, the
application shell, or a sibling feature.

## Crates

| Crate | Responsibility | Hardware |
|---|---|---|
| `nfc-core` | Card identity, card type, shared errors, ISO/IEC 14443A constants | no |
| `nfc-transport` | `Reader`, `Transport`, raw and capability traits, device facts | no |
| `nfc-drivers` | PN532 UART, ACR122U USB/CCID, and ACR122U PC/SC mechanics | yes |
| `nfc-classic` | Classic geometry, BCC, access bits, value blocks, dump, diff, exchange | no |
| `nfc-recover` | Crypto1, darkside, nested, staticnested, hardnested, FM11RF08S, auto | via transport |
| `nfc-ndef` | NDEF message/record, TLV, templates, validation | no |
| `nfc-type2` | Type 2 / NTAG pages, CC, lock/config, NDEF TLV adapter | via transport |
| `nfc-classic-ndef` | MAD1/MAD2, GPB mapping/access, NDEF sectors, format | no |
| `nfc-type4` | ISO-DEP/APDU CC and NDEF file read/write | via transport |
| `nfc-app` | Device manager, exclusive operations, Classic, Type 2, and Type 4 workflows, key store, backups | via transport |
| `nfctl` | English-only CLI and JSON contract | via `nfc-app` |

## Transport

The transport interface is card-family neutral:

```rust
trait Reader: Send {
    fn device(&self) -> &DeviceInfo;
    fn open(&mut self, conn_string: &str) -> Result<()>;
    fn close(&mut self) -> Result<()>;
    fn card_info(&mut self) -> Result<CardInfo>;
    fn authenticate(&mut self, block: u8, key_type: KeyType, key: Key) -> Result<()>;
    fn read_block(&mut self, block: u8) -> Result<[u8; 16]>;
    fn write_block(&mut self, block: u8, data: [u8; 16]) -> Result<()>;
    fn transceive_bytes(&mut self, tx: &[u8]) -> Result<Vec<u8>>;
    fn transceive_bits(&mut self, tx: &[u8], tx_bits: usize) -> Result<(Vec<u8>, usize)>;
    fn set_raw_mode(&mut self, enabled: bool) -> Result<()>;

    // Type 2 primitives. Drivers manage CRC-A and raw mode internally.
    fn read_page_quad(&mut self, start_page: u8) -> Result<[u8; 16]>;
    fn write_page(&mut self, page: u8, data: [u8; 4]) -> Result<()>;
    fn password_auth(&mut self, password: [u8; 4]) -> Result<[u8; 2]>;

    // ISO-DEP primitives. State-changing APDUs are never retried by a driver.
    fn apdu_capabilities(&self) -> ApduCapabilities;
    fn transceive_apdu(&mut self, apdu: &[u8]) -> Result<ApduResponse>;
}
```

Trailer and manufacturer writes are separate capability traits. Card families,
NDEF, and attack engines stay above this boundary. No OS handle, native
pointer, or driver-specific type crosses it.

Drivers are reached only through the factory trait:

```rust
trait Transport: Send + Sync {
    fn enumerate(&self) -> Result<Vec<DeviceInfo>>;
    fn open(&self, conn_string: &str) -> Result<Box<dyn Reader>>;
}
```

`DriverRegistry` in `nfc-drivers` implements it for the supported connstrings.
`nfc-app` receives an `Arc<dyn Transport>` and never names a driver type.
Trait objects are also what keeps the boundary object-safe and testable.

`Transport::open` only constructs a reader. `DeviceManager` then calls
`Reader::open` with the same exact connstring; that step performs the hardware
I/O and can fail with `Unsupported`, `Io`, or `Timeout`. The reader stays open
across operations and is closed on release or when a selected device
disappears.

### PN532 UART

`pn532_uart` is split into four layers:

- `frame`: normal and extended PN532 frames, ACK/NACK/error frames, length and
  data checksums, and resynchronization after junk.
- `link`: a small byte-transport trait, with `serialport` at 115200 8N1 for
  hardware and a scripted link for tests.
- `protocol`: wake-up, firmware validation, register setup, card selection,
  MIFARE authentication and block access through `InDataExchange`, and raw
  byte/bit exchange through `InCommunicateThru`.
- reader: exact-connstring open/close and the `Reader`/`TrailerWriter`
  implementations.

Every host command carries the HSU wake-up preamble, so the driver works after
the PN532 power-down state and while it is already awake. Card selection stores
the selected `CardInfo`; MIFARE authentication appends the last four UID bytes
after the key, as `InDataExchange` requires. `cycle_rf_field` powers the field
off and on and clears the cached selection so raw frames can start from an
IDLE card.

Raw mode disables hardware CRC and rejects ordinary MIFARE operations; ordinary
mode rejects raw transceive. The PN532 still generates and checks parity for raw
bit frames, so the transport trait carries bits and a received bit count rather
than caller-supplied parity. Repeated selection cycles the RF field first: a
card that is already ACTIVE does not answer a second `InListPassiveTarget`.
Enumeration lists serial ports without opening them; only an exact open
validates that the device is a PN532.

### ACR122U

The ACR122U has one shared command layer and two links:

- `acr122`: `AcrLink`, the ACR122 pseudo-APDU (`FF 00 00 00 Lc D4 ...`),
  firmware validation, initiator and RF field setup, card selection, Classic
  block access, raw bytes/bits, the Gen1A sequence, and the shared
  `Acr122Reader` state behind both wrappers.
- `acr122_usb`: `nusb` enumeration and interface claim, CCID message framing,
  ICC power-on, `PC_to_RDR_XfrBlock`, timeouts, stale bulk-IN draining, and the
  exact `acr122_usb:<bus>:<address>` connstring.
- `acr122_pcsc`: PC/SC context and exact reader lookup, `SCARD_SHARE_DIRECT`
  with the CCID escape control (`SCARD_CTL_CODE(3500)`) for raw PN532 frames,
  a `SCARD_SHARE_SHARED` T=1 fallback, and the exact
  `acr122_pcsc:<reader name>` connstring.

The ACR122U carries a PN532 behind its own firmware, so the shared command
layer reuses the `pn53x` command values and response parsing. Responses arrive
as `D5 <command+1>` plus an ISO 7816 status word; `61xx` is continued with
GetResponse, and `6300` maps to the missing-card error. The ACR122U vendor CCID
status `0x81` is not an error on data blocks.

Initialization puts the internal PN532 in initiator mode and explicitly cycles
the RF field on, because the driver turns it off on close. Repeated selection
releases the current target first. Raw frames require an RF field cycle rather
than `InRelease` alone to return the card to IDLE. The Gen1A path reselects the
card before entering raw mode so no Crypto1 authentication state encrypts the
backdoor frames.

The PC/SC link accepts only reader names containing `ACR122`. Opening in
direct mode leaves card activation to the driver's own `InListPassiveTarget`,
so card state is identical to the direct USB path; a firmware that rejects
direct mode falls back to T=1 transmit and reports the direct failure in the
error if the fallback also fails. `pcscd` claims the same CCID interface, so
the direct USB path and the PC/SC transport cannot be used at once and never
fall back to each other. A busy interface is reported with the instruction to
stop `pcscd`; PC/SC contention names the other client. `pcscd` being absent
yields no PC/SC devices in enumeration, and `nfctl diag` reports the service
state without failing the direct USB list.

## Device ownership

One reader is held by one operation at a time. `nfc-app` owns:

- the device manager and device list;
- the cross-process reader lock;
- an exclusive reader lease for operations;
- cancellation and task events;
- pre-write backups and post-write verification.

`DeviceManager` opens the selected reader on first use and returns an
`Operation` guard that borrows it for the duration of the operation. The guard
checks a `CancelToken` before card I/O, reads every write back before reporting
success, and exposes an explicit identity check. Reselecting a card resets
Classic authentication, so workflows call the identity check at sector or
operation boundaries rather than between an authentication and its write. The
GUI, CLI, card polling, and recovery use the same manager. Recovery does not
hand the reader to another process.

Classic workflows add key selection, pre-mutation backups, access checks, and
read-back verification on top of the same lease. Cross-process locking and
physical-reader identity remain future work; the in-process lease is exclusive
by borrow and is the current boundary.

## Classic core

`nfc-classic` is the hardware-free Classic model. It owns:

- 1K/4K geometry: sectors, blocks, trailers, and access groups, including the
  sixteen-block large 4K sectors;
- manufacturer-block BCC validation for known four-byte UIDs;
- access-bit complement decoding, data and trailer permissions, and value
  blocks;
- `Dump` with a per-byte known mask and per-block status, so unknown bytes are
  never confused with known zero bytes;
- readability counts, trailer validation, and block diff with byte and trailer
  detail;
- dump interchange for project JSON, raw images, MCT text, and Proxmark3
  `mfc v2`/`mfc v3` JSON;
- `DumpFormat::Auto`, which exports a complete image as Proxmark3 raw and a
  partial image as the project format, and detects either on load.

The project format is version 2 and is the only format that represents partial
images, masked bytes, malformed trailers, sector keys, device identity, and read
timestamps. Version 1 projects still load with empty metadata. Raw, MCT, and
Proxmark3 exports require a complete image with valid trailers; they are
image-only and drop key metadata. BCC is checked when the card identity is
known. Imports reject duplicate JSON keys, missing blocks, inconsistent
addresses, broken trailer complements, invalid key metadata, and Proxmark3
SectorKeys that disagree with trailer bytes. Default interchange names are
`card-dump.bin`, `card-dump.mfc.json`, `card-dump.mct`, and
`card-dump.nfc-foundry.json`. The exchange layer works on bytes; the workflow
layer owns file paths, sidecars, and backup policy.

`nfc-app::classic` composes the model with the transport:

- `KeyStore` stores sector-specific and wildcard A/B/any keys with provenance,
  card-scoped verification records, and import reports. It persists JSON
  version 2 (version 1 loads), imports Proxmark3 `.dic` text without aborting on
  a malformed line, and restores dump key metadata. `.dic` export writes unique
  uppercase keys and drops sector, key type, provenance, and verification;
  imported `.dic` keys apply to every sector as `any`.
- A full read authenticates each sector with the first accepted candidate and
  reads block 0 with sector 0. If sector 0 does not authenticate, block 0 stays
  unknown. Authentication and read failures remain visible per block; unknown
  bytes are never reported as known. Every sector with at least one read block
  contributes an authenticated key to the result and to dump metadata; the CLI
  and GUI record it in the card-scoped key store.
- A block write checks the expected card identity, the current access bits,
  selects an allowed key type, saves a project-format backup, writes, reads
  back, and rechecks card identity. A mismatched identity returns
  `CardChanged` before the backup. Block 0 is rejected by this path.
- A standalone UID write requires Advanced mode, a four-byte UID Classic 1K,
  and a one-shot preflight bound to the four sector-0 blocks and the card
  identity. It backs up sector 0, orders the Gen1A protected writer (ACR122U
  and PN532 UART share one reader-neutral sequence) before an ordinary
  CUID/Gen2 block-0 write, reselects after every attempt, and verifies
  the complete manufacturer block, BCC, and unchanged sector trailer. An
  ordinary immutable card refuses the write without changing data. Restore and
  the ordinary block path still reject block 0.
- A restore requires confirmation, verifies layout and known identity, refuses
  to change block 0, requires every pending block to exist in the backup,
  writes data blocks before the sector trailer, and stops at the first failure
  unless the caller explicitly opts into continuing. The source dump identity
  and an optional expected card are checked before the backup. The result
  carries one outcome per block: written, unchanged, unknown, or failed.
- A key scan authenticates candidates as A and B per sector and verifies each
  success with a trailer read. It never writes.
- Backups are written atomically to the XDG data directory (or an injected
  directory) before the first mutation.

## Classic and NDEF boundary

Classic block geometry and NDEF are separate models:

- `nfc-classic` knows sectors, blocks, trailers, access bits, dumps, and dump
  interchange.
- `nfc-ndef` knows messages, records, TLVs, and templates; it has no card
  knowledge.
- `nfc-type2` maps NDEF TLVs onto Type 2 pages and CC/locks.
- `nfc-classic-ndef` maps NDEF TLVs onto MAD1/MAD2, NFC sectors, and the
  per-sector GPB mapping version/access conditions. Classic has no capability
  container; the GPB is its capability descriptor.
- `nfc-app` composes family-specific workflows from these crates.

This keeps NDEF testable without a reader and prevents a Classic page editor
from becoming the NDEF implementation.

`nfc-ndef` implements that boundary:

- `NdefRecord` and `NdefMessage` enforce TNF field rules, 255-octet TYPE/ID
  limits, short and normal payload lengths, and the MB/ME/CF/SR/IL flags.
  Chunked payloads are reassembled into logical records; `encode_chunked`
  lets the caller choose chunk sizes.
- `Tlv` and `TlvStream` frame NULL, Lock Control, Memory Control, NDEF Message,
  Proprietary, reserved unknown, and Terminator TLVs with short and
  three-octet lengths. Unknown and proprietary values and post-terminator
  padding survive a parse/encode round trip.
- `ndef_capacity` computes the largest NDEF message that fits with its TLV
  header and Terminator.
- `template` builds and parses Text, URI, Smart Poster, vCard, Wi-Fi WSC,
  Bluetooth LE OOB, and DPP records. Unknown WSC attributes and BLE AD
  structures are preserved.
- Decoding is strict and reports byte offsets. Reserved TNF values, reserved
  URI identifier codes, bad chunk order, missing MB/ME, and truncated fields
  are errors rather than silent repairs. No card or transport type enters the
  crate.

## Type 2 / NTAG

`nfc-type2` is the hardware-free Type 2 model plus reader-backed workflows. It
owns:

- CC validation: `E1` magic, version nibbles, `CC[2] * 8` data-area size, and
  read/write access.
- Product layouts for Ultralight, NTAG213/215/216, and generic CC-sized tags;
  page roles; user-memory ranges; static and dynamic lock positions; and
  configuration page addresses. NTAG215's 504-byte user area and 496-byte
  NDEF data area stay distinct.
- `Type2Image` with explicit unknown pages, page iteration, NDEF-area
  extraction, and project JSON that preserves partial images.
- Static/dynamic lock bits and pending one-way updates. Dynamic lock byte 3 is
  RFUI and read-back compares only the first three bytes.
- NTAG configuration (`MIRROR`, `AUTH0`, `ACCESS`, `CFGLCK`, `AUTHLIM`,
  PWD/PACK). PWD and PACK always read as zero and are verified with PWD_AUTH.
- `Type2Auth` and `Type2AuthSession`: per-operation PWD_AUTH with a mandatory
  expected PACK. Reading a protected tag authenticates once after the
  configuration read; writes, NDEF operations, locks, restore, and password
  changes reuse that session. PWD/PACK pages are masked in images, dumps, and
  backups, and the password is redacted from `Debug`.
- NDEF state and mapping: unformatted, empty, and message; in-place NDEF TLV
  replacement; unknown/proprietary TLV preservation; zero padding and
  Terminator handling; exact capacity.
- Verified page/NDEF/format/restore/lock/password functions. Restore rejects a
  different layout or CC and writes user memory before configuration, the CC,
  and lock bytes. PWD/PACK are never restored from a dump.

`nfc-app::type2` adds the service policy: backups before mutation, user-memory
defaults, Advanced confirmations for lock/config/CC/password, NDEF format and
restore confirmation, read-back verification, and card identity checks.
Protected reads and writes take an optional `Type2Auth`; password changes take
the current credentials separately, write PWD/PACK before AUTH0/ACCESS, and
verify the new PACK before reading the configuration back. The CLI and GUI call
those same services.

Both drivers implement the Type 2 READ, WRITE, and PWD_AUTH primitives. They
enter raw framing, append CRC-A, verify the response CRC when present, and
restore the previous raw mode. The 16-byte READ always covers four pages;
`quad_starts` adds an overlapping final read for products whose page count is
not a multiple of four.

## Type 4 / ISO-DEP

`nfc-type4` owns the NFC Forum Type 4 mapping over ISO 7816-4 APDUs. It has no
reader mechanics:

- `apdu` builds SELECT by DF name, SELECT FILE, READ BINARY, and UPDATE
  BINARY, validates short-file offsets and lengths, and maps status words.
- `cc` parses mapping version 2.0, CCLEN, MLe/MLc, and the TLV list. It
  requires the first TLV to be an NDEF File Control TLV, preserves unknown and
  Proprietary File Control TLVs, and exposes read/write access and capacity.
- `file` decodes NLEN and the NDEF message with a capacity check.
- `read` selects the NDEF Tag Application, reads the CC in MLe-sized chunks,
  selects the NDEF file, and reads NLEN and the message.
- `write` refuses read-only and proprietary access before I/O, writes NLEN
  zero, writes the message in MLc and transport chunks, writes the final NLEN,
  and verifies the file by read-back. `clear` writes an empty NDEF file.
- `exchange` exports raw CC and canonical NLEN+message bytes as project JSON.

Both drivers expose `transceive_apdu`. The PN532 UART path uses
`InDataExchange` after auto-RATS; the ACR122U path uses the `D4 40 01`
pseudo-APDU. Each driver advertises a 247-byte command limit, set by the
ACR122U wrapper, and never retries a state-changing APDU.

`nfc-app::type4` adds a pre-mutation backup, read-only refusal, confirmation
for clear, read-back verification, and a card identity check. The phase does
not add a CLI or GUI command; the service is the integration point for a later
surface.

## Recovery

`nfc-recover` is a library, not a process wrapper. It owns:

- Crypto1/LFSR/PRNG, CRC-A, common-prefix recovery, and the `mfkey` solvers
  ported from the current Proxmark3 tree;
- software Crypto1 authentication and nested nonce acquisition;
- darkside, nested, static-nested, hardnested, and `auto` execution;
- hardnested filtering, the 64-lane bit-sliced solver, and PM3 table loading;
- EV1/QL88 signature detection, static encrypted nonce detection, and
  FM11RF08S backdoor key recovery;
- progress, cancellation, and result types.

The attack engine receives an exclusive reader lease from `nfc-app`, not a raw
device handle. It must reselect and verify the card before returning success.

Nonce acquisition needs caller-supplied parity, so `nfc-transport::Reader`
exposes `set_recovery_mode`, `transceive_parity`, and `transceive_parity_bits`
as opt-in capabilities. The PN532 UART and ACR122U drivers implement
parity-disabled framing through `CIU_ManualRCV` and 9-bit frame packing.
`transceive_parity_bits` carries 7-bit short frames such as the WUPA that
precedes a hardnested fast SELECT. Recovery mode is mutually exclusive with
ordinary and raw byte modes. No PM3 firmware command is ported.

Both transports complete software Crypto1 authentication, nested recovery, and
hardnested acquisition with a persistent fast-selected session. A known
response size skips the post-response trailing-bits reset and status read,
removing two round trips from the authentication window. Darkside needs
PRNG-scheduled frames that neither host-driven reader can produce, so it
reports an honest no-sample failure while the solver remains covered by the PM3
golden vector.

## GUI

The GUI stack is **GTK4-rs** (`gtk4` + `libadwaita`), decided in
[ADR 0006](adr/0006-gtk4-rs.md); ADR 0005's GPUI Kit shell was removed and is
not a fallback. The single `nfc-foundry` binary in `crates/gui` calls the same
`nfc-app` services and is the only desktop frontend.

The workbench has a header with the primary `Read card` action, a workspace
selector (`MIFARE Classic` / `NFC Forum Type 2`), a `☰` main menu, and a back
button for the hidden System pages. The Classic workspace exposes Blocks,
NDEF, Keys, Recovery, and Dumps; the Type 2 workspace exposes Pages,
Protection, NDEF, and Dumps. Diagnostics, Activity, Settings, and About live in
the header menu. GUI view code stays presentation-only: card operations run on
a worker thread and report through `async-channel` to `glib` on the main
context. UI tests run under Xvfb and are gated by `NFC_FOUNDRY_GUI_TESTS=1`.

## Errors and cancellation

- `nfc-core::NfcError` is the shared error type across hardware and domain
  boundaries.
- CLI maps errors to stable exit codes; JSON output carries a stable code and
  message.
- `nfc-core::CancelToken` is a shared atomic flag checked before reader I/O and
  between write verification steps. Long operations surface
  `NfcError::Cancelled`; hardware timeouts are explicit and bounded.
