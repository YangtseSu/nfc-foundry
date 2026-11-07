#!/usr/bin/env bash

set -euo pipefail

export LC_ALL=C
export LANG=C

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(CDPATH= cd -- "$SCRIPT_DIR/../.." && pwd)"
LOCK_FILE="$REPO_ROOT/third_party/libnfc/libnfc.lock"
# shellcheck source=platform.sh
source "$SCRIPT_DIR/platform.sh"
# shellcheck source=/dev/null
source "$LOCK_FILE"

required_lock_values=(
  LIBNFC_TAG
  LIBNFC_COMMIT
  LIBNFC_SOURCE_URL
  LIBNFC_LICENSE
  LIBNFC_DRIVERS
  LIBNFC_PKG_CONFIG
  LIBNFC_LIBRARY
)
for lock_name in "${required_lock_values[@]}"; do
  if [[ -z "${!lock_name:-}" ]]; then
    echo "libnfc-ng toolchain: missing $lock_name in $LOCK_FILE" >&2
    exit 1
  fi
done

PLATFORM="$(nfcx_platform)"
case "$PLATFORM" in
  linux-*) ;;
  *)
    echo "libnfc-ng toolchain: only Linux hosts are supported" >&2
    exit 1
    ;;
esac

OUTPUT_ROOT="${NFCX_TOOLCHAIN_OUTPUT_DIR:-$REPO_ROOT/build/toolchain/$PLATFORM}"
SDK_DIR="$OUTPUT_ROOT/sdk"
BUILD_INFO_DIR="$OUTPUT_ROOT/build-info"
RUNTIME_DIR="${NFCX_RUNTIME_DIR:-$REPO_ROOT/runtime/$PLATFORM}"
CACHE_DIR="${NFCX_TOOLCHAIN_CACHE_DIR:-$REPO_ROOT/.cache/toolchain}"
SOURCE_DIR="${NFCX_LIBNFC_SOURCE_DIR:-$CACHE_DIR/libnfc-ng-$LIBNFC_COMMIT}"
DRIVERS="${NFCX_LIBNFC_DRIVERS:-$LIBNFC_DRIVERS}"
TEMP_BASE="${TMPDIR:-/tmp}"
TEMP_BASE="${TEMP_BASE%/}"
CLEANUP_DIR=""

cleanup() {
  case "$CLEANUP_DIR" in
    "$TEMP_BASE"/nfcx-libnfc-ng-*) rm -rf "$CLEANUP_DIR" ;;
  esac
}
trap cleanup EXIT

usage() {
  cat <<'EOF'
Usage: scripts/toolchain/libnfc.sh <command> [arguments]

Commands:
  check                Report host tools and whether build outputs exist.
  build                Build libnfc-ng from the pinned fork commit.
  verify               Verify identity, driver set, and runtime files.
  smoke                Compile and run the env-only device enumeration smoke.
  repeatability-check  Build and verify twice in independent temporary trees.
  hardware-smoke CONN  Enumerate one explicit raw-capable PN53x connstring.

Optional environment:
  NFCX_LIBNFC_SOURCE_DIR      Use an existing libnfc-ng checkout.
  NFCX_LIBNFC_DRIVERS         Override the driver set for a development build.
  NFCX_TOOLCHAIN_CACHE_DIR    Override the source clone directory.
  NFCX_TOOLCHAIN_OUTPUT_DIR   Override the development SDK/output directory.
  NFCX_RUNTIME_DIR            Override the runtime directory.
  NFCX_BUILD_JOBS             Override parallel make job count.
EOF
}

have_command() {
  command -v "$1" >/dev/null 2>&1
}

check_tools() {
  local missing=0 tool
  local tools=(awk autoconf automake cc git grep install libtoolize make mkdir mktemp pkg-config readelf rm sed tar)
  echo "libnfc-ng toolchain: host=$PLATFORM"
  echo "libnfc-ng toolchain: tag=$LIBNFC_TAG commit=$LIBNFC_COMMIT"
  echo "libnfc-ng toolchain: drivers=$DRIVERS"
  for tool in "${tools[@]}"; do
    if have_command "$tool"; then
      printf '  found   %-20s %s\n' "$tool" "$(command -v "$tool")"
    else
      printf '  missing %-20s\n' "$tool"
      missing=1
    fi
  done
  if [[ -f "$SDK_DIR/include/libnfc-ng/nfc/nfc.h" ]]; then
    echo "libnfc-ng toolchain: development SDK present at $SDK_DIR"
  else
    echo "libnfc-ng toolchain: development SDK not built"
  fi
  if compgen -G "$RUNTIME_DIR/$LIBNFC_LIBRARY*" >/dev/null; then
    echo "libnfc-ng toolchain: runtime present at $RUNTIME_DIR"
  else
    echo "libnfc-ng toolchain: runtime not built"
  fi
  return "$missing"
}

obtain_source() {
  if [[ -n "${NFCX_LIBNFC_SOURCE_DIR:-}" ]]; then
    [[ -d "$SOURCE_DIR" ]] || { echo "libnfc-ng toolchain: NFCX_LIBNFC_SOURCE_DIR does not exist: $SOURCE_DIR" >&2; return 1; }
  elif [[ -d "$SOURCE_DIR/.git" ]]; then
    echo "libnfc-ng toolchain: using cached clone at $SOURCE_DIR" >&2
    git -C "$SOURCE_DIR" fetch --quiet --tags origin || true
    git -C "$SOURCE_DIR" checkout --quiet "$LIBNFC_COMMIT"
  else
    mkdir -p "$CACHE_DIR"
    echo "libnfc-ng toolchain: cloning $LIBNFC_SOURCE_URL" >&2
    git clone --quiet "$LIBNFC_SOURCE_URL" "$SOURCE_DIR"
    git -C "$SOURCE_DIR" checkout --quiet "$LIBNFC_COMMIT"
  fi
  local actual
  actual="$(git -C "$SOURCE_DIR" rev-parse HEAD)"
  if [[ "$actual" != "$LIBNFC_COMMIT" ]]; then
    echo "libnfc-ng toolchain: source commit mismatch: got $actual, want $LIBNFC_COMMIT" >&2
    return 1
  fi
  printf '%s\n' "$SOURCE_DIR"
}

prepare_output_root() {
  if [[ -e "$OUTPUT_ROOT" ]]; then
    case "$OUTPUT_ROOT" in
      "$REPO_ROOT/build/toolchain/"*)
        rm -rf "$OUTPUT_ROOT"
        ;;
      *)
        echo "libnfc-ng toolchain: refusing to replace custom output directory: $OUTPUT_ROOT" >&2
        return 1
        ;;
    esac
  fi
  mkdir -p "$SDK_DIR" "$BUILD_INFO_DIR"
}

make_jobs() {
  if [[ -n "${NFCX_BUILD_JOBS:-}" ]]; then
    case "$NFCX_BUILD_JOBS" in
      *[!0-9]*|0)
        echo "libnfc-ng toolchain: NFCX_BUILD_JOBS must be a positive integer" >&2
        return 1
        ;;
      *) printf '%s\n' "$NFCX_BUILD_JOBS" ;;
    esac
  else
    nfcx_cpu_count
  fi
}

build_libnfc() {
  local source_dir work_dir build_dir jobs runtime_library runtime_name soname
  check_tools >/dev/null
  source_dir="$(obtain_source)"
  prepare_output_root
  work_dir="$(mktemp -d "$TEMP_BASE/nfcx-libnfc-ng-build.XXXXXX")"
  CLEANUP_DIR="$work_dir"
  cp -a "$source_dir" "$work_dir/libnfc-ng"
  build_dir="$work_dir/libnfc-ng"

  printf '%s\n' "$LIBNFC_COMMIT" >"$BUILD_INFO_DIR/source-commit.txt"
  cp "$LOCK_FILE" "$BUILD_INFO_DIR/source.lock"

  (
    cd "$build_dir"
    autoreconf -fi >"$BUILD_INFO_DIR/autoreconf.log" 2>&1
    ./configure \
      "--prefix=$SDK_DIR" \
      "--sysconfdir=$SDK_DIR/etc" \
      "--enable-shared" \
      "--disable-static" \
      "--disable-conffiles" \
      "--enable-envvars" \
      "--with-drivers=$DRIVERS" 2>&1 | tee "$BUILD_INFO_DIR/configure.log"
    jobs="$(make_jobs)"
    make -j"$jobs" 2>&1 | tee "$BUILD_INFO_DIR/build.log"
    make install 2>&1 | tee "$BUILD_INFO_DIR/install.log"
  )
  cp "$build_dir/libnfc/Makefile" "$BUILD_INFO_DIR/libnfc.Makefile"
  cp "$build_dir/config.log" "$BUILD_INFO_DIR/configure.config.log" 2>/dev/null || true

  runtime_library="$(find "$SDK_DIR/lib" -maxdepth 1 -type f -name 'libnfcng.so.*.*.*' -print | head -n 1)"
  [[ -n "$runtime_library" ]] || { echo "libnfc-ng toolchain: installed versioned shared object was not found" >&2; return 1; }
  runtime_name="$(basename "$runtime_library")"
  soname="$(readelf -d "$runtime_library" | sed -n 's/.*SONAME.*\[\(.*\)\].*/\1/p')"
  [[ "$soname" == "$LIBNFC_LIBRARY" ]] || { echo "libnfc-ng toolchain: unexpected SONAME $soname" >&2; return 1; }

  mkdir -p "$RUNTIME_DIR/LICENSES"
  rm -f "$RUNTIME_DIR"/libnfcng.so*
  nfcx_strip_release_binary "$PLATFORM" "$runtime_library"
  install -m 0755 "$runtime_library" "$RUNTIME_DIR/$runtime_name"
  ln -s "$runtime_name" "$RUNTIME_DIR/$soname"
  ln -s "$soname" "$RUNTIME_DIR/libnfcng.so"
  install -m 0644 "$build_dir/COPYING" "$RUNTIME_DIR/LICENSES/libnfc-ng-COPYING.txt"
  install -m 0644 "$LOCK_FILE" "$RUNTIME_DIR/source.lock"

  echo "libnfc-ng toolchain: build complete"
  echo "  development SDK: $SDK_DIR"
  echo "  application runtime: $RUNTIME_DIR"
}

verify_driver_selection() {
  local makefile="$BUILD_INFO_DIR/libnfc.Makefile"
  local driver line
  line="$(sed -n 's/^DRIVERS_CFLAGS = //p' "$makefile" | head -n 1)"
  IFS=',' read -r -a drivers <<<"$DRIVERS"
  for driver in "${drivers[@]}"; do
    [[ "$line" == *"-DDRIVER_${driver^^}_ENABLED"* ]] || {
      echo "libnfc-ng verify: $driver was not enabled" >&2
      return 1
    }
  done
}

verify_libnfc() {
  local runtime_library soname
  local required_files=(
    "$SDK_DIR/include/libnfc-ng/nfc/nfc.h"
    "$SDK_DIR/lib/pkgconfig/$LIBNFC_PKG_CONFIG.pc"
    "$BUILD_INFO_DIR/configure.log"
    "$BUILD_INFO_DIR/libnfc.Makefile"
    "$RUNTIME_DIR/LICENSES/libnfc-ng-COPYING.txt"
    "$RUNTIME_DIR/source.lock"
  )
  for required_file in "${required_files[@]}"; do
    [[ -e "$required_file" ]] || { echo "libnfc-ng verify: missing $required_file" >&2; return 1; }
  done
  verify_driver_selection
  if compgen -G "$SDK_DIR/bin/nfc-*" >/dev/null; then
    echo "libnfc-ng verify: the SDK installed a command-line utility" >&2
    return 1
  fi
  if compgen -G "$RUNTIME_DIR/libnfc.so*" >/dev/null; then
    echo "libnfc-ng verify: the upstream library identity was installed" >&2
    return 1
  fi
  runtime_library="$(find "$RUNTIME_DIR" -maxdepth 1 -type f -name 'libnfcng.so.*.*.*' -print | head -n 1)"
  [[ -n "$runtime_library" ]] || { echo "libnfc-ng verify: runtime shared object is missing" >&2; return 1; }
  file "$runtime_library" | grep -Eq 'ELF 64-bit.*x86-64' || { echo "libnfc-ng verify: shared object is not Linux amd64" >&2; return 1; }
  soname="$(readelf -d "$runtime_library" | sed -n 's/.*SONAME.*\[\(.*\)\].*/\1/p')"
  [[ "$soname" == "$LIBNFC_LIBRARY" ]] || { echo "libnfc-ng verify: unexpected SONAME $soname" >&2; return 1; }
  [[ -L "$RUNTIME_DIR/$LIBNFC_LIBRARY" && -L "$RUNTIME_DIR/libnfcng.so" ]] || { echo "libnfc-ng verify: development symlinks are missing" >&2; return 1; }
  if strings "$runtime_library" | grep -Eq '/home/|/Users/|/private/tmp/|/private/var/folders/'; then
    echo "libnfc-ng verify: runtime library contains a private build path" >&2
    return 1
  fi
  cmp -s "$LOCK_FILE" "$RUNTIME_DIR/source.lock" || { echo "libnfc-ng verify: runtime source lock differs from repository lock" >&2; return 1; }
  if ! PKG_CONFIG_PATH="$SDK_DIR/lib/pkgconfig" pkg-config --libs "$LIBNFC_PKG_CONFIG" | grep -q -- '-lnfcng'; then
    echo "libnfc-ng verify: pkg-config does not link libnfcng" >&2
    return 1
  fi
  echo "libnfc-ng verify: PASS ($PLATFORM, $DRIVERS)"
}

smoke_libnfc() {
  local smoke_dir smoke_binary
  smoke_dir="$OUTPUT_ROOT/smoke"
  smoke_binary="$smoke_dir/libnfc-smoke"
  mkdir -p "$smoke_dir"
  env PKG_CONFIG_PATH="$SDK_DIR/lib/pkgconfig" cc \
    "$SCRIPT_DIR/libnfc-smoke.c" \
    $(PKG_CONFIG_PATH="$SDK_DIR/lib/pkgconfig" pkg-config --cflags --libs "$LIBNFC_PKG_CONFIG") \
    -o "$smoke_binary"
  env -u LIBNFC_DEVICE -u LIBNFC_DEFAULT_DEVICE \
    PATH="$RUNTIME_DIR:$PATH" LD_LIBRARY_PATH="$RUNTIME_DIR" \
    LIBNFC_AUTO_SCAN=false LIBNFC_INTRUSIVE_SCAN=false NFCX_EXPECT_DEVICE_COUNT=0 \
    "$smoke_binary"
  env PATH="$RUNTIME_DIR:$PATH" LD_LIBRARY_PATH="$RUNTIME_DIR" \
    LIBNFC_DEVICE="pn532_uart:/dev/nfcx-smoke-not-opened" \
    LIBNFC_AUTO_SCAN=false LIBNFC_INTRUSIVE_SCAN=false NFCX_EXPECT_DEVICE_COUNT=1 \
    "$smoke_binary"
}

hardware_smoke() {
  local connstring="${1:-${LIBNFC_DEVICE:-}}"
  if [[ -z "$connstring" ]]; then
    echo "libnfc-ng hardware smoke: a connstring is required" >&2
    return 2
  fi
  case "$connstring" in
    pn532_uart:*|pn53x_usb:*|acr122_usb:*|acr122_pcsc:*) ;;
    *)
      echo "libnfc-ng hardware smoke: use a raw-capable PN53x connstring: pn532_uart, pn53x_usb, acr122_usb, or acr122_pcsc" >&2
      return 2
      ;;
  esac
  smoke_libnfc
  echo "libnfc-ng hardware smoke: the smoke program only enumerates devices; connect the GUI for card access"
}

repeatability_check() {
  local test_root run_index
  check_tools >/dev/null
  test_root="$(mktemp -d "$TEMP_BASE/nfcx-libnfc-ng-repeatability.XXXXXX")"
  CLEANUP_DIR="$test_root"
  for run_index in 1 2; do
    echo "libnfc-ng repeatability: clean build $run_index/2"
    NFCX_TOOLCHAIN_OUTPUT_DIR="$test_root/run-$run_index/toolchain/$PLATFORM" \
      NFCX_RUNTIME_DIR="$test_root/run-$run_index/runtime/$PLATFORM" \
      "$0" build
    NFCX_TOOLCHAIN_OUTPUT_DIR="$test_root/run-$run_index/toolchain/$PLATFORM" \
      NFCX_RUNTIME_DIR="$test_root/run-$run_index/runtime/$PLATFORM" \
      "$0" verify
    NFCX_TOOLCHAIN_OUTPUT_DIR="$test_root/run-$run_index/toolchain/$PLATFORM" \
      NFCX_RUNTIME_DIR="$test_root/run-$run_index/runtime/$PLATFORM" \
      "$0" smoke
  done
  echo "libnfc-ng repeatability: PASS (two independent clean builds)"
}

command_name="${1:-}"
case "$command_name" in
  check) check_tools ;;
  build) build_libnfc ;;
  verify) verify_libnfc ;;
  smoke) smoke_libnfc ;;
  hardware-smoke) shift; hardware_smoke "${1:-}" ;;
  repeatability-check) repeatability_check ;;
  -h|--help|help) usage ;;
  *) usage >&2; exit 2 ;;
esac
