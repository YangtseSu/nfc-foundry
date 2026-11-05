.PHONY: fmt test vet i18n-check ui-test reuse check dev gui-build build package release-manifest external-engine-fixture toolchain-check libnfc-ng-check libnfc-ng-build libnfc-ng-verify libnfc-ng-smoke libnfc-ng-repeatability-check libnfc-ng-hardware-smoke libnfc-binding-test libnfc-binding-hardware-smoke libnfc-classic-hardware-test libnfc-dump-hardware-test libnfc-dump-restore-hardware-test mfoc-check mfoc-build mfoc-verify mfcuk-check mfcuk-build mfcuk-verify hardnested-check hardnested-build hardnested-verify

GO ?= go
LIBNFC_PLATFORM ?= linux-amd64
LIBNFC_SDK_DIR ?= $(CURDIR)/build/toolchain/$(LIBNFC_PLATFORM)/sdk
LIBNFC_RUNTIME_DIR ?= $(CURDIR)/runtime/$(LIBNFC_PLATFORM)
VERSION ?= 0.1.0
COMMIT ?= $(shell git rev-parse --verify HEAD 2>/dev/null || echo unknown)
BUILD_DATE ?= unknown
DIRTY ?= $(shell test -z "$$(git status --porcelain 2>/dev/null)" && echo false || echo true)
RELEASE_LDFLAGS = -X github.com/YangtseSu/nfc-foundry/internal/buildinfo.Version=$(VERSION) -X github.com/YangtseSu/nfc-foundry/internal/buildinfo.Commit=$(COMMIT) -X github.com/YangtseSu/nfc-foundry/internal/buildinfo.BuildDate=$(BUILD_DATE) -X github.com/YangtseSu/nfc-foundry/internal/buildinfo.Dirty=$(DIRTY)
NATIVE_TAGS := libnfc
NATIVE_ENV = CGO_ENABLED=1 PKG_CONFIG_PATH="$(LIBNFC_SDK_DIR)/lib/pkgconfig" LD_LIBRARY_PATH="$(LIBNFC_RUNTIME_DIR)"

fmt:
	gofmt -w app cmd internal locales main.go

test:
	go test ./...

vet:
	go vet ./...

i18n-check:
	$(GO) tool mygo i18n check .

ui-test:
	go test -count=1 ./internal/desktop

reuse:
	reuse lint

check: test vet i18n-check ui-test reuse

dev:
	@test -f "$(LIBNFC_SDK_DIR)/lib/pkgconfig/libnfc-ng.pc" || (echo "libnfc-ng SDK is missing; run make libnfc-ng-build first" >&2; exit 2)
	env $(NATIVE_ENV) GOFLAGS="-tags=$(NATIVE_TAGS)" $(GO) tool mygo dev .

gui-build:
	@test -f "$(LIBNFC_SDK_DIR)/lib/pkgconfig/libnfc-ng.pc" || (echo "libnfc-ng SDK is missing; run make libnfc-ng-build first" >&2; exit 2)
	@test -d "$(LIBNFC_RUNTIME_DIR)" || (echo "libnfc-ng runtime is missing; run make libnfc-ng-build first" >&2; exit 2)
	mkdir -p build/bin
	env $(NATIVE_ENV) $(GO) build -trimpath -tags "$(NATIVE_TAGS)" -ldflags "$(RELEASE_LDFLAGS)" -o build/bin/nfc-foundry .

build: gui-build
	./scripts/package-runtime.sh build/bin/nfc-foundry

package: build mfoc-build mfcuk-build hardnested-build
	./scripts/release/package-linux.sh "$(VERSION)"

release-manifest:
	./scripts/release/generate-manifest.sh "$(VERSION)" "$(COMMIT)"

# Development/acceptance fixture only. Release packaging must not include it.
external-engine-fixture:
	mkdir -p "$(LIBNFC_RUNTIME_DIR)"
	go build -o "$(LIBNFC_RUNTIME_DIR)/nfcx-fake-engine" ./cmd/nfcx-fake-engine

toolchain-check: libnfc-ng-check mfoc-check mfcuk-check hardnested-check

libnfc-ng-check:
	./scripts/toolchain/libnfc.sh check

libnfc-ng-build:
	./scripts/toolchain/libnfc.sh build

libnfc-ng-verify:
	./scripts/toolchain/libnfc.sh verify

libnfc-ng-smoke:
	./scripts/toolchain/libnfc.sh smoke

libnfc-ng-repeatability-check:
	./scripts/toolchain/libnfc.sh repeatability-check

libnfc-ng-hardware-smoke:
	@test -n "$(LIBNFC_DEVICE)" || (echo "LIBNFC_DEVICE is required, for example pn532_uart:/dev/ttyUSB0" >&2; exit 2)
	./scripts/toolchain/libnfc.sh hardware-smoke "$(LIBNFC_DEVICE)"

mfoc-check:
	./scripts/toolchain/mfoc.sh check

mfoc-build:
	./scripts/toolchain/mfoc.sh build

mfoc-verify:
	./scripts/toolchain/mfoc.sh verify

mfcuk-check:
	./scripts/toolchain/mfcuk.sh check

mfcuk-build:
	./scripts/toolchain/mfcuk.sh build

mfcuk-verify:
	./scripts/toolchain/mfcuk.sh verify

hardnested-check:
	./scripts/toolchain/hardnested.sh check

hardnested-build:
	./scripts/toolchain/hardnested.sh build

hardnested-verify:
	./scripts/toolchain/hardnested.sh verify

libnfc-binding-test:
	@test -f "$(LIBNFC_SDK_DIR)/lib/pkgconfig/libnfc-ng.pc" || (echo "libnfc-ng SDK is missing; run make libnfc-ng-build first" >&2; exit 2)
	@test -d "$(LIBNFC_RUNTIME_DIR)" || (echo "libnfc-ng runtime is missing; run make libnfc-ng-build first" >&2; exit 2)
	env $(NATIVE_ENV) $(GO) test -tags "$(NATIVE_TAGS)" ./...

libnfc-binding-hardware-smoke:
	@test -n "$(LIBNFC_DEVICE)" || (echo "LIBNFC_DEVICE is required, for example pn532_uart:/dev/ttyUSB0" >&2; exit 2)
	@case "$(LIBNFC_DEVICE)" in pn532_uart:*) ;; *) echo "only a pn532_uart connstring is accepted in this stage" >&2; exit 2 ;; esac
	@test -f "$(LIBNFC_SDK_DIR)/lib/pkgconfig/libnfc-ng.pc" || (echo "libnfc-ng SDK is missing; run make libnfc-ng-build first" >&2; exit 2)
	env $(NATIVE_ENV) LIBNFC_DEVICE="$(LIBNFC_DEVICE)" LIBNFC_AUTO_SCAN=false LIBNFC_INTRUSIVE_SCAN=false $(GO) test -count=1 -timeout=10m -tags "$(NATIVE_TAGS) libnfc_hardware" ./internal/nfc/libnfc

libnfc-classic-hardware-test:
	@test -n "$(LIBNFC_DEVICE)" || (echo "LIBNFC_DEVICE is required, for example pn532_uart:/dev/ttyUSB0" >&2; exit 2)
	@case "$(LIBNFC_DEVICE)" in pn532_uart:*) ;; *) echo "only a pn532_uart connstring is accepted in this stage" >&2; exit 2 ;; esac
	@test -f "$(LIBNFC_SDK_DIR)/lib/pkgconfig/libnfc-ng.pc" || (echo "libnfc-ng SDK is missing; run make libnfc-ng-build first" >&2; exit 2)
	env $(NATIVE_ENV) LIBNFC_DEVICE="$(LIBNFC_DEVICE)" LIBNFC_AUTO_SCAN=false LIBNFC_INTRUSIVE_SCAN=false $(GO) test -count=1 -timeout=2m -run '^TestHardwareMIFAREClassicKnownKeyIO$$' -tags "$(NATIVE_TAGS) libnfc_hardware" ./internal/nfc/libnfc

libnfc-dump-hardware-test:
	@test -n "$(LIBNFC_DEVICE)" || (echo "LIBNFC_DEVICE is required, for example pn532_uart:/dev/ttyUSB0" >&2; exit 2)
	@case "$(LIBNFC_DEVICE)" in pn532_uart:*) ;; *) echo "only a pn532_uart connstring is accepted in this stage" >&2; exit 2 ;; esac
	@test -n "$(NFCX_TEST_DUMP_PATH)" || (echo "NFCX_TEST_DUMP_PATH is required so the dump is preserved" >&2; exit 2)
	@test -f "$(LIBNFC_SDK_DIR)/lib/pkgconfig/libnfc-ng.pc" || (echo "libnfc-ng SDK is missing; run make libnfc-ng-build first" >&2; exit 2)
	env $(NATIVE_ENV) LIBNFC_DEVICE="$(LIBNFC_DEVICE)" LIBNFC_AUTO_SCAN=false LIBNFC_INTRUSIVE_SCAN=false NFCX_WORKFLOW_HARDWARE=1 NFCX_TEST_DUMP_PATH="$(NFCX_TEST_DUMP_PATH)" NFCX_TEST_KEY_A="$(NFCX_TEST_KEY_A)" NFCX_TEST_KEY_B="$(NFCX_TEST_KEY_B)" $(GO) test -v -count=1 -timeout=10m -run '^TestHardwareMIFAREClassicDumpRestore$$' -tags "$(NATIVE_TAGS) libnfc_hardware" ./internal/workflow

libnfc-dump-restore-hardware-test:
	@test -n "$(LIBNFC_DEVICE)" || (echo "LIBNFC_DEVICE is required, for example pn532_uart:/dev/ttyUSB0" >&2; exit 2)
	@case "$(LIBNFC_DEVICE)" in pn532_uart:*) ;; *) echo "only a pn532_uart connstring is accepted in this stage" >&2; exit 2 ;; esac
	@test "$(NFCX_HARDWARE_RESTORE_CONFIRM)" = "I_OWN_THIS_CARD" || (echo "set NFCX_HARDWARE_RESTORE_CONFIRM=I_OWN_THIS_CARD to authorize the full-card restore test" >&2; exit 2)
	@test -n "$(NFCX_TEST_DUMP_PATH)" || (echo "NFCX_TEST_DUMP_PATH is required so the recovery dump is preserved" >&2; exit 2)
	@test -f "$(LIBNFC_SDK_DIR)/lib/pkgconfig/libnfc-ng.pc" || (echo "libnfc-ng SDK is missing; run make libnfc-ng-build first" >&2; exit 2)
	env $(NATIVE_ENV) LIBNFC_DEVICE="$(LIBNFC_DEVICE)" LIBNFC_AUTO_SCAN=false LIBNFC_INTRUSIVE_SCAN=false NFCX_WORKFLOW_HARDWARE=1 NFCX_HARDWARE_RESTORE_CONFIRM="$(NFCX_HARDWARE_RESTORE_CONFIRM)" NFCX_TEST_DUMP_PATH="$(NFCX_TEST_DUMP_PATH)" NFCX_TEST_KEY_A="$(NFCX_TEST_KEY_A)" NFCX_TEST_KEY_B="$(NFCX_TEST_KEY_B)" $(GO) test -count=1 -timeout=10m -run '^TestHardwareMIFAREClassicDumpRestore$$' -tags "$(NATIVE_TAGS) libnfc_hardware" ./internal/workflow
