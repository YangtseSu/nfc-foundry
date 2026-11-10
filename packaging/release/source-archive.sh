#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 NFC Foundry contributors
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Build the GPL source archive from a committed ref.
# Usage: source-archive.sh <version> [output-dir] [ref]
set -euo pipefail

version=${1:?usage: source-archive.sh <version> [output-dir] [ref]}
out=${2:-dist}
ref=${3:-HEAD}
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
name="nfc-foundry-$version"

if ! git -C "$root" rev-parse --verify "$ref^{commit}" >/dev/null 2>&1; then
    printf 'unknown git ref: %s\n' "$ref" >&2
    exit 1
fi

if [ -n "$(git -C "$root" status --porcelain --untracked-files=no)" ]; then
    printf 'warning: tracked files have uncommitted changes; the archive is from %s\n' "$ref" >&2
fi

install -d "$out"
git -C "$root" archive --format=tar.gz --prefix="$name/" \
    -o "$out/$name-source.tar.gz" "$ref"
printf 'wrote %s\n' "$out/$name-source.tar.gz"
