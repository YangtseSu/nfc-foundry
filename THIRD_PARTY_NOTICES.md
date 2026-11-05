# Third-party notices

NFC Foundry is distributed under **GPL-3.0-or-later** (see `LICENSE`). The
third-party components below keep their own licenses. Release packages must
include their notices, source locations, pinned versions, and NFC Foundry
patches.

| Component | Use | License | Distribution |
| --- | --- | --- | --- |
| [libnfc-ng](https://github.com/nfc-tools/libnfc) | Forked native transport library, dynamically linked | LGPL-3.0-or-later | Separate `libnfc-ng` Arch package: shared library, headers, pkg-config, license, source metadata, patches |
| [mfoc](https://github.com/nfc-tools/mfoc) | Nested key recovery, separate process | GPL-2.0-or-later | Bundled executable in `nfc-foundry`; source, license, patches |
| [mfcuk](https://github.com/nfc-tools/mfcuk) | Darkside key recovery, separate process | GPL-2.0-or-later | Bundled executable in `nfc-foundry`; source, license, patches |
| [mfoc-hardnested](https://github.com/nfc-tools/mfoc-hardnested) | Hardnested key recovery, separate process | GPL-2.0-or-later | Bundled executable in `nfc-foundry`; source, license, checksums |
| `nfc-mfsetuid` | Gen1A sequence reference; rewritten in Go | BSD-2-Clause | Port retains the copyright/license notice; no upstream executable is shipped |
| NFCX application code | Behavioral reference and ported Go logic | MIT | See `NOTICE` and `LICENSES/MIT.txt`; notices retained |
| [MyGo](https://github.com/egoist/mygo) | Native Go desktop UI framework | MIT | Built into the application; license text included |

Go module dependencies keep their own licenses; release packaging collects
their notices. Wails, the HTML/TypeScript frontend, AppImage tooling, and the
website/telemetry stack from NFCX are not part of the target product.

## Source and redistribution

The external engines are separate programs, not linked into NFC Foundry. They
are launched as child processes and their output is validated by NFC Foundry.

A binary release must be accompanied by the complete corresponding source for
the GPL-covered work, including build/install scripts and any patches. The
recommended release layout is:

```text
nfc-foundry-<version>-x86_64.tar.zst
nfc-foundry-<version>-source.tar.zst
```

The source archive must include this repository, the `libnfc-ng` fork at the
pinned commit, and the patched engine sources. A stable download URL in the
release notes is also acceptable when it points at the exact corresponding
source.

`libnfc-ng` remains LGPL-3.0-or-later even though it is distributed alongside a
GPL-3.0-or-later application. Keep it dynamically linked and provide its source
and relinkable build metadata.
