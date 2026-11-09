# Architecture

## Shape

NFC Foundry is a Rust workspace with two first-class business domains over one
transport and one application service layer.

```text
GPUI Kit GUI / nfctl
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
       /           \
 pn532_uart     acr122_usb
```

Dependencies point downward. A lower crate never depends on a GUI feature, the
application shell, or a sibling feature.

## Crates

| Crate | Responsibility | Hardware |
|---|---|---|
| `nfc-core` | Card identity, card type, shared errors, ISO/IEC 14443A constants | no |
| `nfc-transport` | `Reader`, `Transport`, raw and capability traits, device facts | no |
| `nfc-drivers` | PN532 UART and ACR122U USB/CCID mechanics | yes |
| `nfc-classic` | Classic geometry, BCC, access bits, value blocks, dump, diff, exchange | no |
| `nfc-recover` | Crypto1, darkside, nested, staticnested, hardnested, FM11RF08S, auto | via transport |
| `nfc-ndef` | NDEF message/record, TLV, templates, validation | no |
| `nfc-type2` | Type 2 / NTAG pages, CC, lock/config, NDEF TLV adapter | via transport |
| `nfc-classic-ndef` | MAD1/MAD2, Classic CC, NDEF sectors, format | via transport |
| `nfc-type4` | ISO-DEP/APDU NDEF adapter (later) | via transport |
| `nfc-app` | Device manager, exclusive operations, Classic workflows, key store, backups | via transport |
| `nfctl` | English-only CLI and JSON contract | via `nfc-app` |
| `gui/nfc-foundry` | GPUI Kit shell | via `nfc-app` |
| `gui/classic` | GPUI Kit Classic workspace (view, actions, UI tests) | via `nfc-app` |

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

### ACR122U USB

`acr122_usb` is split into three layers:

- `ccid`: `nusb` enumeration and interface claim, CCID message framing,
  ICC power-on, `PC_to_RDR_XfrBlock`, timeouts, and stale bulk-IN draining.
- `protocol`: the ACR122 pseudo-APDU (`FF 00 00 00 Lc D4 ...`), firmware
  validation, initiator and RF field setup, card selection, Classic block
  access, raw bytes/bits, and the Gen1A sequence.
- reader: exact `acr122_usb:<bus>:<address>` open/close and the
  `Reader`/`ManufacturerWriter` implementations.

The ACR122U carries a PN532 behind its own firmware, so the command layer
reuses the shared `pn53x` command values and response parsing. Responses arrive
as `D5 <command+1>` plus an ISO 7816 status word; `61xx` is continued with
GetResponse, and `6300` maps to the missing-card error. The ACR122U vendor CCID
status `0x81` is not an error on data blocks.

Initialization puts the internal PN532 in initiator mode and explicitly cycles
the RF field on, because the driver turns it off on close. Repeated selection
releases the current target first. Raw frames require an RF field cycle rather
than `InRelease` alone to return the card to IDLE. The Gen1A path reselects the
card before entering raw mode so no Crypto1 authentication state encrypts the
backdoor frames.

`pcscd` claims the same CCID interface through libusb, so the direct USB path
and the PC/SC alias cannot be used at once. A busy interface is reported with
the instruction to stop `pcscd`.

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
  `mfc v2`/`mfc v3` JSON.

The project format is the only format that represents partial images, masked
bytes, and malformed trailers. Raw, MCT, and Proxmark3 exports require a
complete image with valid trailers; BCC is checked when the card identity is
known. Imports reject duplicate JSON keys, missing blocks, inconsistent
addresses, broken trailer complements, and Proxmark3 SectorKeys that disagree
with trailer bytes. The exchange layer works on bytes; the workflow layer owns
file paths, sidecars, and backup policy.

`nfc-app::classic` composes the model with the transport:

- `KeyStore` stores sector-specific and wildcard A/B/any keys with provenance,
  persists JSON, and imports text key files.
- A full read authenticates each sector with the first accepted candidate and
  reads block 0 with sector 0. If sector 0 does not authenticate, block 0 stays
  unknown. Authentication and read failures remain visible per block; unknown
  bytes are never reported as known.
- A block write checks the current access bits, selects an allowed key type,
  saves a project-format backup, writes, reads back, and rechecks card
  identity. Block 0 is rejected.
- A restore requires confirmation, verifies layout and known identity, refuses
  to change block 0, requires every pending block to exist in the backup,
  writes data blocks before the sector trailer, and stops at the first failure
  unless the caller explicitly opts into continuing.
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
- `nfc-classic-ndef` maps NDEF TLVs onto MAD/NDEF sectors and access
  conditions.
- `nfc-app` composes family-specific workflows from these crates.

This keeps NDEF testable without a reader and prevents a Classic page editor
from becoming the NDEF implementation.

## Recovery

`nfc-recover` is a library, not a process wrapper. It owns:

- Crypto1/LFSR/PRNG, CRC-A, common-prefix recovery, and the `mfkey` solvers
  ported from the current Proxmark3 tree;
- software Crypto1 authentication and nested nonce acquisition;
- darkside, nested, static-nested, and `auto` execution;
- progress, cancellation, and result types.

The attack engine receives an exclusive reader lease from `nfc-app`, not a raw
device handle. It must reselect and verify the card before returning success.

Nonce acquisition needs caller-supplied parity, so `nfc-transport::Reader`
exposes `set_recovery_mode` and `transceive_parity` as opt-in capabilities.
The PN532 UART and ACR122U drivers implement parity-disabled framing through
`CIU_ManualRCV` and 9-bit frame packing. Recovery mode is mutually exclusive
with ordinary and raw byte modes. No PM3 firmware command is ported.

Both transports complete software Crypto1 authentication and nested recovery.
A known response size skips the post-response trailing-bits reset and status
read, removing two UART round trips from the authentication window. Darkside
needs PRNG-scheduled frames that neither host-driven reader can produce, so it
reports an honest no-sample failure while the solver remains covered by the PM3
golden vector.

## GUI

GPUI Kit is the only GUI dependency. The application shell composes feature
crates:

```text
gui/nfc-foundry     shell and reader wiring
gui/classic         Classic workspace and UI integration tests
```

The Classic workspace is one window with a reader header, card/file toolbar,
Blocks/Diff/Keys tabs, a block table, and a block inspector:

- `RenderOnce` for value-like presentation.
- `Entity<T>` for input, tables, scrolling, focus, and async state.
- No domain mutation or hardware I/O in `render`.
- Card operations run on a background executor through the shared `nfc-app`
  service; the view updates state through its entity context.
- Theme tokens only; no raw colors or spacing literals in application code.
- English is the current interface language. The application-wide localization
  pass remains a release requirement.

## Errors and cancellation

- `nfc-core::NfcError` is the shared error type across hardware and domain
  boundaries.
- CLI maps errors to stable exit codes; JSON output carries a stable code and
  message.
- `nfc-core::CancelToken` is a shared atomic flag checked before reader I/O and
  between write verification steps. Long operations surface
  `NfcError::Cancelled`; hardware timeouts are explicit and bounded.
