# Note: phase 26.2 DESFire file settings decode

Status: complete 2026-11-12. Plan:
[26-desfire-workbench.md](../plan/26-desfire-workbench.md). Spec:
[26b-file-settings.md](../specs/26b-file-settings.md).

## Implementation

- `nfc-desfire` now exposes `FileType`, `CommunicationMode`, `AccessRight`,
  `AccessRights`, `FileContents`, and `FileSettings`. `FileSettings::parse`
  validates the response and preserves the raw bytes; `FileInfo::decoded`
  decodes one inventory entry.
- `nfc-app::desfire::inventory_text` is the single inventory renderer for the
  CLI and GUI. It prints the version line, application IDs, decoded file
  summaries, and raw settings. A parse failure is shown as `undecoded` with
  the error; the raw bytes are still printed.
- `nfctl desfire info` and the GTK `Inventory` tab use that renderer. The GUI
  never assigns a file type to a response that did not parse.

## Layout evidence

The decode follows the documented `GetFileSettings` response:

- Standard/Backup: 3-byte little-endian size.
- Value: signed 32-bit little-endian lower limit, upper limit, limited-credit
  value, then a flags byte (bit 0 limited credit, bit 1 free GetValue). The
  third field is not the current value; phase 26.3 reads the current value
  with `GetValue`. A card created with value 7 reported a limited-credit value
  of 0 while `GetValue` returned 7. Corrected 2026-11-13: the field is named
  `limited_credit_value` and is not bounded by the value limits.
- Linear/Cyclic Record: 3-byte little-endian record size, maximum count, and
  current count.
- Transaction MAC: key option, key version, then an optional counter limit.
  The header bit `0x20` enables the counter limit; key option bit `0x02`
  selects four bytes, otherwise two. The counter limit is read big-endian.
  The Flipper Zero DESFire parser in the local `nfc-research` tree was used as
  the layout reference; PM3's parser stops after key option/version.
- Additional access rights: one count byte followed by count 16-bit words.

The Transaction MAC counter-limit endianness is not hardware-validated on an
owned card. If a future card disagrees, the raw settings remain printed and
the test vector must be corrected.

## Unit tests

`nfc-desfire` has one valid and one malformed vector per file type:

- Standard and Backup: valid vectors; truncated standard and zero-size backup
  are rejected.
- Value: valid MACed vector; equal limits, an out-of-range value, and reserved
  flags are rejected.
- Linear and Cyclic Record: valid vectors; current count above maximum and a
  zero record size are rejected.
- Transaction MAC: valid vectors with and without a counter limit; a truncated
  counter limit is rejected.
- Additional access rights: valid two-key vector; zero count and a short list
  are rejected.
- Header: unknown file type, communication mode 2, counter bit on a non-MAC
  file, and trailing bytes without the additional-rights bit are rejected.

`nfc-app` tests the shared `inventory_text`: a decodable standard file is
summarized, an unknown type is marked `undecoded`, and both raw byte strings
remain present.

## PN532 UART check

On 2026-11-12 the DESFire EV3 was formatted with the existing NDEF
provisioning path, then `nfctl desfire info` decoded the two standard files:

```text
MIFARE DESFire EV3 · vendor 04 type 01 subtype 01 version 51.0 storage 1A protocol 05
application 00 76 D2
  file 01: standard data · plain · read free, write key 0, rw key 0, change key 0 · 15 bytes
    raw: 00 00 00 E0 0F 00 00
  file 02: standard data · plain · read free, write free, rw free, change key 0 · 255 bytes
    raw: 00 00 E0 EE FF 00 00
```

The NDEF application was then deleted, leaving `applications: none`. This
validates Standard-file decoding on hardware; Value, Record, and Transaction
MAC responses still have unit vectors only. The ACR122U row is not recorded.

## Verification

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
env -u WAYLAND_DISPLAY NFC_FOUNDRY_GUI_TESTS=1 \
  xvfb-run -a env GDK_BACKEND=x11 \
  cargo test -p nfc-foundry gtk_workbench_scenarios
reuse lint
```

All passed on 2026-11-12.

## Next

Phase 26.3 is complete: it reuses `FileSettings` for the read dispatch and adds
Value/GetValue plus Linear/Cyclic Record reads. Phase 26.4 adds the remaining
authentication key families.
