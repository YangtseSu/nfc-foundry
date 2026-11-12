# Note: phase 26.6 DESFire Type 4 NDEF read

Status: complete 2026-11-13. Plan:
[26-desfire-workbench.md](../plan/26-desfire-workbench.md). Spec:
[26f-type4-ndef-read.md](../specs/26f-type4-ndef-read.md).

## Implementation

- `Service::type4_read` already selected the NDEF Tag Application, read the
  CC and NDEF file, validated NLEN and record syntax, and returned the
  capability-container TLVs. This slice exposes that result; it adds no second
  NDEF codec and no write path.
- CLI: `nfctl desfire ndef-read [--output PATH]` prints mapping version,
  MLe/MLc, the NDEF file facts, every decoded record, proprietary/unknown CC
  TLVs, and the read-only reason. `--output` writes the encoded NDEF message
  only; the default is display-only.
- GUI: DESFire → NDEF → Read NDEF runs the same service and renders the CC,
  file facts, records, extra TLVs, and read-only reason in a monospace result
  panel. Read does not require the PICC key or Advanced mode; format and
  delete keep their gates.
- A missing NDEF Tag Application returns the Type 4 selection error; it is
  never shown as an unformatted tag.

## Tests

- `crates/nfctl/tests/cli.rs` adds a mock Type 4 DESFire device with a Text
  record, a proprietary File Control TLV, and an unknown TLV. It checks the
  printed fields and the raw-message output.
- `crates/gui/src/app.rs` adds a formatter test over the same CC fixture and a
  DESFire workspace check for the Read NDEF control.
- `nfc-type4` continues to cover malformed CC/TLV, NLEN, capacity, and record
  cases.

## Hardware record

2026-11-13, ACR122U `acr122_usb:001:002` (direct USB/CCID, `pcscd` stopped):

```text
$ NFC_FOUNDRY_DEVICE=acr122_usb:001:002 \
  cargo test -p nfctl --features hardware-write --test hardware_desfire_ndef -- --ignored
formatted aid=[00, 76, D2] cc=15 state=Empty
test formats_writes_restores_and_deletes_the_ndef_application ... ok
```

The CLI then ran the same card through the new surface:

```text
$ nfctl --device acr122_usb:001:002 desfire format-ndef --key <zero> --advanced --yes
NDEF application formatted
CC: 00 0F 20 00 3B 00 34 04 06 E1 04 00 FF 00 00
NDEF file: empty

$ nfctl --device acr122_usb:001:002 desfire ndef-read
CC 00 0F 20 00 3B 00 34 04 06 E1 04 00 FF 00 00 · mapping 2.0 · MLe 59 · MLc 52
NDEF file E104 · 255 bytes · read free · write free · empty

$ nfctl --device acr122_usb:001:002 desfire delete-ndef --key <zero> --advanced --yes
NDEF application D27600 deleted
applications: none
```

The PN532 UART Type 4 read path was already validated in phase 25.5; this
record adds the CLI/GUI surface and the ACR122U NDEF round trip.
