# Note: phase 26.11 DESFire key management

Date: 2026-11-14. Spec:
[26k-key-management.md](../specs/26k-key-management.md). Plan:
[26-desfire-workbench.md](../plan/26-desfire-workbench.md).

## Implemented

- `nfc-desfire` adds GetKeySettings (`0x45`), ChangeKeySettings (`0x54`), and
  ChangeKey (`0xC4`), plus `KeySettings` and `KeyChange` types.
- `Session` records the authenticated key number so ChangeKey can tell the two
  cases apart.
- ChangeKey builds the D40, EV1, or EV2 frame from the authenticated channel:
  CRC16 and D40 send direction for DES/2K3DES; CRC32 and the session IV for
  3K3DES/AES EV1; EV2 command IV, ISO/IEC 9797-1 padding, and truncated CMAC
  for AES EV2.
- Changing the authenticated key ends the session locally, matching the card's
  status-only response.
- `nfc-app` adds `desfire_key_settings` and `desfire_change_key`, with the
  recording gate, export file, re-authentication, and version read-back.
- `nfctl desfire key-settings` and `nfctl desfire change-key`, and the GTK
  `Keys` tab, call the same services with the same gates.
- The scripted card models per-key values and versions, GetKeySettings,
  ChangeKeySettings, ChangeKey in D40 and EV1 form, and the case-2 response
  without a MAC.

## Protocol facts found on hardware

- **ChangeKeySettings is enciphered, not plain.** Under EV1 AES the first
  attempt sent `54 0F` with a command MAC; the EV3 answered `0x917E`
  (LENGTH_ERROR). nfcjlib also sends ChangeKeySettings through its enciphered
  preprocessing. Sending the byte with a CRC32 (EV1) or padding and CMAC (EV2)
  succeeded.
- **ChangeKey case 2 has no response MAC.** The EV1 AES and EV2 runs both
  answered `9100` with no body after changing the authenticated key. The
  session is dropped before decoding the status; a MAC-required decode failed
  on hardware.
- **D40 ChangeKey uses the D40 send direction.** The first DES attempt used
  standard CBC encryption and the card answered `0x911E` (INTEGRITY_ERROR).
  The authentication code's `d40_send` (block decrypt, CBC XOR, zero IV) is the
  correct direction and passed.
- **Only the application master key may change keys under factory
  KeySettings.** With KeySettings `0x0F` the change-rights nibble is 0, so an
  attempt to change key 0 while authenticated as key 1 returned `0x91AE`
  (AUTHENTICATION_ERROR). The hardware test therefore changes key 1 as key 0
  (case 1), then changes key 0 as key 0 (case 2).
- The key version of a DES key is stored in the parity-bit LSBs and is
  reconstructed with `GetKeyVersion` before the old key enters the XOR.

## PN532 UART hardware record

Run on 2026-11-14 on `pn532_uart:/dev/ttyUSB0` with the owned DESFire EV3 8K.
The environment-gated test `nfc-desfire --features hardware --test
hardware_keys` passed under both EV1 AES and EV2:

```sh
NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfc-desfire --features hardware --test hardware_keys -- --ignored

NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_HARDWARE_WRITE=yes \
NFC_FOUNDRY_HARDWARE_KEY_TYPE=ev2 \
NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
cargo test -p nfc-desfire --features hardware --test hardware_keys -- --ignored
```

The test created AES application `F0 0D 0B` (two AES keys) and DES application
`F0 0D 0C` (two DES keys). For each application it:

- changed a different key while authenticated with key 0 (case 1);
- changed the authenticated key itself (case 2);
- re-authenticated with every new key and read its version back;
- toggled the free file-list bit of the KeySettings and restored it;
- deleted the application afterwards.

The PICC master key was not changed. The final inventory reported
`applications: none` and 7520 free bytes.

The CLI surface was exercised on the same card with a temporary AES
application `F0 0D 0D`:

```text
application F00D0D · settings 0F · ... · change rights: key 0 authenticates key changes · 2 keys · AES
key versions: 0=0 · 1=0

application F00D0D · key 1 · AES · key version 0 -> 4
re-authenticated with the new key · version read back 4
new key exported: /tmp/desfire-key-1.json (mode 0600)

key versions: 0=0 · 1=4
```

The exported file was mode `-rw-------` and contained the schema, application,
key number, family, version, and the key. `app-delete` removed the application
and wrote its manifest.

The test card was formatted once during this work because prior tombstone
applications left 32 free bytes. This is the documented behavior for the
reserved test card.

## ACR122U record

Not recorded. The card was in the PN532 field when the ACR122U was checked;
the same hardware test can be rerun with `NFC_FOUNDRY_DEVICE=acr122_usb:001:002`
as soon as a card is placed on that reader.

## Blockers

**Multiple key sets and key rolling.** AN12752 states that EV2/EV3 applications
support up to 16 key sets and that keyset rolling exists, and lists
InitializeKeySet (`0x56`), FinalizeKeySet (`0x57`), and RollKeySet (`0x55`).
The command parameters, the key set settings encoding, and the EV3
CreateApplication KeySett3 layout are not in any public document available
here. The MIFARE DESFire Light datasheet and AN12343, which cover EV2 secure
messaging and ChangeKey, do not include the key set commands. The full EV2/EV3
functional specification needs an NXP permission grant. No key set command is
sent, and no active key set version is claimed.

**Cross-family PICC master key change.** Changing the PICC master key from DES
to 3K3DES/AES requires a mixed frame whose rules are not covered by the public
ChangeKey examples. The service refuses it and keeps same-family changes. A
dedicated test card would be needed to validate the transition.

## Checks

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
reuse lint
```

The GTK unit test `app::tests::desfire_workspace_switches_and_parses_inputs`
covers the `Keys` tab, its buttons, the Advanced-mode gating, and the empty-AID
PICC parse. The service integration suite `nfc-app/tests/desfire_keys.rs`
covers gates, the export file and its permissions, version read-back, the DES
path, and the PICC path on the scripted card.
