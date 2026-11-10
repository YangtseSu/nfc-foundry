<!--
SPDX-FileCopyrightText: 2026 NFC Foundry contributors
SPDX-License-Identifier: MIT OR Apache-2.0 OR Zlib OR MPL-2.0
-->

# Standard third-party license texts

The release bundle includes the license files published by each dependency.
Some crates declare an SPDX license expression without publishing a license
file, mostly platform-specific crates that are not linked on Linux. The
standard texts in `LICENSES/` cover those expressions:

- `MIT.txt`
- `Apache-2.0.txt`
- `Zlib.txt`
- `MPL-2.0.txt`

The dependency bundle's `manifest.tsv` records the declared expression and the
files that were copied for every package. A package that ships its own license
file keeps that file; the shared text is only the fallback for a package with
no bundled file.
