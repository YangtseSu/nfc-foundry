#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(CDPATH= cd -- "$SCRIPT_DIR/../.." && pwd)"
VERSION="${1:?version is required}"
DIST="$REPO_ROOT/dist"
APP_ROOT="$REPO_ROOT/build/nfc-foundry-linux-amd64"
ARCHIVE="$DIST/nfc-foundry-$VERSION-linux-amd64.tar.gz"

"$REPO_ROOT/scripts/package-runtime.sh" "$REPO_ROOT/build/bin/nfc-foundry" "$APP_ROOT"
mkdir -p "$DIST"

install -m 0644 "$REPO_ROOT/README.md" "$APP_ROOT/README.md"
install -m 0644 "$REPO_ROOT/NOTICE" "$APP_ROOT/NOTICE"
install -m 0644 "$REPO_ROOT/THIRD_PARTY_NOTICES.md" "$APP_ROOT/THIRD_PARTY_NOTICES.md"
mkdir -p "$APP_ROOT/LICENSES"
install -m 0644 "$REPO_ROOT/LICENSE" "$APP_ROOT/LICENSES/GPL-3.0-or-later.txt"
install -m 0644 "$REPO_ROOT/LICENSES/LGPL-3.0-or-later.txt" "$APP_ROOT/LICENSES/LGPL-3.0-or-later.txt"
install -m 0644 "$REPO_ROOT/LICENSES/GPL-2.0-or-later.txt" "$APP_ROOT/LICENSES/GPL-2.0-or-later.txt"
install -m 0644 "$REPO_ROOT/LICENSES/MIT.txt" "$APP_ROOT/LICENSES/MIT.txt"
install -m 0644 "$REPO_ROOT/LICENSES/BSD-2-Clause.txt" "$APP_ROOT/LICENSES/BSD-2-Clause.txt"

tar -C "$REPO_ROOT/build" -czf "$ARCHIVE" nfc-foundry-linux-amd64
echo "created $ARCHIVE"
