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

Transitive dependencies and their exact versions are recorded in
`Cargo.lock`. Before a release, generate and review the full notice bundle with
the selected dependency-license tooling.

Proxmark3-derived recovery code and tables retain their GPL-3.0-or-later
notices. The old mfoc/mfcuk/mfoc-hardnested implementations are not shipped;
they may appear only in development notes as test oracles.
