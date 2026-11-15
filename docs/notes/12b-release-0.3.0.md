# Release 0.3.0

Date: 2026-11-15. Previous release: `v0.2.0`.

## Changes

- China T-Union read-only profile (phase 14a):
  - PPSE and `A000000632010105` selection, purse balance, transaction and
    travel records.
  - `nfctl tunion read` and the GTK `China T-Union` workspace.
  - Validated on PN532 UART, ACR122U direct USB, and ACR122U PC/SC.
- EMV payment-card read-only profile (phase 14b):
  - PPSE application list, highest-priority AID selection, zero-amount GPO,
    AFL records, BCD/ASCII Track 2, masked PAN, expiry, AIP/ATC facts.
  - `nfctl emv read` and the GTK `Bank card` workspace.
  - Validated on a UnionPay debit card over PN532 UART and a Visa credit card
    over ACR122U direct USB. ACR122U PC/SC is pending.
- Documentation: phase 14a/14b specs, ADRs, notes, CLI contract, parity,
  workbench, testing, and hardware matrix.
- No breaking CLI or file-format changes. No card-write, payment, credit,
  top-up, PIN, or resident-ID path was added.

## Verification

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check -p nfc-foundry
reuse lint
env -u WAYLAND_DISPLAY NFC_FOUNDRY_GUI_TESTS=1 xvfb-run -a env GDK_BACKEND=x11 \
  cargo test -p nfc-foundry emv_workspace
```

Hardware:

```sh
NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
  cargo test -p nfctl --features hardware --test hardware_tunion -- --ignored
NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_DEVICE=pn532_uart:/dev/ttyUSB0 \
  cargo test -p nfctl --features hardware --test hardware_emv -- --ignored
NFC_FOUNDRY_HARDWARE=1 NFC_FOUNDRY_DEVICE=acr122_usb:001:002 \
  cargo test -p nfctl --features hardware --test hardware_emv -- --ignored
```

The T-Union ACR122U direct USB and PC/SC rows were recorded before this
release; the EMV hardware test was run on both the PN532 UART and ACR122U
direct USB paths. No test touched a card write or payment command.

## Packaging

- `Cargo.toml` and `Cargo.lock` are at `0.3.0`.
- `packaging/aur/PKGBUILD` and `.SRCINFO` build `v0.3.0`.
- The GitHub release workflow builds
  `nfc-foundry-0.3.0-x86_64.tar.gz` and `SHA256SUMS` from the `v0.3.0` tag.
- AUR publication is a separate maintainer action; this note records the
  source package update only.

## Known limitations

- EMV reads only the highest-priority PPSE application; a card with multiple
  payment AIDs exposes the selected one, not every AID's data.
- Type B EMV, Type 4B, Type 3 / FeliCa, ISO 15693 / Type 5, FMCOS, Calypso,
  CIPURSE, and ITSO remain unsupported.
- Chinese resident ID remains blocked on an authorized SAM and written
  authorization.
- The EMV GPO may advance the card ATC and include a cryptogram; the
  cryptogram is discarded and never sent to an issuer.
