.PHONY: fmt test vet i18n-check ui-test reuse check dev gui-build cli-build build package release-manifest external-engine-fixture toolchain-check libnfc-ng-check libnfc-ng-build libnfc-ng-verify libnfc-ng-smoke libnfc-ng-repeatability-check libnfc-ng-hardware-smoke libnfc-binding-test libnfc-binding-hardware-smoke libnfc-classic-hardware-test libnfc-dump-hardware-test libnfc-dump-restore-hardware-test libnfc-gen1a-hardware-test libnfc-type2-hardware-test libnfc-type2-protected-hardware-test libnfc-classic-ndef-hardware-test libnfc-apdu-hardware-test mfoc-check mfoc-build mfoc-verify mfcuk-check mfcuk-build mfcuk-verify hardnested-check hardnested-build hardnested-verify

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

cli-build:
	@test -f "$(LIBNFC_SDK_DIR)/lib/pkgconfig/libnfc-ng.pc" || (echo "libnfc-ng SDK is missing; run make libnfc-ng-build first" >&2; exit 2)
	@test -d "$(LIBNFC_RUNTIME_DIR)" || (echo "libnfc-ng runtime is missing; run make libnfc-ng-build first" >&2; exit 2)
	mkdir -p build/bin
	env $(NATIVE_ENV) $(GO) build -trimpath -tags "$(NATIVE_TAGS)" -ldflags "$(RELEASE_LDFLAGS)" -o build/bin/nfctl ./cmd/nfctl

build: gui-build cli-build
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

libnfc-gen1a-hardware-test:
	@test -n "$(LIBNFC_DEVICE)" || (echo "LIBNFC_DEVICE is required, for example acr122_usb:001:005" >&2; exit 2)
	@case "$(LIBNFC_DEVICE)" in pn532_uart:*|pn53x_usb:*|acr122_usb:*) ;; *) echo "use a raw-capable PN53x connstring: pn532_uart, pn53x_usb, or acr122_usb" >&2; exit 2 ;; esac
	@test "$(NFCX_HARDWARE_GEN1A_CONFIRM)" = "I_OWN_THIS_CARD" || (echo "set NFCX_HARDWARE_GEN1A_CONFIRM=I_OWN_THIS_CARD to authorize the Gen1A UID write test" >&2; exit 2)
	@test -n "$(NFCX_TEST_UID)" || (echo "NFCX_TEST_UID is required; use 8 hexadecimal digits" >&2; exit 2)
	@test -n "$(NFCX_TEST_UID_BACKUP_DIR)" || (echo "NFCX_TEST_UID_BACKUP_DIR is required so the block 0 backup survives the test" >&2; exit 2)
	@test -f "$(LIBNFC_SDK_DIR)/lib/pkgconfig/libnfc-ng.pc" || (echo "libnfc-ng SDK is missing; run make libnfc-ng-build first" >&2; exit 2)
	env $(NATIVE_ENV) LIBNFC_DEVICE="$(LIBNFC_DEVICE)" LIBNFC_AUTO_SCAN=false LIBNFC_INTRUSIVE_SCAN=false NFCX_WORKFLOW_HARDWARE=1 NFCX_HARDWARE_GEN1A_CONFIRM="$(NFCX_HARDWARE_GEN1A_CONFIRM)" NFCX_TEST_UID="$(NFCX_TEST_UID)" NFCX_TEST_UID_BACKUP_DIR="$(NFCX_TEST_UID_BACKUP_DIR)" NFCX_TEST_KEY_A="$(NFCX_TEST_KEY_A)" $(GO) test -v -count=1 -timeout=10m -run '^TestHardwareGen1AUIDWrite$$' -tags "$(NATIVE_TAGS) libnfc_hardware" ./internal/workflow

libnfc-type2-hardware-test:
	@test -n "$(LIBNFC_DEVICE)" || (echo "LIBNFC_DEVICE is required, for example acr122_usb:001:005" >&2; exit 2)
	@case "$(LIBNFC_DEVICE)" in pn532_uart:*|pn53x_usb:*|acr122_usb:*) ;; *) echo "use a raw-capable PN53x connstring: pn532_uart, pn53x_usb, or acr122_usb" >&2; exit 2 ;; esac
	@test -f "$(LIBNFC_SDK_DIR)/lib/pkgconfig/libnfc-ng.pc" || (echo "libnfc-ng SDK is missing; run make libnfc-ng-build first" >&2; exit 2)
	env $(NATIVE_ENV) LIBNFC_DEVICE="$(LIBNFC_DEVICE)" LIBNFC_AUTO_SCAN=false LIBNFC_INTRUSIVE_SCAN=false NFCX_WORKFLOW_HARDWARE=1 NFCX_HARDWARE_TYPE2_CONFIRM="$(NFCX_HARDWARE_TYPE2_CONFIRM)" NFCX_TEST_TYPE2_BACKUP_DIR="$(NFCX_TEST_TYPE2_BACKUP_DIR)" NFCX_TEST_TYPE2_PWD="$(NFCX_TEST_TYPE2_PWD)" $(GO) test -v -count=1 -timeout=10m -run '^TestHardwareType2ReadWrite$$' -tags "$(NATIVE_TAGS) libnfc_hardware" ./internal/workflow

libnfc-apdu-hardware-test:
	@test -n "$(LIBNFC_DEVICE)" || (echo "LIBNFC_DEVICE is required, for example acr122_usb:001:004" >&2; exit 2)
	@case "$(LIBNFC_DEVICE)" in pn532_uart:*|acr122_usb:*) ;; *) echo "use a validated connstring: pn532_uart or acr122_usb" >&2; exit 2 ;; esac
	@test -f "$(LIBNFC_SDK_DIR)/lib/pkgconfig/libnfc-ng.pc" || (echo "libnfc-ng SDK is missing; run make libnfc-ng-build first" >&2; exit 2)
	env $(NATIVE_ENV) LIBNFC_DEVICE="$(LIBNFC_DEVICE)" LIBNFC_AUTO_SCAN=false LIBNFC_INTRUSIVE_SCAN=false NFCX_WORKFLOW_HARDWARE=1 NFCX_HARDWARE_DESFIRE_CONFIRM="$(NFCX_HARDWARE_DESFIRE_CONFIRM)" NFCX_HARDWARE_TYPE4_CONFIRM="$(NFCX_HARDWARE_TYPE4_CONFIRM)" NFCX_HARDWARE_TYPE4_WRITE="$(NFCX_HARDWARE_TYPE4_WRITE)" NFCX_TEST_TYPE4_BACKUP_DIR="$(NFCX_TEST_TYPE4_BACKUP_DIR)" $(GO) test -v -count=1 -timeout=10m -run '^TestHardware(ISODEP|DESFireRead|Type4NDEF)$$' -tags "$(NATIVE_TAGS) libnfc_hardware" ./internal/workflow

libnfc-classic-ndef-hardware-test:
	@test -n "$(LIBNFC_DEVICE)" || (echo "LIBNFC_DEVICE is required, for example acr122_usb:001:004" >&2; exit 2)
	@case "$(LIBNFC_DEVICE)" in pn532_uart:*|acr122_usb:*) ;; *) echo "use a validated Classic connstring: pn532_uart or acr122_usb" >&2; exit 2 ;; esac
	@test "$(NFCX_HARDWARE_CLASSIC_NDEF_CONFIRM)" = "I_OWN_THIS_CARD" || (echo "set NFCX_HARDWARE_CLASSIC_NDEF_CONFIRM=I_OWN_THIS_CARD to write the test card" >&2; exit 2)
	@test -n "$(NFCX_TEST_CLASSIC_NDEF_BACKUP_DIR)" || (echo "NFCX_TEST_CLASSIC_NDEF_BACKUP_DIR is required so the backup survives the test" >&2; exit 2)
	@test -f "$(LIBNFC_SDK_DIR)/lib/pkgconfig/libnfc-ng.pc" || (echo "libnfc-ng SDK is missing; run make libnfc-ng-build first" >&2; exit 2)
	env $(NATIVE_ENV) LIBNFC_DEVICE="$(LIBNFC_DEVICE)" LIBNFC_AUTO_SCAN=false LIBNFC_INTRUSIVE_SCAN=false NFCX_WORKFLOW_HARDWARE=1 NFCX_HARDWARE_CLASSIC_NDEF_CONFIRM="$(NFCX_HARDWARE_CLASSIC_NDEF_CONFIRM)" NFCX_HARDWARE_CLASSIC_NDEF_FORMAT="$(NFCX_HARDWARE_CLASSIC_NDEF_FORMAT)" NFCX_HARDWARE_CLASSIC_NDEF_SECTORS="$(NFCX_HARDWARE_CLASSIC_NDEF_SECTORS)" NFCX_TEST_CLASSIC_NDEF_BACKUP_DIR="$(NFCX_TEST_CLASSIC_NDEF_BACKUP_DIR)" NFCX_TEST_KEY_A="$(NFCX_TEST_KEY_A)" NFCX_TEST_KEY_B="$(NFCX_TEST_KEY_B)" $(GO) test -v -count=1 -timeout=10m -run '^TestHardwareClassicNDEF$$' -tags "$(NATIVE_TAGS) libnfc_hardware" ./internal/workflow

libnfc-type2-protected-hardware-test:
	@test -n "$(LIBNFC_DEVICE)" || (echo "LIBNFC_DEVICE is required, for example acr122_usb:001:005" >&2; exit 2)
	@case "$(LIBNFC_DEVICE)" in pn532_uart:*|pn53x_usb:*|acr122_usb:*) ;; *) echo "use a raw-capable PN53x connstring: pn532_uart, pn53x_usb, or acr122_usb" >&2; exit 2 ;; esac
	@test -f "$(LIBNFC_SDK_DIR)/lib/pkgconfig/libnfc-ng.pc" || (echo "libnfc-ng SDK is missing; run make libnfc-ng-build first" >&2; exit 2)
	@test "$(NFCX_HARDWARE_TYPE2_PROTECTED_CONFIRM)" = "I_OWN_THIS_DISPOSABLE_TAG" || (echo "set NFCX_HARDWARE_TYPE2_PROTECTED_CONFIRM=I_OWN_THIS_DISPOSABLE_TAG for a disposable tag" >&2; exit 2)
	@test -n "$(NFCX_TEST_TYPE2_BACKUP_DIR)" || (echo "NFCX_TEST_TYPE2_BACKUP_DIR is required so the protected backup survives the test" >&2; exit 2)
	env $(NATIVE_ENV) LIBNFC_DEVICE="$(LIBNFC_DEVICE)" LIBNFC_AUTO_SCAN=false LIBNFC_INTRUSIVE_SCAN=false NFCX_WORKFLOW_HARDWARE=1 NFCX_HARDWARE_TYPE2_PROTECTED_CONFIRM="$(NFCX_HARDWARE_TYPE2_PROTECTED_CONFIRM)" NFCX_TEST_TYPE2_BACKUP_DIR="$(NFCX_TEST_TYPE2_BACKUP_DIR)" NFCX_TEST_TYPE2_PWD="$(NFCX_TEST_TYPE2_PWD)" NFCX_HARDWARE_TYPE2_PROTECT_AUTH0="$(NFCX_HARDWARE_TYPE2_PROTECT_AUTH0)" NFCX_HARDWARE_TYPE2_PROTECT_ACCESS="$(NFCX_HARDWARE_TYPE2_PROTECT_ACCESS)" NFCX_HARDWARE_TYPE2_PROTECT_STATIC_LOCK="$(NFCX_HARDWARE_TYPE2_PROTECT_STATIC_LOCK)" NFCX_HARDWARE_TYPE2_PROTECT_DYNAMIC_LOCK="$(NFCX_HARDWARE_TYPE2_PROTECT_DYNAMIC_LOCK)" $(GO) test -v -count=1 -timeout=10m -run '^TestHardwareType2ProtectedWrite$$' -tags "$(NATIVE_TAGS) libnfc_hardware" ./internal/workflow
