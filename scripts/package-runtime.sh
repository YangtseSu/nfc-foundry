#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)"
# shellcheck source=toolchain/platform.sh
source "$SCRIPT_DIR/toolchain/platform.sh"

PLATFORM="$(nfcx_platform)"
case "$PLATFORM" in
  linux-*) ;;
  *) echo "runtime packaging supports Linux only" >&2; exit 1 ;;
esac

APP_PATH="${1:-$REPO_ROOT/build/bin/nfc-foundry}"
APP_ROOT="${2:-$REPO_ROOT/build/nfc-foundry-linux-amd64}"
SOURCE="$REPO_ROOT/runtime/$PLATFORM"
DESTINATION="$APP_ROOT/runtime/$PLATFORM"

[[ -x "$APP_PATH" ]] || { echo "application executable is missing: $APP_PATH" >&2; exit 1; }
[[ -d "$SOURCE" ]] || { echo "libnfc-ng runtime is missing; run make libnfc-ng-build first" >&2; exit 1; }

rm -rf "$APP_ROOT"
mkdir -p "$APP_ROOT" "$DESTINATION"
install -m 0755 "$APP_PATH" "$APP_ROOT/nfc-foundry"

for library in "$SOURCE"/libnfcng.so*; do
  [[ -e "$library" || -L "$library" ]] && cp -P "$library" "$DESTINATION/"
done
[[ -e "$DESTINATION/libnfcng.so.6" ]] || { echo "libnfcng.so.6 is missing from the runtime" >&2; exit 1; }

for engine in mfoc mfcuk mfoc-hardnested; do
  if [[ -x "$SOURCE/$engine" ]]; then
    install -m 0755 "$SOURCE/$engine" "$DESTINATION/$engine"
  else
    echo "warning: $engine is not built; the packaged app cannot run that recovery engine" >&2
  fi
done

if [[ -d "$SOURCE/LICENSES" ]]; then
  mkdir -p "$DESTINATION/LICENSES"
  find "$SOURCE/LICENSES" -maxdepth 1 -type f -exec install -m 0644 {} "$DESTINATION/LICENSES/" \;
fi
for metadata in source.lock mfoc-source.lock mfcuk-source.lock mfoc-hardnested-source.lock; do
  [[ -f "$SOURCE/$metadata" ]] && install -m 0644 "$SOURCE/$metadata" "$DESTINATION/$metadata"
done

patchelf --set-rpath '$ORIGIN/runtime/linux-amd64' "$APP_ROOT/nfc-foundry"
echo "packaged unsigned Linux application at $APP_ROOT"
