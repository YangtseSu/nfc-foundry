# Third-party notices

NFC Foundry is GPL-3.0-or-later. The Rust dependency graph includes components
under other licenses. This file lists direct dependencies; a release must also
include the complete dependency license texts and any required notices.

| Dependency | Version | License |
|---|---:|---|
| `gpui-kit` | 0.9.x | Apache-2.0 |
| `nusb` | 0.2.x | Apache-2.0 OR MIT |
| `serialport` | 4.10.x | MPL-2.0 |
| `clap` | 4.6.x | MIT OR Apache-2.0 |
| `thiserror` | 2.0.x | MIT OR Apache-2.0 |
| `serde` | 1.x | MIT OR Apache-2.0 |
| `serde_json` | 1.x | MIT OR Apache-2.0 |
| `proptest` | 1.12.x | MIT OR Apache-2.0 |
| `tempfile` | 3.27.x | MIT OR Apache-2.0 |

Transitive dependencies and their exact versions are recorded in
`Cargo.lock`. Before a release, generate and review the full notice bundle with
the selected dependency-license tooling.

Proxmark3-derived recovery code retains its GPL-3.0-or-later notices:

- `crates/nfc-recover/src/crypto1.rs` derives from
  `common/crapto1/crapto1.c` and `crypto1.c` (bla, Proxmark3 contributors).
- `crates/nfc-recover/src/mfkey.rs` derives from
  `client/src/mifare/mfkey.c` (Roel Verdult, Proxmark3 contributors).
- `crates/nfc-recover/src/acquire.rs`, `classify.rs`, `darkside.rs`,
  `nested.rs`, and `attack.rs` implement behavior from
  `client/src/mifare/mifarehost.c` and `armsrc/mifarecmd.c`.

The old mfoc/mfcuk/mfoc-hardnested implementations are not shipped; they may
appear only in development notes as test oracles.

The Gen1A unlock and manufacturer-block write sequence in
`crates/nfc-drivers/src/acr122_usb/protocol.rs` is derived from the libnfc
`nfc-mfsetuid` example (Adam Laurie, 2011; Dario Carluccio, 2014). It is
available under BSD-2-Clause OR GPL-3.0-or-later; the BSD text is
`LICENSES/BSD-2-Clause.txt`.
