# Third-party notices

NFC Foundry is GPL-3.0-or-later. The Rust dependency graph includes components
under other licenses. This file lists direct dependencies. A release ships the required license
texts and notices for the shipped work; it does not duplicate dependency
license files or generate a dependency manifest.

| Dependency | Version | License |
|---|---:|---|
| `nusb` | 0.2.x | Apache-2.0 OR MIT |
| `pcsc` | 2.9.x | MIT |
| `pcsc-sys` | 1.3.x | MIT |
| `serialport` | 4.10.x | MPL-2.0 |
| `clap` | 4.6.x | MIT OR Apache-2.0 |
| `thiserror` | 2.0.x | MIT OR Apache-2.0 |
| `serde` | 1.x | MIT OR Apache-2.0 |
| `serde_json` | 1.x | MIT OR Apache-2.0 |
| `proptest` | 1.12.x | MIT OR Apache-2.0 |
| `tempfile` | 3.27.x | MIT OR Apache-2.0 |
| `lz4_flex` | 0.14.x | MIT |

Transitive dependencies and their exact versions are recorded in
`Cargo.lock`. The release installs only NFC Foundry's own license texts and
notices. It does not duplicate dependency license files and does not generate a
dependency manifest; full texts remain with their upstream crates.

Proxmark3-derived recovery code retains its GPL-3.0-or-later notices:

- `crates/nfc-recover/src/crypto1.rs` derives from
  `common/crapto1/crapto1.c` and `crypto1.c` (bla, Proxmark3 contributors).
- `crates/nfc-recover/src/mfkey.rs` derives from
  `client/src/mifare/mfkey.c` (Roel Verdult, Proxmark3 contributors).
- `crates/nfc-recover/src/acquire.rs`, `classify.rs`, `darkside.rs`,
  `nested.rs`, and `attack.rs` implement behavior from
  `client/src/mifare/mifarehost.c` and `armsrc/mifarecmd.c`.
- `crates/nfc-recover/src/hardnested/` derives from
  `client/src/cmdhfmfhard.c`, `client/deps/hardnested/` (piwi, Proxmark3
  contributors), and the state tables in `client/resources/hardnested_tables/`
  (piwi). The bitsliced kernel builds on aczid's MIT-licensed `crypto1_bs`
  (Copyright (c) 2015-2016 Aram Verstegen); the MIT notice is reproduced below.

The hardnested state tables are a direct copy of PM3's
`client/resources/hardnested_tables/` and are distributed under
GPL-3.0-or-later. They are required attack data, not generated at install
time.

```text
Bit-sliced Crypto-1 brute-forcing implementation
Copyright (c) 2015-2016 Aram Verstegen

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
```

The old mfoc/mfcuk/mfoc-hardnested implementations are not shipped; they may
appear only in development notes as test oracles.

The Gen1A unlock and manufacturer-block write sequence in
`crates/nfc-drivers/src/acr122_usb/protocol.rs` is derived from the libnfc
`nfc-mfsetuid` example (Adam Laurie, 2011; Dario Carluccio, 2014). It is
available under BSD-2-Clause OR GPL-3.0-or-later. This project distributes
the combined work under GPL-3.0-or-later; the source tree keeps
`LICENSES/BSD-2-Clause.txt` for the alternative grant, but the installed
package ships only the GPL text.
