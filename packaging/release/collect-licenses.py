#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 NFC Foundry contributors
# SPDX-License-Identifier: GPL-3.0-or-later
"""Collect dependency license files into a release bundle.

The release must carry the license text of every crate it links. Cargo exposes
the resolved package list and each registry package ships its license files
next to its manifest. This script copies those files into one directory per
package and writes a manifest with the declared SPDX expression.

Packages that only declare an SPDX expression (no license file) still appear in
the manifest so the release reader can resolve the text from the expression.
"""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

LICENSE_PATTERNS = (
    "license",
    "licence",
    "copying",
    "notice",
    "unlicense",
    "patents",
    "copyright",
)


def cargo_metadata(root: Path) -> dict:
    output = subprocess.run(
        [
            "cargo",
            "metadata",
            "--locked",
            "--format-version",
            "1",
            "--manifest-path",
            str(root / "Cargo.toml"),
        ],
        check=True,
        capture_output=True,
        text=True,
    )
    return json.loads(output.stdout)


def safe_name(value: str) -> str:
    return re.sub(r"[^A-Za-z0-9._-]+", "_", value)


def license_files(package_dir: Path) -> list[Path]:
    found = []
    for entry in sorted(package_dir.iterdir()):
        if not entry.is_file():
            continue
        name = entry.name.lower()
        if any(name == pattern or name.startswith(f"{pattern}.") for pattern in LICENSE_PATTERNS) or any(
            name.startswith(f"{pattern}-") for pattern in LICENSE_PATTERNS
        ):
            found.append(entry)
    return found


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, default=Path("."))
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()

    root = args.root.resolve()
    metadata = cargo_metadata(root)
    output = args.output
    output.mkdir(parents=True, exist_ok=True)

    rows = []
    copied = 0
    for package in metadata["packages"]:
        manifest = Path(package["manifest_path"]).parent
        if package.get("source") is None:
            continue
        directory = output / safe_name(f"{package['name']}-{package['version']}")
        files = license_files(manifest)
        if files:
            directory.mkdir(parents=True, exist_ok=True)
            for file in files:
                destination = directory / file.name
                shutil.copy2(file, destination)
                destination.chmod(0o644)
                copied += 1
        rows.append(
            (
                package["name"],
                package["version"],
                package.get("license") or "",
                package.get("license_file") or "",
                ", ".join(file.name for file in files),
            )
        )

    rows.sort()
    with (output / "manifest.tsv").open("w", encoding="utf-8") as handle:
        handle.write("package\tversion\tlicense\tlicense_file\tbundled_files\n")
        for row in rows:
            handle.write("\t".join(row) + "\n")

    print(f"license bundle: {len(rows)} packages, {copied} files -> {output}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
