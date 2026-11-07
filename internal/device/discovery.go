package device

import (
	"strings"

	"github.com/YangtseSu/nfc-foundry/internal/nfc"
)

// CollapseReaderDuplicates removes PC/SC aliases of an ACR122 reader that is
// already reachable through direct USB or the ACR122 PC/SC driver. The physical
// reader is the same device, and exposing every alias makes automatic reader
// selection ambiguous and invites two processes to claim one reader.
//
// All ACR122 PC/SC aliases are suppressed when any direct-USB ACR122 entry
// exists because the two naming schemes cannot be matched one-to-one. Non-ACR122
// PC/SC readers are preserved because they have no direct-USB alias in the
// enumeration result.
func CollapseReaderDuplicates(devices []nfc.DeviceInfo) []nfc.DeviceInfo {
	directACR122 := false
	acr122PCSC := false
	for _, device := range devices {
		switch nfc.DriverOf(device.ConnString) {
		case nfc.DriverACR122USB:
			directACR122 = true
		case nfc.DriverACR122PCSC:
			acr122PCSC = true
		}
	}
	if !directACR122 && !acr122PCSC {
		return devices
	}
	result := make([]nfc.DeviceInfo, 0, len(devices))
	for _, device := range devices {
		switch nfc.DriverOf(device.ConnString) {
		case nfc.DriverACR122PCSC:
			if directACR122 {
				continue
			}
		case nfc.DriverPCSC:
			if isACR122PCSCReader(device.ConnString) && (directACR122 || acr122PCSC) {
				continue
			}
		}
		result = append(result, device)
	}
	return result
}

func isACR122PCSCReader(connString string) bool {
	_, name, found := strings.Cut(connString, ":")
	if !found {
		return false
	}
	name = strings.ToLower(name)
	return strings.Contains(name, "acr122") || strings.Contains(name, "touchatag")
}
