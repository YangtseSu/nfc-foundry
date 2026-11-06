# Phase 05e spec: NDEF record catalogue and decoded views

Date: 2026-11-07

## Goal

Extend the Type 2 NDEF module from Text/URI to a validated record catalogue,
decode the records into human-readable fields, and replace the GUI's raw-hex
NDEF display.

## Domain model

Add to `type2.Record`:

```go
Kind    string  // text, uri, smartposter, vcard, wifi, bluetooth, app, mime, external, unknown
Summary string
Fields  []Field
```

```go
type Field struct {
    Key   string
    Value string
}
```

`NDEF.Summary` carries the first useful record summary.

## Encoders

```go
EncodeTextMessage(text, language string) ([]byte, error)
EncodeURIMessage(uri string) ([]byte, error)
EncodeSmartPosterMessage(uri, title string) ([]byte, error)
EncodeExternalMessage(recordType string, payload []byte) ([]byte, error)
EncodeVCardMessage(fields map[string]string) ([]byte, error)
EncodeWiFiMessage(ssid, password, security string) ([]byte, error)
EncodeBluetoothMessage(address, name string) ([]byte, error)
EncodeTemplate(template string, fields map[string]string) ([]byte, error)
```

The generic record encoder must support first/last flags so Smart Poster can
carry a nested URI + Text message with correct MB/ME bits.

### vCard

`text/vcard` MIME, vCard 3.0, CRLF lines. Fields: `FN`, `N`, `ORG`, `TITLE`,
`TEL;TYPE=CELL`, `EMAIL`, `ADR;TYPE=WORK`, `URL`, `NOTE`. Escape backslash,
semicolon, comma, and newline.

### Wi-Fi WSC

MIME `application/vnd.wfa.wsc`. Outer Credential TLV `0x100E` containing:

- Network Index `0x1026` = 1;
- SSID `0x1045`;
- Authentication Type `0x1003` (Open `0x0001`, WPA-Personal `0x0002`,
  WPA2-Personal `0x0020`, mixed `0x0022`);
- Encryption Type `0x100F` (None `0x0001`, WEP `0x0002`, TKIP `0x0004`,
  AES `0x0008`, mixed TKIP+AES `0x000C`);
- Network Key `0x1027` when not Open;
- MAC Address `0x1020` = broadcast `FF:FF:FF:FF:FF:FF`;
- Vendor Extension `0x1049` = `00 37 2A`, Version2 `0x00/0x01/0x20`,
  Network Key Shareable `0x02/0x01/0x01`.

All WSC integers are big-endian TLVs with 16-bit type and length.

### Bluetooth easy pairing

MIME `application/vnd.bluetooth.ep.oob`. Payload:

- 2-byte little-endian OOB length including the length field;
- 6-byte BD_ADDR in reverse textual order;
- optional EIR `0x09` complete local name, encoded as length/type/value.

### Smart Poster

Well-known type `Sp`. Payload is a nested NDEF message with a URI record first
and a Text title record last.

### Android Application Record

External type `android.com:pkg`, payload is the package name.

## Templates

`EncodeTemplate` accepts stable field maps:

| Template | Fields |
|---|---|
| `text` | `text` |
| `uri` | `uri` |
| `custom` | `uri` |
| `search` | `search_prefix`, `query` |
| `payment` | `uri` |
| `social` | `uri` |
| `app` | `package` |
| `email` | `to`, `subject`, `body` |
| `contact` | `name`, `organization`, `title`, `phone`, `email`, `address`, `url`, `note` |
| `phone` | `phone` |
| `sms` | `number`, `body` |
| `location` | `latitude`, `longitude`, `label` |
| `address` | `address` |
| `bluetooth` | `address`, `name` |
| `wifi` | `ssid`, `password`, `security` |
| `dpp` | `dpp_uri` |
| `smartposter` | `uri`, `title` |

Search appends a URL-escaped query to the provider prefix. Email uses `mailto:`
with optional `subject`/`body`. Phone uses `tel:`. Location uses `geo:`.
Address uses `geo:0,0?q=`.

## Decoding

- MIME `text/vcard` / `text/x-vcard`: parse folded vCard lines, unescape values,
  and emit fields.
- MIME `text/plain`: UTF-8 text.
- MIME `application/json`: UTF-8 JSON preview.
- MIME `application/vnd.wfa.wsc`: parse outer/inner TLVs; emit SSID,
  authentication, encryption, MAC, and a masked network key.
- MIME `application/vnd.bluetooth.ep.oob`: parse OOB length, reversed BD_ADDR,
  EIR name/class.
- External `android.com:pkg`: package name.
- Smart Poster: parse nested URI and title.
- Unknown: keep a printable preview and raw payload.

## GUI

- Template selector from `type2.Templates()` with localized labels.
- Fields rendered from the selected template; empty required fields disable the
  write button.
- Preview the encoded length through the application service before opening the
  confirmation dialog.
- Decoded record cards replace the raw payload lines.
- Raw message/payload toggle, off by default.
- Wi-Fi network keys are masked in read views; the editor still accepts a
  password.

## Tests

- Encoder round-trips for Text, URI, Smart Poster, vCard, Wi-Fi, Bluetooth, AAR.
- Template validation for every template, including missing required fields.
- vCard parsing of folded and escaped values.
- Wi-Fi/Bluetooth parser field extraction.
- GUI template selection, dynamic fields, confirmation, and decoded record view.
- CLI `ndef read` prints decoded fields; `--raw` remains the raw message mode.
- Hardware vCard read displays contact fields.

## Non-goals

- Chunked NDEF, signatures, and arbitrary MIME editors.
- Connection Handover negotiation.
- WPA3/WSC 3.0 claims.
- Protected Type 2 writes.
