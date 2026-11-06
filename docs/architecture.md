# Architecture

NFC Foundry is split into a standalone native library, a Go application core,
and two presentation surfaces: the MyGo native GUI and the `nfctl` CLI.

```text
MyGo native GUI            nfctl CLI
        \                    /
         Go application services
         |-- device manager and lock
         |-- card-family modules
         |-- dump/restore and write safety
         `-- external engine adapters
                    |
          CGO + narrow C shim
                    |
              libnfc-ng (LGPL)
                    |
     Linux readers: pn532_uart, USB, PC/SC
```

## Components

### libnfc-ng

A standalone fork of libnfc. It owns device discovery, driver/bus handling, card
selection, raw transceive, and the libnfc-compatible API. It is a pure library
package: shared library, headers, and pkg-config only. It does not install
upstream `nfc-*` utilities.

The fork keeps upstream Windows/macOS source for reference, but NFC Foundry
builds and tests Linux only.

### C shim

A narrow CGO layer inside `internal/nfc/libnfc`. It exposes copied values and
stable error codes, never libnfc pointers. It provides:

- device enumeration and open/close;
- card selection and UID/ATQA/SAK;
- Classic authentication and block access where the high-level API is enough;
- raw `transceive_bytes` and `transceive_bits`;
- raw-mode property control for Gen1A and Type 2 CRC-A framing.

### Go application core

The core is independent of the GUI and process execution:

- `Reader`/`Transport` interfaces for device access;
- card-family modules (Classic with the in-process Gen1A sequence, Type 2/NTAG
  with NDEF, later ISO14443-4/DESFire, FeliCa);
- dump/restore validation, key catalog, key scan, recovery orchestration;
- the global device manager and lock;
- external engine adapters and result validation.

The GUI and CLI call this layer. They do not perform NFC I/O themselves.

### Presentation

- **MyGo native GUI**: English base language, complete Simplified Chinese
  translation. It renders state and invokes services.
- **nfctl CLI**: English-only, JSON-capable, suitable for scripts and CI.

## Device ownership

One process owns a reader at a time. Within the application, one global device
manager serialises the GUI, CLI, polling, and external engines. Across
processes, use a lock file keyed by the exact connstring so a CLI command and
the GUI cannot open the same reader concurrently.

External engine handoff:

1. stop polling;
2. close the in-process reader;
3. run the engine with `LIBNFC_DEVICE=<connstring>`, autoscan off, intrusive
   scan off;
4. stream output and support real cancellation;
5. validate exit status and artifacts;
6. reopen the reader;
7. verify keys and written data in-process.

Native Gen1A writing does not use this handoff. It stays in-process through the
raw shim API.

## Card-family model

Each card family is a Go module with:

- detection and capability declaration;
- a memory or command model;
- read/write preflight;
- confirmation requirements for destructive operations;
- read-back verification;
- a dump/import/export format where applicable.

Do not put card-specific semantics in libnfc-ng or the GUI.

## Future emulation boundary

Card emulation is a possible direction for mobile Linux devices. Keep the
transport interface capable of target-mode operations and raw frames, but do
not advertise or implement emulation until a separate validated design exists.
The initial GUI and CLI do not expose emulation commands.

## Dependency and license boundaries

- `nfc-foundry`: GPL-3.0-or-later.
- `libnfc-ng`: LGPL-3.0-or-later, dynamically linked.
- `mfoc`, `mfcuk`, `mfoc-hardnested`: GPL-2.0-or-later, separate processes.
- NFCX-derived code: MIT notice retained.
- MyGo: MIT.

System `libusb` and `pcsclite` are not bundled. The external engines are
bundled into the application package but are not linked into the Go binary.
