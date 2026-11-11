# Note: Wi-Fi WSC records in the NDEF view

## Problem

The NDEF view decoded Text and URI records only. Every other record rendered
`String::from_utf8_lossy(payload)`, so an `application/vnd.wfa.wsc` payload
appeared as replacement glyphs (two boxes with an X), and the SSID and network
key were neither visible nor copyable.

## Implemented

- `record_summary` decodes `application/vnd.wfa.wsc` media records with the
  existing `nfc_ndef::WifiRecord` parser and shows the authentication and
  encryption labels (for example `Wi-Fi · WSC` and `WPA2-Personal · AES`).
- WSC records add `SSID` and `Password` rows. Each row is a selectable
  monospace label plus a `Copy` button that writes the raw value to the
  clipboard.
- Any other media payload is shown as hex when it is not clean UTF-8 text
  (control characters included), instead of a lossy string full of U+FFFD.
- `nfctl ndef read` prints the same decoded summary:
  `record N: wifi ssid "…" auth 0x…. encryption 0x…. key "…"`.

## Hardware verification

NTAG215, UID `04 77 CD 3B DE 2A 81`, read over
`pn532_uart:/dev/ttyUSB0` on 2026-11-11:

```
NDEF: 1 record(s), 86 bytes, capacity 491
record 0: wifi ssid "<redacted>" auth 0x0020 encryption 0x000C key "<redacted>"
```

The payload used the standard Credential TLV with a Network Index (0x1026), SSID, authentication, encryption, network-key, and MAC attributes. The real SSID and key were redacted here; the parser preserves the Network Index and MAC attributes as unknown credential attributes.

## Tests

- `cargo test -p nfc-foundry wifi_wsc_records_decode_and_binary_media_uses_hex`
  mirrors the card's WSC payload structure with test values and asserts the SSID, password,
  authentication, encryption, and the hex fallback for a binary media record.
- `cargo test -p nfc-foundry gtk_workbench_scenarios` (gated) still passes.
- `cargo test --workspace`, `cargo clippy --workspace --all-targets --
  -D warnings`, `cargo fmt --all --check`, and `reuse lint` pass.
