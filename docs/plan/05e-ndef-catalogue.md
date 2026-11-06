# Phase 05e: NDEF record catalogue and decoded views

## Goal

Replace the two-template NDEF editor and raw-hex read view with a record
catalogue and human-readable decoded views.

Status: done (2026-11-07). See `docs/notes/05e-ndef-catalogue.md`.

## Record and template support

Records:

- Text, URI, Smart Poster;
- MIME `text/plain`, `text/vcard`, `text/x-vcard`, `application/json`;
- MIME `application/vnd.wfa.wsc` (Wi-Fi WSC credential);
- MIME `application/vnd.bluetooth.ep.oob` (Bluetooth BR/EDR easy pairing);
- External `android.com:pkg` (Android Application Record);
- unknown MIME/external records with a printable-payload fallback.

Templates:

- Text
- URL/URI
- Custom URI
- Search (provider URL prefix + query)
- Payment link
- Social profile URL
- Application package
- Email (`mailto:` with subject/body)
- Contact (vCard 3.0 MIME)
- Phone (`tel:`)
- SMS (`sms:` with optional body)
- Location (`geo:` latitude/longitude/optional label)
- Address (`geo:0,0?q=`)
- Bluetooth easy pairing (address + optional name)
- Wi-Fi WSC (SSID + security + password)
- Smart Poster (URI + title)

The search template accepts a provider URL prefix, not a single hard-coded
search engine. The Wi-Fi security choices are Open, WPA-Personal, WPA2-Personal,
and WPA/WPA2 mixed.

## Decoded read view

- Show record kind, summary, and decoded fields.
- vCard: name, organization, title, phone, email, address, URL, note.
- Wi-Fi: SSID, authentication, encryption, MAC, and a masked network key.
- Bluetooth: device address, name, class when present.
- Application: package name.
- Smart Poster: title and URI.
- Text/URI: decoded content.
- Raw payload and message hex are hidden by default behind a Show raw toggle.
- Unknown records show a printable preview and their raw payload.

## UI

- Replace the Text/URI buttons with a template selector.
- Render only the fields for the selected template.
- Keep confirmation, backup, read-back, and password masking.
- Add English and Simplified Chinese labels for templates, fields, record
  kinds, and security choices.

## Acceptance

- A vCard tag that previously showed `42 45 49 ...` shows contact fields.
- A Wi-Fi tag shows SSID/security/MAC and masks the network key.
- A Bluetooth tag shows the device address and name.
- A Smart Poster shows its title and URI.
- The write editor can create every template listed above.
- Raw hex is opt-in.
- `make check` passes.

## Non-goals

- No arbitrary NDEF record editor, chunked-record editor, or signature records.
- No NFC Forum Connection Handover negotiation; only static Wi-Fi/Bluetooth
  records.
- No WPA3/WSC 3.0 claims, because the implemented WSC credential attributes are
  the validated 2.0 subset.
- No protected Type 2 writes; those remain Phase 05d.
