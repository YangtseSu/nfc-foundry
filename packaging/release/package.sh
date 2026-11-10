#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 NFC Foundry contributors
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Build the relocatable binary tarball from an already built target/release.
# Usage: package.sh <version> [output-dir]
set -euo pipefail

version=${1:?usage: package.sh <version> [output-dir]}
out=${2:-dist}
root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
build=${NFC_FOUNDRY_BUILD_DIR:-$root/target/release}
name="nfc-foundry-$version-x86_64"
stage="$out/$name"

for binary in nfc-foundry nfctl; do
    if [ ! -x "$build/$binary" ]; then
        printf 'missing release binary: %s\n' "$build/$binary" >&2
        printf 'run: cargo build --release --locked --workspace\n' >&2
        exit 1
    fi
    if ! file "$build/$binary" | grep -q 'x86-64'; then
        printf 'release binary is not Linux amd64: %s\n' "$build/$binary" >&2
        exit 1
    fi
done

rm -rf "$stage"
install -d "$stage/bin" "$stage/lib/udev/rules.d" "$stage/share/nfc-foundry" \
    "$stage/share/licenses/nfc-foundry" "$stage/share/doc/nfc-foundry"

install -m 0755 "$build/nfc-foundry" "$stage/bin/nfc-foundry"
install -m 0755 "$build/nfctl" "$stage/bin/nfctl"
install -m 0644 "$root/resources/udev/70-nfc-foundry.rules" \
    "$stage/lib/udev/rules.d/70-nfc-foundry.rules"
cp -a "$root/resources/hardnested_tables" "$stage/share/nfc-foundry/"

install -m 0644 "$root/LICENSE" "$stage/share/licenses/nfc-foundry/LICENSE"
cp -a "$root/LICENSES" "$stage/share/licenses/nfc-foundry/LICENSES"
install -m 0644 "$root/NOTICE" "$stage/share/licenses/nfc-foundry/NOTICE"
install -m 0644 "$root/THIRD_PARTY_NOTICES.md" \
    "$stage/share/licenses/nfc-foundry/THIRD_PARTY_NOTICES.md"
install -m 0644 "$root/packaging/release/standard-license-texts.md" \
    "$stage/share/licenses/nfc-foundry/STANDARD_LICENSE_TEXTS.md"
python3 "$root/packaging/release/collect-licenses.py" --root "$root" \
    --output "$stage/share/licenses/nfc-foundry/dependencies"

install -m 0644 "$root/README.md" "$stage/README.md"
install -m 0644 "$root/Cargo.lock" "$stage/Cargo.lock"
for document in architecture hardware hardware-matrix testing packaging recovery ndef; do
    install -m 0644 "$root/docs/$document.md" \
        "$stage/share/doc/nfc-foundry/$document.md"
done
install -m 0644 "$root/docs/plan/README.md" "$stage/share/doc/nfc-foundry/plan.md"
cp -a "$root/docs/adr" "$stage/share/doc/nfc-foundry/adr"

epoch=${SOURCE_DATE_EPOCH:-$(git -C "$root" log -1 --format=%ct)}
tar --sort=name --mtime="@$epoch" --owner=0 --group=0 --numeric-owner \
    -cf - -C "$out" "$name" | gzip -n -9 >"$out/$name.tar.gz"
printf 'wrote %s\n' "$out/$name.tar.gz"
