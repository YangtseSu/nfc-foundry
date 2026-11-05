#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(CDPATH= cd -- "$SCRIPT_DIR/../.." && pwd)"

VERSION="${1:?version is required}"
COMMIT="${2:?Git commit is required}"
ROOT="$REPO_ROOT/build/nfc-foundry-linux-amd64/runtime/linux-amd64"

[[ -d "$ROOT" ]] || { echo "packaged runtime is missing: $ROOT" >&2; exit 1; }
rm -rf "$ROOT/LICENSES/go"
"$SCRIPT_DIR/collect-go-licenses.sh" "$ROOT/LICENSES/go"
(
  cd "$REPO_ROOT"
  go run ./cmd/nfc-foundry-release manifest \
    -repository "$REPO_ROOT" -root "$ROOT" -output "$ROOT/manifest.json" \
    -platform linux-amd64 -version "$VERSION" -commit "$COMMIT"
  go run ./cmd/nfc-foundry-release verify -root "$ROOT" -manifest "$ROOT/manifest.json"
)
